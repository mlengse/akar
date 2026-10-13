//! T033 probe matrix — empty-result schema behaviour (P136 residue / FR-015).
//!
//! P130 closed the *projection* path: `PhysicalProjection` rebuilds the logical
//! schema for zero-row input. The remaining P136 class is every OTHER path that
//! emits or reads a zero-row chunk: exhausted non-projection scans, `UNWIND`
//! over an empty list, name-based consumer reads. This probe runs the matrix
//! against the current build and records, per path, whether the zero-row result
//! still carries its schema (`field_names`) or arrives bare.
//!
//! A path that succeeds but returns a zero-row chunk with EMPTY `field_names`
//! is a live reproduction of the defect (the consumer then fails with
//! `Variable 'X' not found (chunk has no field_names)`); a hard query error is
//! a stronger failure. Both are printed as findings; the final assertion
//! requires every path to at least execute, so the gate stays green while the
//! schema state is recorded. T034 (the P136 fix) tightens this into a
//! contract test.

use akar_main::{Connection, Database, SystemConfig};
use std::sync::Arc;

struct Outcome {
    path: &'static str,
    sql: &'static str,
    ok: bool,
    error: String,
    rows: usize,
    field_names: Vec<String>,
}

fn setup() -> (tempfile::TempDir, Arc<Database>, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    (dir, db, conn)
}

fn probe(conn: &Connection, path: &'static str, sql: &'static str) -> Outcome {
    match conn.query(sql) {
        Ok(result) => {
            let chunk = result.chunks.first();
            Outcome {
                path,
                sql,
                ok: true,
                error: String::new(),
                rows: chunk.map_or(0, |c| c.size),
                field_names: chunk.map_or_else(Vec::new, |c| c.field_names.clone()),
            }
        }
        Err(error) => Outcome {
            path,
            sql,
            ok: false,
            error: error.to_string(),
            rows: 0,
            field_names: Vec::new(),
        },
    }
}

fn empty_schema() -> (tempfile::TempDir, Arc<Database>, Connection) {
    let (dir, db, conn) = setup();
    conn.query("CREATE NODE TABLE Probe(id INT64, name STRING, PRIMARY KEY (id))")
        .expect("create node table");
    conn.query("CREATE REL TABLE Linked(FROM Probe TO Probe)")
        .expect("create rel table");
    (dir, db, conn)
}

#[test]
fn empty_result_schema_matrix() {
    let (_dir, _db, conn) = empty_schema();

    let cases: Vec<(&'static str, &'static str)> = vec![
        // P130's closed path (baseline): projection over an empty source.
        (
            "P1 projection + ORDER BY alias",
            "MATCH (n:Probe) RETURN n.id AS id ORDER BY id",
        ),
        // The recorded manifestation: ORDER BY over the source expression.
        (
            "P2 projection + ORDER BY source expr",
            "MATCH (n:Probe) RETURN n.id AS id ORDER BY n.id",
        ),
        // Non-projection returns over an exhausted scan.
        ("P3 bare node return (no projection)", "MATCH (n:Probe) RETURN n"),
        ("P4 node property return (no alias)", "MATCH (n:Probe) RETURN n.id"),
        // UNWIND over an empty list.
        ("P5 UNWIND empty list", "UNWIND [] AS x RETURN x"),
        ("P6 UNWIND empty list + ORDER BY", "UNWIND [] AS x RETURN x ORDER BY x"),
        // Aggregation over an empty source.
        ("P7 aggregate over empty", "MATCH (n:Probe) RETURN count(n) AS c"),
        // Pipeline (WITH) over an empty source then ORDER BY.
        (
            "P8 WITH pipeline + ORDER BY",
            "MATCH (n:Probe) WITH n RETURN n ORDER BY n.id",
        ),
        // Exhausted relationship scan (the sulur connection_counts shape).
        (
            "P9 rel scan (empty pool)",
            "MATCH (a:Probe)-[r:Linked]->(b:Probe) RETURN a.id AS source",
        ),
        (
            "P10 rel var return (empty pool)",
            "MATCH (a:Probe)-[r:Linked]->(b:Probe) RETURN r",
        ),
        // Table function over an empty database table.
        (
            "P11 table function",
            "MATCH (n:Probe) RETURN n.id AS id ORDER BY id LIMIT 10",
        ),
    ];

    let outcomes: Vec<Outcome> = cases.iter().map(|(path, sql)| probe(&conn, path, sql)).collect();

    println!("\n=== T033 empty-result schema matrix (akar, current build) ===");
    let mut bare_schema = 0usize;
    let mut hard_errors = 0usize;
    for outcome in &outcomes {
        let verdict = if !outcome.ok {
            hard_errors += 1;
            format!("HARD ERROR: {}", outcome.error)
        } else if outcome.rows == 0 && outcome.field_names.is_empty() {
            bare_schema += 1;
            "LIVE REPRO: zero rows, EMPTY field_names".to_string()
        } else if outcome.rows == 0 {
            format!("ok: zero rows carry schema {:?}", outcome.field_names)
        } else {
            format!("ok: {} rows, schema {:?}", outcome.rows, outcome.field_names)
        };
        println!("[{}] {} -> {}", outcome.path, outcome.sql, verdict);
    }
    println!(
        "=== summary: {} hard error(s), {} bare-schema path(s) ===\n",
        hard_errors, bare_schema
    );

    // T034 contract: every path must execute, and every zero-row result must
    // carry its schema. A bare zero-row chunk (empty `field_names`) is the P136
    // defect — live-reproduced by this matrix on `ORDER BY ... LIMIT` before
    // the `PhysicalLimit` fix (limit.rs).
    for outcome in &outcomes {
        assert!(
            outcome.ok,
            "T033: path [{}] failed to execute: {}",
            outcome.path, outcome.error
        );
        if outcome.rows == 0 {
            assert!(
                !outcome.field_names.is_empty(),
                "T033/T034: path [{}] (`{}`) returned a zero-row result with EMPTY \
                 field_names — the P136 defect is live on this path",
                outcome.path,
                outcome.sql
            );
        }
    }
}
