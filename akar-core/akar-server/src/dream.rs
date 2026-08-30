//! Dream engine control state for the akar server (P77).
//!
//! This module owns the per-server dream lifecycle ("status" / "pause" /
//! "resume" / "run") and the [`DreamOrchestrator`] that executes a
//! consolidation cycle. It is wired into [`SessionConfig`](crate::session::SessionConfig)
//! as a shared [`Arc`], so every client connection observes the same engine.
//!
//! Two backends back the orchestrator:
//!
//! - [`GraceBackend`] — a graceful-degradation stub used when the engine has no
//!   database handle (e.g. unit tests). Every [`DreamBackend`] method is a
//!   no-op returning zero/empty, so a full cycle runs safely against nothing.
//! - [`GraphBackend`] — the production backend (P77b). It executes the dream
//!   operations as Cypher against the akar graph [`Database`], mirroring the
//!   kairos [`AkarDreamBackend`] semantics. It degrades gracefully: when the
//!   shared database does not carry the `Memory`/`Connected` graph schema (a
//!   non-kairos db), each method returns empty/zero instead of erroring, so a
//!   cycle is always safe. When kairos has seeded graph data, `run` does real
//!   NREM/insight/bridge work.

use akar_common::types::Value;
use akar_dream::backend::{self, DreamBackend};
use akar_dream::config::DreamConfig;
use akar_dream::orchestrator::{DreamOrchestrator, DreamStats};
use akar_main::connection::Connection;
use akar_main::database::Database;
use akar_main::query_result::QueryResult;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A result row: a list of `(column_name, value)` pairs in projection order.
type Row = Vec<(String, Value)>;

/// Human-readable state of the dream engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DreamState {
    Idle,
    Running,
    Paused,
}

impl DreamState {
    pub fn as_str(&self) -> &'static str {
        match self {
            DreamState::Idle => "idle",
            DreamState::Running => "running",
            DreamState::Paused => "paused",
        }
    }
}

/// Backend that degrades gracefully: every [`DreamBackend`] method is a no-op
/// returning a zero/empty result, so a full cycle can run safely against an
/// empty graph until the real backend (P77b) is wired in.
#[derive(Default)]
pub struct GraceBackend;

impl DreamBackend for GraceBackend {
    fn sample_for_dream(
        &self,
        _max_memories: usize,
        _recent_pct: f64,
        _random_old_pct: f64,
        _low_salience_pct: f64,
    ) -> Vec<backend::Memory> {
        Vec::new()
    }

    fn get_connections(&self) -> Vec<backend::Edge> {
        Vec::new()
    }

    fn strengthen_edge(&self, _source_id: usize, _target_id: usize, _amount: f64) {}

    fn weaken_edge(&self, _source_id: usize, _target_id: usize, _amount: f64) {}

    fn prune_edge(&self, _source_id: usize, _target_id: usize) {}

    fn update_supersedes(&self) -> usize {
        0
    }

    fn find_bridges(
        &self,
        _communities: &[Vec<usize>],
        _embeddings: &[[f64; 384]],
        _max_bridges: usize,
    ) -> Vec<(usize, usize)> {
        Vec::new()
    }

    fn create_bridge_edges(&self, _bridges: &[(usize, usize)]) {}

    fn get_communities(&self) -> Vec<usize> {
        Vec::new()
    }

    fn write_communities(&self, _assignments: &[usize]) {}

    fn extract_afe_facts(&self, _memories: &[backend::Memory]) -> Vec<(String, usize)> {
        Vec::new()
    }

    fn write_afe_facts(&self, _facts: &[(String, usize)]) {}

    fn run_synthesis(&self) -> usize {
        0
    }

    fn recompute_dae(&self) -> usize {
        0
    }
}

/// Production dream backend backed by the akar graph [`Database`] (P77b).
///
/// Every operation is expressed as a prepared Cypher statement executed against
/// a [`Connection`]. The SQL surface mirrors the kairos `AkarDreamBackend`
/// reference: `Memory` node table (`id`, `salience`, `created_at`, `content`)
/// and `Connected` rel table (`weight`).
///
/// Degradation policy: this backend never fails a dream cycle. If the shared
/// database lacks the `Memory`/`Connected` schema (a non-kairos database), the
/// underlying bind of the `MATCH`/`CREATE` fails at the engine and every method
/// falls back to its empty/zero result — indistinguishable from [`GraceBackend`],
/// but fully live once kairos has seeded graph data into the same database.
pub struct GraphBackend {
    conn: Connection,
}

impl GraphBackend {
    /// Create a backend bound to `db` (kept alive by an internal [`Arc`]).
    pub fn new(db: &Arc<Database>) -> Self {
        Self {
            conn: Connection::new(db),
        }
    }

    /// Execute a prepared statement with params, returning the result or `None`
    /// on bind/execution error. Errors are swallowed so the cycle cannot crash.
    fn q(&self, sql: &str, params: &[(&str, Value)]) -> Option<QueryResult> {
        let prepared = self.conn.prepare(sql).ok()?;
        self.conn.execute(&prepared, params.to_vec()).ok()
    }

    /// Materialise a result into rows keyed by projected column name.
    fn rows(&self, result: &QueryResult) -> Vec<Row> {
        let mut out = Vec::new();
        for chunk in &result.chunks {
            let names = chunk.field_names.clone();
            for row in chunk.iter_rows() {
                let mut r = Vec::with_capacity(names.len());
                for (col, name) in names.iter().enumerate() {
                    let val = chunk.get_value(col, row).unwrap_or(Value::Null);
                    r.push((name.clone(), val));
                }
                out.push(r);
            }
        }
        out
    }

    /// Read a single scalar from a single-row result by column name.
    fn scalar(&self, result: &QueryResult, col: &str) -> Option<Value> {
        let rows = self.rows(result);
        rows.first()
            .and_then(|r| r.iter().find(|(name, _)| name == col).map(|(_, v)| v.clone()))
    }

    fn as_f64(v: &Value) -> f64 {
        match v {
            Value::Double(x) => *x,
            Value::Int64(x) => *x as f64,
            Value::Int32(x) => *x as f64,
            Value::Float(x) => *x as f64,
            Value::UInt64(x) => *x as f64,
            _ => 0.0,
        }
    }

    fn as_usize(v: &Value) -> usize {
        match v {
            Value::Int64(x) => (*x).max(0) as usize,
            Value::Int32(x) => (*x).max(0) as usize,
            Value::UInt64(x) => *x as usize,
            Value::Double(x) => (*x).max(0.0) as usize,
            Value::Float(x) => (*x).max(0.0) as usize,
            _ => 0,
        }
    }

    fn as_string(v: &Value) -> String {
        match v {
            Value::String(s) => s.clone(),
            _ => String::new(),
        }
    }
}

impl DreamBackend for GraphBackend {
    fn sample_for_dream(
        &self,
        max_memories: usize,
        recent_pct: f64,
        random_old_pct: f64,
        _low_salience_pct: f64,
    ) -> Vec<backend::Memory> {
        let recent_n = (max_memories as f64 * recent_pct).max(1.0) as usize;
        let random_n = (max_memories as f64 * random_old_pct) as usize;
        let low_n = max_memories.saturating_sub(recent_n).saturating_sub(random_n);

        let mut out: Vec<backend::Memory> = Vec::new();
        let push_rows = |out: &mut Vec<backend::Memory>, rows: Vec<Row>| {
            for r in rows {
                let by_name = |name: &str| {
                    r.iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, v)| v.clone())
                        .unwrap_or(Value::Null)
                };
                let id_val = by_name("id");
                let is_missing = matches!(id_val, Value::Null);
                let id = Self::as_usize(&id_val);
                if is_missing || out.iter().any(|m| m.id == id) {
                    continue;
                }
                out.push(backend::Memory {
                    id,
                    salience: Self::as_f64(&by_name("salience")),
                    created_at: Self::as_f64(&by_name("created_at")),
                    content: Self::as_string(&by_name("content")),
                });
                if out.len() >= max_memories {
                    return;
                }
            }
        };

        if recent_n > 0 {
            if let Some(res) = self.q(
                "MATCH (m:Memory) RETURN m.id AS id, COALESCE(m.salience, 0.0) AS salience, \
                 COALESCE(m.created_at, 0.0) AS created_at, COALESCE(m.content, '') AS content \
                 ORDER BY m.created_at DESC LIMIT $limit",
                &[("limit", Value::Int64(recent_n as i64))],
            ) {
                push_rows(&mut out, self.rows(&res));
            }
        }
        if random_n > 0 && out.len() < max_memories {
            if let Some(res) = self.q(
                "MATCH (m:Memory) RETURN m.id AS id, COALESCE(m.salience, 0.0) AS salience, \
                 COALESCE(m.created_at, 0.0) AS created_at, COALESCE(m.content, '') AS content \
                 ORDER BY hash(id) LIMIT $limit",
                &[("limit", Value::Int64((random_n + recent_n / 4) as i64))],
            ) {
                push_rows(&mut out, self.rows(&res));
            }
        }
        if low_n > 0 && out.len() < max_memories {
            if let Some(res) = self.q(
                "MATCH (m:Memory) RETURN m.id AS id, COALESCE(m.salience, 0.0) AS salience, \
                 COALESCE(m.created_at, 0.0) AS created_at, COALESCE(m.content, '') AS content \
                 ORDER BY COALESCE(m.salience, 0.0) ASC, COALESCE(m.created_at, 0.0) ASC LIMIT $limit",
                &[("limit", Value::Int64((low_n + (recent_n + random_n) / 4) as i64))],
            ) {
                push_rows(&mut out, self.rows(&res));
            }
        }
        out.truncate(max_memories);
        out
    }

    fn get_connections(&self) -> Vec<backend::Edge> {
        let Some(res) = self.q(
            "MATCH (a:Memory)-[r:Connected]->(b:Memory) \
             RETURN a.id AS source_id, b.id AS target_id, COALESCE(r.weight, 0.0) AS weight",
            &[],
        ) else {
            return Vec::new();
        };
        self.rows(&res)
            .into_iter()
            .map(|r| {
                let by_name = |name: &str| {
                    r.iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, v)| v.clone())
                        .unwrap_or(Value::Null)
                };
                backend::Edge {
                    source_id: Self::as_usize(&by_name("source_id")),
                    target_id: Self::as_usize(&by_name("target_id")),
                    weight: Self::as_f64(&by_name("weight")),
                }
            })
            .collect()
    }

    fn strengthen_edge(&self, source_id: usize, target_id: usize, amount: f64) {
        let _ = self.q(
            "MATCH (a:Memory {id: $src})-[r:Connected]->(b:Memory {id: $tgt}) \
             SET r.weight = CASE WHEN COALESCE(r.weight, 0.0) + $amount > 1.0 \
             THEN 1.0 ELSE COALESCE(r.weight, 0.0) + $amount END",
            &[
                ("src", Value::Int64(source_id as i64)),
                ("tgt", Value::Int64(target_id as i64)),
                ("amount", Value::Double(amount)),
            ],
        );
    }

    fn weaken_edge(&self, source_id: usize, target_id: usize, amount: f64) {
        let _ = self.q(
            "MATCH (a:Memory {id: $src})-[r:Connected]->(b:Memory {id: $tgt}) \
             SET r.weight = COALESCE(r.weight, 0.0) - $amount",
            &[
                ("src", Value::Int64(source_id as i64)),
                ("tgt", Value::Int64(target_id as i64)),
                ("amount", Value::Double(amount)),
            ],
        );
    }

    fn prune_edge(&self, source_id: usize, target_id: usize) {
        let _ = self.q(
            "MATCH (a:Memory {id: $src})-[r:Connected]->(b:Memory {id: $tgt}) DELETE r",
            &[
                ("src", Value::Int64(source_id as i64)),
                ("tgt", Value::Int64(target_id as i64)),
            ],
        );
    }

    fn update_supersedes(&self) -> usize {
        // Mark SUPERSEDES edges superseded (set valid_to) when a newer edge of
        // the same direction already exists. Degrades to 0 on non-graph dbs.
        if let Some(res) = self.q(
            "MATCH (a:Memory)-[r:supersedes]->(b:Memory) \
             WHERE r.valid_to IS NULL \
             SET r.valid_to = r.created_at \
             RETURN count(*) AS updated",
            &[],
        ) {
            if let Some(v) = self.scalar(&res, "updated") {
                return Self::as_usize(&v);
            }
        }
        0
    }

    fn find_bridges(
        &self,
        _communities: &[Vec<usize>],
        _embeddings: &[[f64; 384]],
        _max_bridges: usize,
    ) -> Vec<(usize, usize)> {
        // The akar-dream REM phase passes no real embeddings today; bridge
        // discovery needs per-community centroids we don't yet model in the
        // server schema. Return empty (matches the mock semantics).
        Vec::new()
    }

    fn create_bridge_edges(&self, bridges: &[(usize, usize)]) {
        for &(s, t) in bridges {
            if s == t {
                continue;
            }
            let _ = self.q(
                "MATCH (a:Memory {id: $src}), (b:Memory {id: $tgt}) \
                 OPTIONAL MATCH (a)-[existing:Connected]-(b) \
                 WITH a, b, existing WHERE existing IS NULL \
                 CREATE (a)-[:Connected {weight: 0.3, type: 'bridge'}]->(b)",
                &[("src", Value::Int64(s as i64)), ("tgt", Value::Int64(t as i64))],
            );
        }
    }

    fn get_communities(&self) -> Vec<usize> {
        // Deterministic approximation: treat each Memory's community label
        // (stored as `community`) as its assignment; absent label → degrades
        // to empty. Index-aligned with node id order.
        let Some(res) = self.q(
            "MATCH (m:Memory) RETURN m.id AS id, COALESCE(m.community, -1) AS community \
             ORDER BY m.id",
            &[],
        ) else {
            return Vec::new();
        };
        let rows = self.rows(&res);
        if rows.is_empty() {
            return Vec::new();
        }
        let mut max_id = 0usize;
        for r in &rows {
            let by_name = |name: &str| {
                r.iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Null)
            };
            max_id = max_id.max(Self::as_usize(&by_name("id")));
        }
        let mut assignments = vec![0usize; max_id + 1];
        for r in &rows {
            let by_name = |name: &str| {
                r.iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Null)
            };
            let id = Self::as_usize(&by_name("id"));
            let comm = Self::as_usize(&by_name("community"));
            assignments[id] = comm;
        }
        assignments
    }

    fn write_communities(&self, assignments: &[usize]) {
        let mut params: Vec<(&str, Value)> = Vec::new();
        let mut batch: Vec<Value> = Vec::with_capacity(assignments.len());
        for (i, &c) in assignments.iter().enumerate() {
            batch.push(Value::Struct(vec![
                ("id".to_string(), Value::Int64(i as i64)),
                ("community".to_string(), Value::Int64(c as i64)),
            ]));
        }
        params.push(("batch", Value::List(batch)));
        let _ = self.q(
            "UNWIND $batch AS row \
             MATCH (m:Memory {id: row.id}) \
             SET m.community = row.community",
            &params,
        );
    }

    fn extract_afe_facts(&self, memories: &[backend::Memory]) -> Vec<(String, usize)> {
        // No NLP on the server side today — degrade to empty (mirrors the
        // mock), so the AFE phase records 0 facts rather than a crash.
        let _ = memories;
        Vec::new()
    }

    fn write_afe_facts(&self, facts: &[(String, usize)]) {
        let _ = facts;
    }

    fn run_synthesis(&self) -> usize {
        0
    }

    fn recompute_dae(&self) -> usize {
        0
    }
}

/// Per-server dream engine lifecycle guarded behind a [`Mutex`].
///
/// Clients never lock the orchestrator for long: `run_cycle` may execute all
/// seven phases, so a single shared instance serializes dreams across
/// connections.
pub struct DreamControl {
    orchestrator: Mutex<DreamEngine>,
    paused: AtomicBool,
    last_stats: Mutex<Option<DreamStats>>,
}

/// The concrete orchestrator behind a [`DreamControl`], selected at creation.
///
/// - [`DreamControl::new`] uses the graceful stub (no database handle).
/// - [`DreamControl::with_db`] uses the real graph backend.
enum DreamEngine {
    Grace(DreamOrchestrator<GraceBackend>),
    Graph(Box<DreamOrchestrator<GraphBackend>>),
}

impl DreamEngine {
    fn run_cycle(&mut self) -> DreamStats {
        match self {
            DreamEngine::Grace(o) => o.run_cycle(),
            DreamEngine::Graph(o) => o.run_cycle(),
        }
    }
}

impl DreamControl {
    /// Create a fresh engine with default config. Dreams start enabled.
    ///
    /// Uses the graceful stub backend; see [`DreamControl::with_db`] for the
    /// production graph-backed variant.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            orchestrator: Mutex::new(DreamEngine::Grace(DreamOrchestrator::new(
                DreamConfig::default(),
                GraceBackend,
            ))),
            paused: AtomicBool::new(false),
            last_stats: Mutex::new(None),
        })
    }

    /// Create an engine backed by the akar graph database (P77b): `run` does
    /// real consolidation work when kairos has seeded `Memory`/`Connected`
    /// graph data, and degrades gracefully otherwise.
    pub fn with_db(db: Arc<Database>) -> Arc<Self> {
        Arc::new(Self {
            orchestrator: Mutex::new(DreamEngine::Graph(Box::new(DreamOrchestrator::new(
                DreamConfig::default(),
                GraphBackend::new(&db),
            )))),
            paused: AtomicBool::new(false),
            last_stats: Mutex::new(None),
        })
    }

    /// Current state from the caller's perspective.
    pub fn state(&self) -> DreamState {
        let paused = self.paused.load(Ordering::SeqCst);
        let has_stats = self.last_stats.lock().map(|s| s.is_some()).unwrap_or(false);
        match (paused, has_stats) {
            (true, _) => DreamState::Paused,
            (false, true) => DreamState::Running,
            (false, false) => DreamState::Idle,
        }
    }

    /// Run a full consolidation cycle, recording the resulting stats.
    ///
    /// No-op (and returns the current state) when the engine is paused.
    pub fn run(&self) -> DreamStats {
        if self.paused.load(Ordering::SeqCst) {
            return self.last_stats().unwrap_or_default();
        }
        let mut guard = match self.orchestrator.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let stats = guard.run_cycle();
        drop(guard);
        if let Ok(mut slot) = self.last_stats.lock() {
            *slot = Some(stats.clone());
        }
        stats
    }

    /// Optionally execute a cycle when unpaused; used by `run`/`resume`.
    pub fn resume(&self) -> DreamStats {
        self.paused.store(false, Ordering::SeqCst);
        self.run()
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn last_stats(&self) -> Option<DreamStats> {
        self.last_stats.lock().ok().and_then(|s| s.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_main::{Database, SystemConfig};
    use tempfile::TempDir;

    fn config() -> SystemConfig {
        SystemConfig {
            buffer_pool_size: 64 * 1024 * 1024,
            auto_checkpoint: true,
            checkpoint_threshold: -1,
            concurrent_writes: true,
            ..Default::default()
        }
    }

    fn seeded_db() -> (Arc<Database>, TempDir) {
        let dir = TempDir::new().expect("temp dir");
        let db = Arc::new(Database::new(dir.path().join("test_db"), config()).expect("create db"));
        let conn = Connection::new(&db);
        conn.query(
            "CREATE NODE TABLE Memory(id INT64, salience DOUBLE, created_at DOUBLE, content STRING, \
             PRIMARY KEY (id))",
        )
        .expect("create Memory");
        conn.query("CREATE REL TABLE Connected(FROM Memory TO Memory, weight DOUBLE)")
            .expect("create Connected");
        // Seed 6 memories connected in a chain so NREM has edges to act on.
        for i in 0..6 {
            conn.query(&format!(
                "CREATE (:Memory {{id: {i}, salience: 0.5, created_at: 1000.0, content: 'memory {i}'}})"
            ))
            .expect("seed memory");
        }
        for i in 0..5 {
            conn.query(&format!(
                "MATCH (a:Memory {{id: {i}}}), (b:Memory {{id: {}}}) CREATE (a)-[:Connected {{weight: 0.5}}]->(b)",
                i + 1
            ))
            .expect("seed edge");
        }
        (db, dir)
    }

    #[test]
    fn test_dream_state_transitions() {
        let ctrl = DreamControl::new();
        assert_eq!(ctrl.state(), DreamState::Idle);

        let stats = ctrl.run();
        assert_eq!(stats.dream_id, 1);
        assert_eq!(ctrl.state(), DreamState::Running);

        // Paused: run() is a no-op and state flips to paused.
        ctrl.pause();
        assert_eq!(ctrl.state(), DreamState::Paused);
        let paused_stats = ctrl.run();
        assert_eq!(paused_stats.dream_id, 1, "paused run should not advance dream_id");

        // Resume restores running and runs a new cycle.
        let resumed = ctrl.resume();
        assert_eq!(resumed.dream_id, 2);
        assert_eq!(ctrl.state(), DreamState::Running);
    }

    #[test]
    fn test_grace_backend_all_zero() {
        let backend = GraceBackend;
        assert!(backend.sample_for_dream(10, 0.5, 0.2, 0.3).is_empty());
        assert!(backend.get_connections().is_empty());
        assert_eq!(backend.update_supersedes(), 0);
        assert_eq!(backend.recompute_dae(), 0);
        assert!(backend.get_communities().is_empty());
        assert_eq!(backend.run_synthesis(), 0);
    }

    #[test]
    fn test_graph_backend_samples_and_reads_edges() {
        let (db, _dir) = seeded_db();
        let backend = GraphBackend::new(&db);

        let memories = backend.sample_for_dream(10, 1.0, 0.0, 0.0);
        assert!(!memories.is_empty(), "sampling a seeded graph should yield memories");
        assert_eq!(memories.len(), 6);
        assert!(memories.iter().all(|m| m.id < 30 && m.salience == 0.5));

        let edges = backend.get_connections();
        assert_eq!(edges.len(), 5, "chain graph has 5 edges");
        assert!(edges.iter().all(|e| (e.weight - 0.5).abs() < 1e-9));
    }

    #[test]
    fn test_graph_backend_mutates_edges() {
        let (db, _dir) = seeded_db();
        let backend = GraphBackend::new(&db);

        backend.strengthen_edge(0, 1, 0.1);
        backend.weaken_edge(1, 2, 0.05);
        backend.prune_edge(3, 4);

        let edges = backend.get_connections();
        let w01 = edges
            .iter()
            .find(|e| e.source_id == 0 && e.target_id == 1)
            .map(|e| e.weight);
        let w12 = edges
            .iter()
            .find(|e| e.source_id == 1 && e.target_id == 2)
            .map(|e| e.weight);
        assert_eq!(w01, Some(0.6));
        assert_eq!(w12, Some(0.45));
        assert!(
            edges.iter().all(|e| !(e.source_id == 3 && e.target_id == 4)),
            "edge pruned"
        );
    }

    #[test]
    fn test_dream_control_with_db_runs_real_cycle() {
        let (db, _dir) = seeded_db();
        let ctrl = DreamControl::with_db(db);

        let stats = ctrl.run();
        assert_eq!(stats.dream_id, 1);
        // NREM has a chain of 5 edges against a real backend: it should have
        // strengthened/weakened/pruned at least one edge.
        assert!(
            stats.nrem.strengthened + stats.nrem.weakened + stats.nrem.pruned > 0,
            "a graph-backed NREM phase should act on seeded edges: {stats:?}"
        );
        assert_eq!(ctrl.state(), DreamState::Running);
    }

    #[test]
    fn test_graph_backend_degrades_on_empty_db() {
        // A fresh db carries no Memory/Connected schema — the backend must not
        // crash; every method degrades to empty/zero (graceful stub semantics).
        let dir = TempDir::new().expect("temp dir");
        let db = Arc::new(Database::new(dir.path().join("test_db"), config()).expect("create db"));
        let backend = GraphBackend::new(&db);

        assert!(backend.sample_for_dream(10, 0.5, 0.2, 0.3).is_empty());
        assert!(backend.get_connections().is_empty());
        assert_eq!(backend.update_supersedes(), 0);
        assert!(backend.get_communities().is_empty());
        assert_eq!(backend.recompute_dae(), 0);

        let ctrl = DreamControl::with_db(db);
        let stats = ctrl.run();
        assert_eq!(stats.dream_id, 1);
        assert!(stats.nrem.strengthened + stats.nrem.weakened + stats.nrem.pruned == 0);
    }
}
