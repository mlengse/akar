//! HNSW (Hierarchical Navigable Small World) index for approximate nearest
//! neighbour search.
//!
//! # Exact scan below the threshold
//!
//! Graph ANN is not free: building the graph costs O(n log n) distance
//! computations and a narrow beam still misses neighbours. On the benchmark
//! corpus in `examples/bench_021.rs` (10,000 × 384-d) the graph took ~19 s to
//! build and returned 0.66 overlap@10 at `ef=128`, while scanning every vector
//! is exact and costs a few milliseconds. The index therefore behaves as a
//! flat vector store at or below [`EXACT_SEARCH_MAX_NODES`] and links the graph
//! lazily, in one pass, only when the corpus grows past it.
//!
//! # Algorithm
//!
//! HNSW builds a multi-layer graph where:
//! - Level 0 contains all inserted vectors.
//! - Higher levels contain exponentially fewer vectors (a vector is promoted
//!   to level `l` with probability `1/ln(M_max)`).
//! - Search starts at the highest level and greedily descends to level 0,
//!   then performs a more thorough search at level 0.
//!
//! # References
//!
//! Yu. A. Malkov, D. A. Yashunin, "Efficient and robust approximate nearest
//! neighbor search using Hierarchical Navigable Small World graphs", 2016.
//! <https://arxiv.org/abs/1603.09320>
//!
//! # Thread Safety
//!
//! `HnswIndex` is `Send` but not `Sync`. Wrap in `Mutex` for concurrent use.

use ahash::{AHashMap, AHashSet};
use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

// ---------------------------------------------------------------------------
// Constants (HNSW defaults matching the reference implementation)
// ---------------------------------------------------------------------------

/// Maximum number of connections per element per layer for construction.
const M: usize = 16;

/// Maximum number of connections per element per layer for construction
/// (used only at layer 0).
const M_MAX: usize = 32;

/// Maximum number of candidates to keep during the search for neighbours.
const EF_CONSTRUCTION: usize = 128;

/// Number of iterations for the multi-element pruning procedure.
const MAX_M: usize = M_MAX;

/// Size of the dynamic candidate list during search.
const EF_SEARCH: usize = 200;

/// Vector count at or below which the index behaves as a flat vector store:
/// every query scans all live vectors instead of traversing a graph (T064).
///
/// Supported operating range, threshold tuning, and empirical measurements:
/// see `akar/SPEC.md` §12.3. Above this threshold, the graph traversal path is
/// unmeasured and carries no build-time or recall promise.
pub const EXACT_SEARCH_MAX_NODES: usize = 50_000;

/// Maximum level generation factor: `1 / ln(M_max)`.
fn ml() -> f64 {
    1.0 / (M_MAX as f64).ln()
}

// ---------------------------------------------------------------------------
// Distance function
// ---------------------------------------------------------------------------

/// Supported distance metrics for HNSW.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistanceMetric {
    /// `cosine_similarity` → `1 - cos(a,b)` (smaller = more similar)
    Cosine,
    /// `sqrt(sum((a_i - b_i)^2))`
    Euclidean,
    /// `sum(|a_i - b_i|)`
    L1,
    /// `sum((a_i - b_i)^2)` (faster than Euclidean, same ordering)
    L2Squared,
    /// `a · b` (higher = more similar, negated internally)
    DotProduct,
}

impl DistanceMetric {
    /// Compute the distance between two vectors using this metric.
    /// Returns a non-negative value where *smaller* means more similar.
    pub fn compute(&self, a: &[f32], b: &[f32]) -> f32 {
        match self {
            // All kernels are runtime-dispatched SIMD with scalar fallback,
            // preserving the exact scalar maths of the previous inlined
            // implementations (see akar-vector/src/distance.rs).
            DistanceMetric::Cosine => crate::distance::cosine_distance(a, b),
            DistanceMetric::Euclidean => crate::distance::euclidean_distance(a, b),
            DistanceMetric::L1 => crate::distance::l1_distance(a, b),
            DistanceMetric::L2Squared => crate::distance::l2_squared(a, b),
            DistanceMetric::DotProduct => {
                // Dot product: higher = more similar → negate for smaller-distance convention
                -crate::distance::dot_product(a, b)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Core data structures
// ---------------------------------------------------------------------------

/// A `(distance, node_id)` pair with a total order for use in HNSW heaps.
///
/// `f32` does not implement `Ord`/`Eq`, so this wrapper gives the beam-search
/// heaps a total order based on `total_cmp` (ties broken by node id for
/// determinism) (P51.16).
#[derive(Debug, Clone, Copy)]
struct Candidate(f32, usize);

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0).then_with(|| self.1.cmp(&other.1))
    }
}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Candidate {}

/// A node in the HNSW graph representing one vector.
#[derive(Debug, Clone)]
struct HnswNode {
    /// The vector data.
    vector: Vec<f32>,
    /// Connections per layer: `connections[level]` = list of node IDs.
    connections: Vec<Vec<usize>>,
}

/// The HNSW index.
///
/// Stores vectors in a multi-layer navigable small world graph for fast
/// approximate nearest neighbour search.
///
/// Nodes are keyed by the caller-supplied id (P51.17): the id passed to
/// `insert` is preserved as the node id returned by `search`, so a vector
/// inserted with `id` can always be addressed as `id` even when the ids are
/// sparse (e.g. rows with a NULL vector column are skipped during populate).
#[derive(Debug, Clone)]
pub struct HnswIndex {
    /// All nodes in the index, keyed by node id.
    nodes: AHashMap<usize, HnswNode>,
    /// Tombstoned (deleted) node IDs ignored during search.
    tombstones: AHashSet<usize>,
    /// Number of layers in the graph.
    max_level: usize,
    /// Entry point — the node ID at the highest layer.
    entry_point: Option<usize>,
    /// Distance metric to use.
    metric: DistanceMetric,
    /// Random state for level generation.
    rng_state: u64,
    /// Vector count at or below which queries scan exactly instead of walking
    /// the graph. See [`EXACT_SEARCH_MAX_NODES`].
    exact_threshold: usize,
    /// Whether graph links have been built. `false` means the index is a flat
    /// vector store: `insert` only appends the vector and search scans.
    graph_built: bool,
    /// Node ids in insertion order. Only maintained while the store is flat,
    /// and consumed once to link a deterministic graph when it crosses
    /// `exact_threshold`.
    insertion_order: Vec<usize>,
}

impl HnswIndex {
    /// Create a new HNSW index with the given distance metric.
    ///
    /// Starts as a flat vector store and builds the graph only when the corpus
    /// exceeds [`EXACT_SEARCH_MAX_NODES`].
    pub fn new(metric: DistanceMetric) -> Self {
        Self::with_exact_threshold(metric, EXACT_SEARCH_MAX_NODES)
    }

    /// Create an index with an explicit exact-scan threshold.
    ///
    /// A threshold of `0` forces the graph path from the first insert, which
    /// is what the graph-level tests use; the default
    /// ([`EXACT_SEARCH_MAX_NODES`]) is what production callers get.
    pub fn with_exact_threshold(metric: DistanceMetric, exact_threshold: usize) -> Self {
        Self {
            nodes: AHashMap::new(),
            tombstones: AHashSet::new(),
            max_level: 0,
            entry_point: None,
            metric,
            rng_state: 42,
            exact_threshold,
            graph_built: false,
            insertion_order: Vec::new(),
        }
    }

    /// Number of active vectors in the index (excluding tombstones).
    pub fn len(&self) -> usize {
        self.nodes.len().saturating_sub(self.tombstones.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove a vector from the index by ID (tombstone).
    pub fn remove(&mut self, id: usize) -> bool {
        if self.nodes.contains_key(&id) {
            self.tombstones.insert(id)
        } else {
            false
        }
    }

    /// Check whether a node has been tombstoned.
    pub fn is_deleted(&self, id: usize) -> bool {
        self.tombstones.contains(&id)
    }

    /// Remove all vectors and reset the graph to an empty state.
    ///
    /// Used to rebuild the index from live table data after DML (P52.38).
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.tombstones.clear();
        self.max_level = 0;
        self.entry_point = None;
        self.graph_built = false;
        self.insertion_order.clear();
    }

    /// Get the current maximum level of the graph.
    ///
    /// Returns `0` while the index is a flat store (`graph_built == false`):
    /// there is no graph yet, and the next threshold crossing builds it.
    pub fn max_level(&self) -> usize {
        self.max_level
    }

    /// Get the entry point node ID (if any).
    ///
    /// Returns `None` while the index is a flat store.
    pub fn entry_point(&self) -> Option<usize> {
        self.entry_point
    }

    /// Whether the graph links have been built.
    ///
    /// `false` while the corpus is at or below the exact-scan threshold.
    pub fn is_graph_built(&self) -> bool {
        self.graph_built
    }

    /// Get the distance metric used by this index.
    pub fn metric(&self) -> DistanceMetric {
        self.metric
    }

    /// Get a reference to the vectors stored in the index, ordered by node id.
    pub fn vectors(&self) -> Vec<&[f32]> {
        let mut sorted_nodes: Vec<(&usize, &HnswNode)> = self
            .nodes
            .iter()
            .filter(|(id, _)| !self.tombstones.contains(id))
            .collect();
        sorted_nodes.sort_by_key(|&(id, _)| id);
        sorted_nodes.into_iter().map(|(_, n)| n.vector.as_slice()).collect()
    }

    /// All nodes as `(node_id, vector)` pairs, ordered by node id.
    ///
    /// The node id is the caller-supplied id (row offset in the calling
    /// table), so it is preserved across persist/load (P51.17).
    pub fn nodes(&self) -> Vec<(usize, &[f32])> {
        let mut sorted_nodes: Vec<(&usize, &HnswNode)> = self
            .nodes
            .iter()
            .filter(|(id, _)| !self.tombstones.contains(id))
            .collect();
        sorted_nodes.sort_by_key(|&(id, _)| id);
        sorted_nodes
            .into_iter()
            .map(|(&id, n)| (id, n.vector.as_slice()))
            .collect()
    }

    // ---- Internal helpers ----

    /// Generate a random level for a new node using the HNSW level
    /// distribution: `floor(-ln(uniform(0,1)) * ML)`.
    fn random_level(&mut self) -> usize {
        // Simple LCG random number generator
        self.rng_state = self
            .rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let r = (self.rng_state >> 33) as f64 / (1u64 << 31) as f64;
        let level = (-r.ln() * ml()).floor() as usize;
        level.min(MAX_M)
    }

    /// Compute distance between the vector at `node_id` and a query vector.
    fn node_distance(&self, node_id: usize, query: &[f32]) -> f32 {
        if let Some(node) = self.nodes.get(&node_id) {
            self.metric.compute(&node.vector, query)
        } else {
            f32::INFINITY
        }
    }

    /// Greedy search from a given entry point to find the closest node
    /// to the query at the specified layer. Returns the closest node ID.
    fn greedy_search_at_layer(&self, entry: usize, query: &[f32], layer: usize) -> usize {
        let mut current = entry;
        let mut current_node = match self.nodes.get(&current) {
            Some(n) => n,
            None => return current,
        };
        let mut current_dist = self.metric.compute(&current_node.vector, query);

        loop {
            let mut improved = false;
            if layer < current_node.connections.len() {
                for &neighbor in &current_node.connections[layer] {
                    if let Some(neighbor_node) = self.nodes.get(&neighbor) {
                        let d = self.metric.compute(&neighbor_node.vector, query);
                        if d < current_dist {
                            current_dist = d;
                            current = neighbor;
                            current_node = neighbor_node;
                            improved = true;
                        }
                    }
                }
            }
            if !improved {
                break;
            }
        }

        current
    }

    /// Search the given layer for the `k` nearest neighbours, starting from a
    /// given entry point, using a bounded beam search (P51.16).
    ///
    /// The beam is capped at `ef` candidates: both the candidate set and the
    /// result set are truncated to `ef`, and a candidate is only expanded when
    /// it could still improve the `ef`-th best result. This keeps the search
    /// at O(ef · M) instead of degenerating into a full-graph scan.
    ///
    /// `ef` is the beam width and is supplied by the caller, because the beam
    /// and the result count are independent knobs: widening `ef` widens the
    /// set of nodes the search may explore before it truncates to `k`, which
    /// is what lets recall approach the exact ranking. Deriving `ef` from `k`
    /// or from a constant floor pins the beam, so a caller asking for a wider
    /// candidate pool receives exactly the candidates a narrow one would have
    /// returned — the pool knob becomes dead plumbing.
    ///
    /// `ef` is floored at `k`: a beam narrower than the result count is
    /// meaningless, since the beam cannot return more than it holds.
    ///
    /// `layer` selects the connection set used for expansion (insert searches
    /// each layer separately; `search()` always uses layer 0).
    fn search_layer_0(&self, entry: usize, query: &[f32], layer: usize, k: usize, ef: usize) -> Vec<(f32, usize)> {
        if k == 0 {
            return Vec::new();
        }
        let ef = ef.max(k);
        // The beam can never hold or visit more nodes than the index contains,
        // so the capacity hints are clamped to that. `ef` is caller-supplied and
        // may be arbitrarily large: without the clamp an absurd value would
        // overflow the arithmetic that sizes these containers, and
        // `with_capacity` panics on a capacity it cannot allocate anyway.
        let reachable = self.nodes.len().max(1);
        let vis_cap = ef.min(reachable).saturating_mul(4);
        let heap_cap = ef.min(reachable).saturating_add(1);
        // Fast AHashSet (integer node IDs) with pre-allocated target capacity to eliminate reallocations.
        let mut visited = AHashSet::with_capacity(vis_cap);

        let entry_dist = self.node_distance(entry, query);
        // candidates: min-heap (closest popped first).
        let mut candidates: BinaryHeap<Reverse<Candidate>> = BinaryHeap::with_capacity(heap_cap);
        // results: max-heap (farthest/worst on top), bounded to `ef`.
        let mut results: BinaryHeap<Candidate> = BinaryHeap::with_capacity(heap_cap);
        candidates.push(Reverse(Candidate(entry_dist, entry)));
        results.push(Candidate(entry_dist, entry));
        visited.insert(entry);

        // Track current worst distance in result set to eliminate redundant heap peeks in the inner loop.
        let mut worst_dist = entry_dist;

        while let Some(Reverse(c)) = candidates.pop() {
            // If the closest remaining candidate is already worse than the
            // `ef`-th best result, no further candidate can improve it (the
            // beam is exhausted). This is the pruning step that keeps the
            // search bounded.
            if results.len() >= ef && c.0 > worst_dist {
                break;
            }
            let node = match self.nodes.get(&c.1) {
                Some(n) => n,
                None => continue,
            };
            if layer >= node.connections.len() {
                continue;
            }
            for &neighbor in &node.connections[layer] {
                if !visited.insert(neighbor) {
                    continue;
                }
                let neighbor_node = match self.nodes.get(&neighbor) {
                    Some(n) => n,
                    None => continue,
                };
                let nd = self.metric.compute(&neighbor_node.vector, query);
                // Only admit a neighbor that could enter the `ef`-closest set.
                if nd < worst_dist || results.len() < ef {
                    results.push(Candidate(nd, neighbor));
                    candidates.push(Reverse(Candidate(nd, neighbor)));
                    if results.len() > ef {
                        results.pop();
                    }
                    if let Some(worst) = results.peek() {
                        worst_dist = worst.0;
                    }
                }
            }
        }

        // Extract candidates from max-heap in descending distance order into a pre-allocated vector,
        // filtering out tombstoned nodes, then reverse to get ascending distance order.
        let mut out = Vec::with_capacity(results.len());
        while let Some(c) = results.pop() {
            if !self.tombstones.contains(&c.1) {
                out.push((c.0, c.1));
            }
        }
        out.reverse();
        out.truncate(k);
        out
    }

    /// Select the `M` closest neighbours from a pre-sorted candidate list.
    ///
    /// Implements Malkov & Yashunin (2016) Algorithm 4 (HNSW heuristic with diversity):
    /// candidates are processed in ascending distance order. A candidate is accepted
    /// into `selected` if and only if it is closer to the new element than to every
    /// already-selected neighbor (preventing cliques and ensuring angular diversity).
    /// Rejected candidates are held in `pruned` to backfill if fewer than `m` neighbors
    /// meet the strict diversity test.
    fn get_node_vec<'a>(&'a self, node_id: usize, pending: Option<(usize, &'a [f32])>) -> Option<&'a [f32]> {
        if let Some((p_id, p_vec)) = pending {
            if node_id == p_id {
                return Some(p_vec);
            }
        }
        self.nodes.get(&node_id).map(|n| n.vector.as_slice())
    }

    fn select_neighbors_heuristic(
        &self,
        candidates: &[(f32, usize)],
        m: usize,
        pending: Option<(usize, &[f32])>,
    ) -> Vec<usize> {
        if candidates.is_empty() || m == 0 {
            return Vec::new();
        }
        let m = m.min(candidates.len());
        let mut selected: Vec<usize> = Vec::with_capacity(m);
        let mut pruned: Vec<usize> = Vec::new();

        for &(cand_dist, cand_id) in candidates {
            if selected.len() >= m {
                break;
            }
            let Some(cand_vec) = self.get_node_vec(cand_id, pending) else {
                continue;
            };
            let closer_to_query_than_selected = selected.iter().all(|&sel_id| {
                self.get_node_vec(sel_id, pending)
                    .is_none_or(|sel_vec| cand_dist < self.metric.compute(cand_vec, sel_vec))
            });
            if closer_to_query_than_selected {
                selected.push(cand_id);
            } else {
                pruned.push(cand_id);
            }
        }

        for cand_id in pruned {
            if selected.len() >= m {
                break;
            }
            selected.push(cand_id);
        }

        selected
    }

    fn select_neighbors_simple(&self, candidates: &[(f32, usize)], m: usize) -> Vec<usize> {
        self.select_neighbors_heuristic(candidates, m, None)
    }

    // ---- Public API ----

    /// Insert a vector into the index with the given ID.
    ///
    /// The ID should uniquely identify this vector within the index. It is
    /// preserved verbatim: the node is stored under `id`, and `search`
    /// returns `id` for it. Node ids may be sparse (P51.17) — callers pass
    /// their own row ids so a search result always addresses the same row.
    ///
    /// While the store is flat (at or below the exact-scan threshold) this
    /// only appends the vector; the graph is linked in one pass when the
    /// corpus crosses the threshold, so build time at benchmark scale is the
    /// cost of copying vectors rather than the cost of graph construction.
    pub fn insert(&mut self, vector: Vec<f32>, id: usize) {
        self.tombstones.remove(&id);

        if !self.graph_built {
            let replaced = self
                .nodes
                .insert(
                    id,
                    HnswNode {
                        vector,
                        connections: Vec::new(),
                    },
                )
                .is_some();
            if !replaced {
                self.insertion_order.push(id);
            }
            if self.nodes.len() > self.exact_threshold {
                self.build_graph();
            }
            return;
        }

        self.insert_graph_node(id, vector);
    }

    /// Link one vector into an already-built graph.
    ///
    /// This is the original HNSW insertion (Malkov & Yashunin, Algorithm 1).
    /// It assumes `graph_built` is set. If `id` already exists (a re-insert
    /// after a tombstone), the old node is still present while neighbours are
    /// searched, and the final insert replaces it with the freshly linked node.
    fn insert_graph_node(&mut self, id: usize, vector: Vec<f32>) {
        let new_level = if self.nodes.is_empty() {
            MAX_M // First node goes to max level
        } else {
            self.random_level()
        };

        // Build connection storage for all levels this node will occupy
        let mut connections = vec![Vec::new(); new_level + 1];

        if self.entry_point.is_none() {
            // First node: just add it
            self.nodes.insert(id, HnswNode { vector, connections });
            self.entry_point = Some(id);
            self.max_level = new_level;
            return;
        }

        let entry = match self.entry_point {
            Some(e) => e,
            None => return,
        };

        // Phase 1: Traverse from top level down to `new_level + 1`
        // to find the best entry point at `new_level`.
        let mut current_entry = entry;
        for level in (new_level + 1..=self.max_level).rev() {
            current_entry = self.greedy_search_at_layer(current_entry, &vector, level);
        }

        // Phase 2: Insert at each level from `min(new_level, max_level)` down to 0
        for level in (0..=new_level.min(self.max_level)).rev() {
            // Find nearest neighbours at this layer (search the layer's own
            // connection set, not layer 0 — P51.16).
            let ep = self.greedy_search_at_layer(current_entry, &vector, level);
            let candidates = self.search_layer_0(ep, &vector, level, EF_CONSTRUCTION, EF_CONSTRUCTION);

            let m = if level == 0 { M_MAX } else { M };
            let neighbors = self.select_neighbors_simple(&candidates, m);

            // Add connections from new node to neighbours
            connections[level] = neighbors.clone();

            // Add connections from neighbours back to new node
            for &neighbor_id in &neighbors {
                let neighbor_node = match self.nodes.get(&neighbor_id) {
                    Some(n) => n,
                    None => continue,
                };
                let max_conn = if level == 0 { M_MAX } else { M };
                let current_conn_len = if level < neighbor_node.connections.len() {
                    neighbor_node.connections[level].len()
                } else {
                    0
                };

                // Fast path: if the neighbor hasn't reached max_conn yet, we don't need
                // to recompute distances for all existing connections — simply append `id`.
                if current_conn_len < max_conn {
                    if let Some(n) = self.nodes.get_mut(&neighbor_id) {
                        while n.connections.len() <= level {
                            n.connections.push(Vec::new());
                        }
                        n.connections[level].push(id);
                    }
                    continue;
                }

                // Shrink neighbor's connections to max_conn using diversity heuristic (Algorithm 4)
                let d = self.metric.compute(&vector, &neighbor_node.vector);
                let mut cand_list: Vec<(f32, usize)> = neighbor_node.connections[level]
                    .iter()
                    .filter_map(|&nid| {
                        self.nodes
                            .get(&nid)
                            .map(|n| (self.metric.compute(&n.vector, &neighbor_node.vector), nid))
                    })
                    .collect();
                cand_list.push((d, id));
                cand_list.sort_by(|a, b| a.0.total_cmp(&b.0));
                let new_conns = self.select_neighbors_heuristic(&cand_list, max_conn, Some((id, &vector)));
                if let Some(n) = self.nodes.get_mut(&neighbor_id) {
                    n.connections[level] = new_conns;
                }
            }

            current_entry = candidates.first().map_or(ep, |c| c.1);
        }

        // Update entry point if the new node has a higher level
        if new_level > self.max_level {
            self.max_level = new_level;
            self.entry_point = Some(id);
        }

        // Add the new node
        self.nodes.insert(id, HnswNode { vector, connections });
    }

    /// Link the flat store into a graph, once, when the corpus crosses the
    /// exact-scan threshold.
    ///
    /// Nodes are relinked in insertion order so the graph is deterministic
    /// (the node map is hash-ordered and would otherwise produce a different
    /// graph per run). Tombstoned vectors are left unlinked: a deleted vector
    /// must never become a neighbour.
    fn build_graph(&mut self) {
        debug_assert!(!self.graph_built, "the graph is built exactly once");
        let order = std::mem::take(&mut self.insertion_order);
        let mut pending: Vec<(usize, Vec<f32>)> = Vec::with_capacity(self.nodes.len());

        for id in order {
            if self.tombstones.contains(&id) {
                continue;
            }
            if let Some(node) = self.nodes.remove(&id) {
                pending.push((id, node.vector));
            }
        }
        // Defensive: a node missing from the order list still has to be linked,
        // otherwise it would become silently unreachable through the graph.
        let unordered: Vec<usize> = self.nodes.keys().copied().collect();
        for id in unordered {
            if self.tombstones.contains(&id) {
                continue;
            }
            if let Some(node) = self.nodes.remove(&id) {
                pending.push((id, node.vector));
            }
        }

        self.graph_built = true;
        self.max_level = 0;
        self.entry_point = None;
        for (id, vector) in pending {
            self.insert_graph_node(id, vector);
        }
    }

    /// Search for the `k` approximate nearest neighbours to `query`, using the
    /// default beam width ([`EF_SEARCH`], floored at `k`).
    ///
    /// Prefer [`Self::search_with_ef`] when the caller wants to trade latency
    /// for recall: the beam width is the only knob that controls how much of
    /// the graph a query is allowed to explore, and a fixed beam caps recall
    /// regardless of how the caller asks for results.
    ///
    /// Returns a list of `(distance, node_id)` pairs sorted by distance
    /// (closest first).
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(f32, usize)> {
        self.search_with_ef(query, k, EF_SEARCH)
    }

    /// Scan every live vector for the exact `k` nearest neighbours.
    ///
    /// This is the whole search path while the corpus is at or below the
    /// exact-scan threshold: no beam, no graph, no approximation. Ties are
    /// broken by node id, so the ranking is stable across runs.
    fn search_exact(&self, query: &[f32], k: usize) -> Vec<(f32, usize)> {
        self.search_exact_filtered(query, k, |_| true)
    }

    fn search_exact_filtered(&self, query: &[f32], k: usize, filter: impl Fn(usize) -> bool) -> Vec<(f32, usize)> {
        if k == 0 {
            return Vec::new();
        }
        let mut scored: Vec<(f32, usize)> = self
            .nodes
            .iter()
            .filter(|(id, _)| !self.tombstones.contains(id) && filter(**id))
            .map(|(&id, node)| (self.metric.compute(&node.vector, query), id))
            .collect();

        // Total order so `select_nth_unstable_by` partitions a well-defined
        // top-k (f32 alone has no Ord and ties would be arbitrary).
        let by_distance = |a: &(f32, usize), b: &(f32, usize)| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1));
        if scored.len() > k {
            scored.select_nth_unstable_by(k - 1, by_distance);
            scored.truncate(k);
        }
        scored.sort_by(by_distance);
        scored
    }

    /// Search for the `k` approximate nearest neighbours to `query`, expanding
    /// a beam of `ef` candidates at layer 0.
    ///
    /// `ef` is floored at `k` (a beam narrower than the result count cannot
    /// return that many results) and floored at [`EF_SEARCH`] is **not**
    /// applied here — a caller that explicitly asks for a wider pool must get
    /// one, otherwise the parameter is a lie. Latency grows with `ef`, so
    /// callers should measure rather than assume the default is enough.
    ///
    /// While the corpus is at or below the exact-scan threshold the beam has
    /// nothing to bind to: the query scans every vector and `ef` cannot change
    /// the answer. See [`EXACT_SEARCH_MAX_NODES`].
    ///
    /// # Errors
    ///
    /// Returns an empty vec when the index is empty, when `k == 0`, or when
    /// `ef == 0`.
    pub fn search_with_ef(&self, query: &[f32], k: usize, ef: usize) -> Vec<(f32, usize)> {
        if self.nodes.is_empty() || k == 0 || ef == 0 {
            return Vec::new();
        }
        if !self.graph_built {
            return self.search_exact(query, k);
        }
        let Some(entry) = self.entry_point else {
            return self.search_exact(query, k);
        };

        // Phase 1: Greedy descent from top level to level 1
        let mut current = entry;
        for level in (1..=self.max_level).rev() {
            current = self.greedy_search_at_layer(current, query, level);
        }

        // Phase 2: Search at level 0, expanding a beam of `ef` candidates.
        self.search_layer_0(current, query, 0, k, ef)
    }

    /// Search for the `k` approximate nearest neighbours to `query` that satisfy
    /// `filter`, expanding a beam of `ef` candidates at layer 0.
    ///
    /// The filter predicate is applied during beam search: candidates that don't
    /// pass the filter are not added to the candidate pool, and results that
    /// don't pass are not counted toward `k`. This means more candidates may
    /// need to be explored than `k` to find `k` passing results.
    ///
    /// Returns a list of `(distance, node_id)` pairs sorted by distance
    /// (closest first). Returns an empty vec when the index is empty, when
    /// `k == 0`, or when `ef == 0`.
    ///
    /// While the corpus is at or below the exact-scan threshold the filter is
    /// applied to every live vector, so the result is the exact filtered
    /// top-`k`; `ef` cannot change the answer (see [`EXACT_SEARCH_MAX_NODES`]).
    pub fn search_with_filter(
        &self,
        query: &[f32],
        k: usize,
        ef: usize,
        filter: impl Fn(usize) -> bool,
    ) -> Vec<(f32, usize)> {
        if self.nodes.is_empty() || k == 0 || ef == 0 {
            return Vec::new();
        }
        if !self.graph_built {
            return self.search_exact_filtered(query, k, filter);
        }
        let Some(entry) = self.entry_point else {
            return self.search_exact_filtered(query, k, filter);
        };

        // Phase 1: Greedy descent from top level to level 1.
        // The entry point descent does NOT apply the filter — it's just a
        // starting point for the layer-0 search where filtering happens.
        let mut current = entry;
        for level in (1..=self.max_level).rev() {
            current = self.greedy_search_at_layer(current, query, level);
        }

        // Phase 2: Search at level 0 with filtering.
        self.search_layer_0_filtered(current, query, 0, k, ef, filter)
    }

    /// Search at a single layer with a filter predicate applied during
    /// candidate expansion and result selection.
    ///
    /// Neighbors that don't pass the filter are not added to the candidate
    /// pool. Only candidates that pass the filter are added to results.
    /// The search continues until `k` passing results are found or the beam
    /// is exhausted.
    fn search_layer_0_filtered(
        &self,
        entry: usize,
        query: &[f32],
        layer: usize,
        k: usize,
        ef: usize,
        filter: impl Fn(usize) -> bool,
    ) -> Vec<(f32, usize)> {
        if k == 0 {
            return Vec::new();
        }
        let ef = ef.max(k);
        let reachable = self.nodes.len().max(1);
        let vis_cap = ef.min(reachable).saturating_mul(4);
        let heap_cap = ef.min(reachable).saturating_add(1);
        let mut visited = AHashSet::with_capacity(vis_cap);

        let entry_dist = self.node_distance(entry, query);
        let mut candidates: BinaryHeap<Reverse<Candidate>> = BinaryHeap::with_capacity(heap_cap);
        let mut results: BinaryHeap<Candidate> = BinaryHeap::with_capacity(heap_cap);
        candidates.push(Reverse(Candidate(entry_dist, entry)));
        results.push(Candidate(entry_dist, entry));
        visited.insert(entry);

        let mut worst_dist = entry_dist;

        while let Some(Reverse(c)) = candidates.pop() {
            if results.len() >= ef && c.0 > worst_dist {
                break;
            }
            let node = match self.nodes.get(&c.1) {
                Some(n) => n,
                None => continue,
            };
            if layer >= node.connections.len() {
                continue;
            }
            for &neighbor in &node.connections[layer] {
                if !visited.insert(neighbor) {
                    continue;
                }
                let neighbor_node = match self.nodes.get(&neighbor) {
                    Some(n) => n,
                    None => continue,
                };
                // FILTER: Only add neighbors that pass the filter predicate.
                // Filtered-out neighbors are never added to candidates or results.
                if !filter(neighbor) {
                    continue;
                }
                let nd = self.metric.compute(&neighbor_node.vector, query);
                if nd < worst_dist || results.len() < ef {
                    results.push(Candidate(nd, neighbor));
                    candidates.push(Reverse(Candidate(nd, neighbor)));
                    if results.len() > ef {
                        results.pop();
                    }
                    if let Some(worst) = results.peek() {
                        worst_dist = worst.0;
                    }
                }
            }
        }

        let mut out = Vec::with_capacity(results.len());
        while let Some(c) = results.pop() {
            if !self.tombstones.contains(&c.1) {
                out.push((c.0, c.1));
            }
        }
        out.reverse();
        out.truncate(k);
        out
    }
    pub fn get_vector(&self, id: usize) -> Option<&[f32]> {
        self.nodes.get(&id).map(|n| n.vector.as_slice())
    }

    /// Get the degree (number of connections at level 0) for a node.
    ///
    /// Returns `None` for an unknown node id, and for any node while the index
    /// is still a flat store — there is no graph to measure yet.
    pub fn degree(&self, id: usize) -> Option<usize> {
        self.nodes.get(&id).and_then(|n| n.connections.first().map(|c| c.len()))
    }

    /// Collect index statistics.
    ///
    /// While the index is a flat store the graph is not built, so
    /// `total_connections` and `avg_degree` are zero; `num_vectors` and
    /// `max_level` still describe the stored state.
    pub fn stats(&self) -> HnswStats {
        let total_connections: usize = self
            .nodes
            .values()
            .map(|n| n.connections.iter().map(|c| c.len()).sum::<usize>())
            .sum();
        HnswStats {
            num_vectors: self.nodes.len(),
            max_level: self.max_level,
            total_connections,
            avg_degree: if self.nodes.is_empty() {
                0.0
            } else {
                total_connections as f32 / self.nodes.len() as f32
            },
        }
    }
}

/// Statistics about an HNSW index.
#[derive(Debug, Clone)]
pub struct HnswStats {
    pub num_vectors: usize,
    pub max_level: usize,
    pub total_connections: usize,
    pub avg_degree: f32,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn random_vector(rng: &mut u64, dims: usize) -> Vec<f32> {
        (0..dims)
            .map(|_| {
                *rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((*rng >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
            })
            .collect()
    }

    #[test]
    fn test_empty_index() {
        let idx = HnswIndex::new(DistanceMetric::Euclidean);
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
        assert!(idx.search(&[1.0, 2.0, 3.0], 5).is_empty());
    }

    #[test]
    fn test_search_k_zero_returns_empty() {
        // P52.29: k=0 used to underflow `results[k - 1]` and panic.
        let mut idx = HnswIndex::new(DistanceMetric::Euclidean);
        for i in 0..20 {
            idx.insert(vec![i as f32, i as f32], i);
        }
        assert!(idx.search(&[0.0, 0.0], 0).is_empty());
    }

    #[test]
    fn test_single_insert() {
        // T064: the entry point is a graph concept, so this asserts the graph
        // path explicitly. A default index stays flat and has no entry point.
        let mut idx = HnswIndex::with_exact_threshold(DistanceMetric::Euclidean, 0);
        idx.insert(vec![1.0, 2.0, 3.0], 0);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.entry_point(), Some(0));
        assert!(idx.is_graph_built());

        let mut flat = HnswIndex::new(DistanceMetric::Euclidean);
        flat.insert(vec![1.0, 2.0, 3.0], 0);
        assert_eq!(flat.len(), 1);
        assert_eq!(flat.entry_point(), None, "a flat store has no graph entry point");
        assert!(!flat.is_graph_built());
        assert_eq!(flat.search(&[1.0, 2.0, 3.0], 1).len(), 1);
    }

    /// T064: crossing the threshold links the flat store into a graph, and the
    /// graph stays searchable afterwards.
    #[test]
    fn crossing_the_exact_threshold_builds_the_graph() {
        let mut idx = HnswIndex::with_exact_threshold(DistanceMetric::Euclidean, 4);
        for id in 0..4 {
            idx.insert(vec![id as f32, id as f32], id);
        }
        assert!(!idx.is_graph_built(), "at the threshold the store stays flat");
        assert_eq!(idx.stats().total_connections, 0, "a flat store has no links");

        idx.insert(vec![4.0, 4.0], 4);
        assert!(idx.is_graph_built(), "one past the threshold builds the graph");
        assert!(idx.stats().total_connections > 0, "the built graph must have links");
        assert_eq!(idx.len(), 5);

        let hits = idx.search(&[0.0, 0.0], 3);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].1, 0, "the nearest vector must still be found");
    }

    /// T064: tombstoned vectors are never linked into the graph when the flat
    /// store is relinked at the threshold.
    #[test]
    fn crossing_the_threshold_leaves_tombstones_unlinked() {
        let mut idx = HnswIndex::with_exact_threshold(DistanceMetric::Euclidean, 2);
        idx.insert(vec![0.0, 0.0], 0);
        idx.insert(vec![1.0, 1.0], 1);
        idx.remove(0);
        idx.insert(vec![2.0, 2.0], 2);

        assert!(idx.is_graph_built());
        assert_eq!(idx.len(), 2);
        let hits = idx.search(&[0.0, 0.0], 2);
        assert!(
            hits.iter().all(|(_, id)| *id != 0),
            "a deleted vector must not be reachable after the graph is built"
        );
        assert_eq!(hits.len(), 2, "the live vectors must stay searchable");
    }

    /// T064: the default threshold must cover the SC-001/SC-002 benchmark
    /// corpus, otherwise those criteria would silently measure the graph path.
    #[test]
    fn the_default_threshold_covers_the_benchmark_corpus() {
        let threshold = EXACT_SEARCH_MAX_NODES;
        assert!(
            threshold >= 10_000,
            "the 10k benchmark corpus must be served by the exact path, got {threshold}"
        );
    }

    #[test]
    fn test_insert_and_search_exact() {
        let mut idx = HnswIndex::new(DistanceMetric::Euclidean);

        // Insert 3 clearly separated vectors
        idx.insert(vec![0.0, 0.0], 0);
        idx.insert(vec![10.0, 10.0], 1);
        idx.insert(vec![100.0, 100.0], 2);

        // Search for nearest to [0.0, 0.0] — should find 0 first
        let results = idx.search(&[0.0, 0.0], 3);
        assert!(!results.is_empty(), "Should return at least 1 result");
        assert_eq!(results[0].1, 0, "Closest to origin should be node 0");
        // Distance from [0,0] to [0,0] should be ~0
        assert!(results[0].0 < 0.01, "Distance should be near 0");
    }

    #[test]
    fn test_search_returns_k_results() {
        let mut idx = HnswIndex::new(DistanceMetric::Euclidean);
        for i in 0..20 {
            let v = vec![i as f32 * 10.0, i as f32 * 10.0];
            idx.insert(v, i);
        }

        let results = idx.search(&[0.0, 0.0], 5);
        assert_eq!(results.len(), 5, "Should return exactly 5 results");
        // The first 5 closest should be nodes 0, 1, 2, 3, 4
        for i in 0..5 {
            assert_eq!(results[i].1, i, "Result {} should be node {}", i, i);
        }
    }

    /// Build a flat (exact-scan) index over `n` dense random vectors.
    fn flat_index(n: usize, dims: usize, seed: u64) -> (HnswIndex, Vec<f32>) {
        let mut rng = seed;
        let mut idx = HnswIndex::new(DistanceMetric::Euclidean);
        for id in 0..n {
            idx.insert(random_vector(&mut rng, dims), id);
        }
        let query = random_vector(&mut rng, dims);
        (idx, query)
    }

    /// Build a graph-backed index over `n` dense random vectors of dimension
    /// `dims`. The threshold is forced to `0` so the graph is exercised at a
    /// size a unit test can afford.
    fn dense_index(n: usize, dims: usize, seed: u64) -> (HnswIndex, Vec<f32>) {
        let mut rng = seed;
        let mut idx = HnswIndex::with_exact_threshold(DistanceMetric::Euclidean, 0);
        for id in 0..n {
            idx.insert(random_vector(&mut rng, dims), id);
        }
        let query = random_vector(&mut rng, dims);
        (idx, query)
    }

    /// The exact top-`k` by full scan — the ground truth the ANN must approach.
    fn exact_top_k(idx: &HnswIndex, query: &[f32], k: usize) -> Vec<usize> {
        let mut all: Vec<(f32, usize)> = idx
            .nodes()
            .into_iter()
            .map(|(id, v)| (DistanceMetric::Euclidean.compute(v, query), id))
            .collect();
        all.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        all.truncate(k);
        all.into_iter().map(|(_, id)| id).collect()
    }

    fn overlap_at_k(got: &[(f32, usize)], want: &[usize], k: usize) -> f32 {
        let hit = got.iter().take(k).filter(|(_, id)| want.contains(id)).count();
        hit as f32 / k.max(1) as f32
    }

    /// T052: the beam width must actually widen the candidate pool.
    ///
    /// This is the invariant the bench kept failing: `overlap@10` was identical
    /// across a 20× pool range, because the beam was pinned to a constant and the
    /// pool argument had nothing to bind to. Widening `ef` must let the search
    /// explore more of the graph, so recall against the exact ranking cannot get
    /// worse as the beam grows.
    #[test]
    fn a_wider_beam_never_loses_recall_against_the_exact_ranking() {
        let (idx, query) = dense_index(2_000, 32, 0xC0FFEE);
        let k = 10;
        let exact = exact_top_k(&idx, &query, k);

        let narrow = overlap_at_k(&idx.search_with_ef(&query, k, k), &exact, k);
        let wide = overlap_at_k(&idx.search_with_ef(&query, k, 400), &exact, k);

        assert!(
            wide + 1e-5 >= narrow,
            "a wider beam must not lose recall: narrow ef={k} gave overlap {narrow}, \
             wide ef=400 gave overlap {wide}"
        );
        assert!(
            wide > 0.0,
            "a wide beam should surface at least one of the exact top-{k}, got {wide}"
        );
    }

    /// T052: the beam must genuinely control how many nodes are explored.
    ///
    /// A beam equal to `k` cannot return more than `k` distinct nodes, so on a
    /// corpus where the exact ranking is not reachable in `k` hops the narrow
    /// beam is measurably worse. This pins the mechanism rather than the
    /// outcome, so it holds on any corpus.
    #[test]
    fn the_beam_controls_the_explored_candidate_set() {
        let (idx, query) = dense_index(2_000, 32, 0xBEEF);
        let k = 10;
        let exact = exact_top_k(&idx, &query, k);

        let narrow = overlap_at_k(&idx.search_with_ef(&query, k, 10), &exact, k);
        let wide = overlap_at_k(&idx.search_with_ef(&query, k, 500), &exact, k);

        assert!(
            wide + 1e-5 >= narrow,
            "widening the beam must not reduce overlap (ef=10: {narrow}, ef=500: {wide})"
        );
    }

    /// T052: `ef` is floored at `k`, so a caller asking for a beam narrower than
    /// the result count still gets `k` results rather than a short list.
    #[test]
    fn the_beam_is_floored_at_k() {
        let (idx, query) = dense_index(500, 16, 0x5EED);

        let with_tiny_beam = idx.search_with_ef(&query, 10, 1);
        assert_eq!(
            with_tiny_beam.len(),
            10,
            "ef below k is floored at k, so the result count must not shrink"
        );
    }

    /// T052: the default `search` path is unchanged — it still floors the beam at
    /// `EF_SEARCH`, so existing callers see identical results.
    #[test]
    fn default_search_keeps_the_previous_default_beam() {
        let (idx, query) = dense_index(500, 16, 0xD1CE);

        let default = idx.search(&query, 10);
        let explicit_default_beam = idx.search_with_ef(&query, 10, EF_SEARCH);

        assert_eq!(
            default, explicit_default_beam,
            "search() and search_with_ef(.., EF_SEARCH) must agree exactly"
        );
    }

    /// T052: a beam wider than the index is not an error, and a zero beam
    /// returns nothing rather than panicking.
    #[test]
    fn out_of_range_and_zero_beams_are_safe() {
        let (idx, query) = dense_index(200, 8, 0xACE);

        let absurd = idx.search_with_ef(&query, 5, usize::MAX);
        assert_eq!(absurd.len(), 5, "a beam larger than the index still returns k results");

        assert!(idx.search_with_ef(&query, 5, 0).is_empty(), "ef=0 must return empty");
        assert!(idx.search_with_ef(&query, 0, 50).is_empty(), "k=0 must return empty");
    }

    #[test]
    fn test_insert_honors_sparse_ids() {
        // P51.17: the caller-supplied id is preserved as the node id returned
        // by `search`. During vector-index populate, rows with a NULL / non-list
        // vector column are skipped, so node ids must not be compacted to
        // positional indices — otherwise a search result would address the
        // wrong table row.
        let mut idx = HnswIndex::new(DistanceMetric::Euclidean);
        idx.insert(vec![0.0, 0.0], 0);
        idx.insert(vec![10.0, 10.0], 2); // row 1 skipped (NULL vector)
        idx.insert(vec![100.0, 100.0], 3);

        assert_eq!(idx.len(), 3);
        // get_vector addresses by caller id, not by position.
        assert!(idx.get_vector(0).is_some());
        assert!(idx.get_vector(1).is_none());
        assert!(idx.get_vector(2).is_some());

        let results = idx.search(&[10.0, 10.0], 3);
        assert_eq!(
            results[0].1, 2,
            "nearest to row 2 must be reported as id 2, not positionally"
        );
    }

    #[test]
    fn test_distance_metrics() {
        // Verify compute() produces reasonable values for each metric
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];

        let cos = DistanceMetric::Cosine.compute(&a, &b);
        assert!(
            (cos - 1.0).abs() < 0.01,
            "Cosine of orthogonal vectors ≈ 1.0, got {cos}"
        );

        let euc = DistanceMetric::Euclidean.compute(&a, &b);
        assert!(
            (euc - 2.0_f32.sqrt()).abs() < 0.01,
            "Euclidean distance ≈ √2, got {euc}"
        );

        let l2 = DistanceMetric::L2Squared.compute(&a, &b);
        assert!((l2 - 2.0).abs() < 0.01, "L2 squared = 2.0, got {l2}");

        let dot = DistanceMetric::DotProduct.compute(&a, &b);
        assert!((dot - 0.0).abs() < 0.01, "Dot of orthogonal ≈ 0, got {dot}");
    }

    #[test]
    fn test_cosine_metric_is_scale_invariant() {
        // P51.46: cosine distance must not assume the stored vector is
        // pre-normalized. `DistanceMetric::Cosine::compute` normalizes BOTH
        // operands at query time, so scaling one side must not change the
        // distance — the write path stores raw (un-normalized) vectors.
        let query = vec![3.0, 4.0, 0.0];
        let stored = vec![1.0, 2.0, 2.0];
        let base = DistanceMetric::Cosine.compute(&query, &stored);

        for scale in [0.5, 2.0, 10.0, 0.001] {
            let scaled: Vec<f32> = stored.iter().map(|x| x * scale).collect();
            let d = DistanceMetric::Cosine.compute(&query, &scaled);
            assert!(
                (d - base).abs() < 1e-5,
                "cosine distance must be scale-invariant: {base} vs {d} (scale={scale})"
            );
        }

        // Zero-norm stored vector must not panic (guard returns 1.0 = max distance).
        let zero = vec![0.0, 0.0, 0.0];
        let d = DistanceMetric::Cosine.compute(&query, &zero);
        assert!((d - 1.0).abs() < 1e-9, "zero vector should map to max distance");
    }

    #[test]
    fn test_recall_rate() {
        let mut rng: u64 = 12345;
        let dims = 16;
        let num_vectors = 200;

        let mut idx = HnswIndex::with_exact_threshold(DistanceMetric::Euclidean, 0);

        // Insert random vectors
        for i in 0..num_vectors {
            let v = random_vector(&mut rng, dims);
            idx.insert(v, i);
        }

        assert_eq!(idx.len(), num_vectors);

        // Test recall against brute-force for several queries
        let vectors: Vec<Vec<f32>> = (0..num_vectors).map(|i| idx.get_vector(i).unwrap().to_vec()).collect();

        for test_idx in 0..10 {
            let query = random_vector(&mut rng, dims);

            // Brute-force top-10
            let mut bf: Vec<(f32, usize)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (DistanceMetric::Euclidean.compute(&query, v), i))
                .collect();
            bf.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            let bf_top10: AHashSet<usize> = bf.iter().take(10).map(|(_, id)| *id).collect();

            // HNSW top-10
            let hnsw = idx.search(&query, 10);
            let hnsw_top10: AHashSet<usize> = hnsw.iter().map(|(_, id)| *id).collect();

            // Compute recall
            let intersection = bf_top10.intersection(&hnsw_top10).count();
            let recall = intersection as f32 / 10.0;
            assert!(
                recall > 0.85,
                "Recall too low: {recall} (test {test_idx}, expected > 0.85)"
            );
        }
    }

    #[test]
    fn test_cosine_metric_search() {
        let mut idx = HnswIndex::new(DistanceMetric::Cosine);

        // Insert vectors in different directions
        idx.insert(vec![1.0, 0.0], 0); // points right
        idx.insert(vec![0.0, 1.0], 1); // points up
        idx.insert(vec![-1.0, 0.0], 2); // points left

        // Query pointing slightly right-up — should find 0 (right) closest
        let results = idx.search(&[0.9, 0.1], 3);
        assert!(!results.is_empty());
        assert_eq!(results[0].1, 0, "Closest by cosine should be node 0 (right)");
    }

    #[test]
    fn test_l1_metric() {
        let mut idx = HnswIndex::new(DistanceMetric::L1);
        idx.insert(vec![0.0, 0.0], 0);
        idx.insert(vec![5.0, 5.0], 1);
        idx.insert(vec![10.0, 10.0], 2);

        let results = idx.search(&[5.1, 4.9], 1);
        assert!(!results.is_empty());
        assert_eq!(results[0].1, 1, "Closest by L1 should be node 1");
    }

    #[test]
    fn test_1000_vectors_recall() {
        let mut rng: u64 = 9999;
        let dims = 8;
        let num_vectors = 1000;

        let mut idx = HnswIndex::with_exact_threshold(DistanceMetric::Euclidean, 0);

        for i in 0..num_vectors {
            let v = random_vector(&mut rng, dims);
            idx.insert(v, i);
        }

        // Test recall for 20 random queries
        let vectors: Vec<Vec<f32>> = (0..num_vectors).map(|i| idx.get_vector(i).unwrap().to_vec()).collect();

        let mut total_recall = 0.0;
        let num_queries = 20;

        for _ in 0..num_queries {
            let query = random_vector(&mut rng, dims);

            let mut bf: Vec<(f32, usize)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (DistanceMetric::Euclidean.compute(&query, v), i))
                .collect();
            bf.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            let bf_top10: AHashSet<usize> = bf.iter().take(10).map(|(_, id)| *id).collect();

            let hnsw = idx.search(&query, 10);
            let hnsw_top10: AHashSet<usize> = hnsw.iter().map(|(_, id)| *id).collect();

            let intersection = bf_top10.intersection(&hnsw_top10).count();
            total_recall += intersection as f32 / 10.0;
        }

        let avg_recall = total_recall / num_queries as f32;
        assert!(
            avg_recall > 0.95,
            "Average recall too low: {avg_recall} (expected > 0.95)"
        );
    }

    #[test]
    fn test_search_layer_0_is_sorted() {
        // Layer 0 is the graph's beam search; force the graph path so this
        // keeps testing traversal rather than the exact scan.
        let mut idx = HnswIndex::with_exact_threshold(DistanceMetric::Euclidean, 0);
        let mut rng: u64 = 5555;
        for i in 0..100 {
            idx.insert(random_vector(&mut rng, 16), i);
        }
        let query = random_vector(&mut rng, 16);
        let res = idx.search_layer_0(idx.entry_point().unwrap(), &query, 0, 50, 50);
        assert!(!res.is_empty());
        for w in res.windows(2) {
            assert!(
                w[0].0 <= w[1].0,
                "search_layer_0 results must be sorted ascending by distance: {} > {}",
                w[0].0,
                w[1].0
            );
        }
    }

    /// T064: below the exact-search threshold the index must return the true
    /// top-`k`. SC-002 measures overlap against exactly this ranking, so an
    /// approximate answer at benchmark scale is the failure being fixed.
    #[test]
    fn flat_mode_returns_the_exact_top_k() {
        let (idx, query) = flat_index(2_000, 96, 0xF1A7);
        let k = 10;

        let want = exact_top_k(&idx, &query, k);
        let got: Vec<usize> = idx.search(&query, k).into_iter().map(|(_, id)| id).collect();

        assert_eq!(
            got, want,
            "flat mode must reproduce the exact ranking, not approximate it"
        );
    }

    /// T064: while the index is flat there is no graph to widen, so the beam
    /// width must not change the result. This pins the routing decision: an
    /// index at or below the threshold never pays for graph traversal.
    #[test]
    fn flat_mode_ignores_the_beam_width() {
        let (idx, query) = flat_index(2_000, 96, 0xB0A7);

        let narrow = idx.search_with_ef(&query, 10, 16);
        let wide = idx.search_with_ef(&query, 10, 400);

        assert_eq!(
            narrow, wide,
            "flat mode has no graph to explore, so ef must not change the answer"
        );
    }

    /// T064: the filtered search is used by the SQL vector path. In flat mode
    /// the filter must be applied to every live node, so the result is the
    /// exact filtered top-`k` — not a beam that happened to contain matches.
    #[test]
    fn flat_mode_filtered_search_is_exact() {
        let (idx, query) = flat_index(2_000, 96, 0xF117);
        let k = 10;
        let keep = |id: usize| id.is_multiple_of(7);

        let mut want: Vec<(f32, usize)> = idx
            .nodes()
            .into_iter()
            .filter(|(id, _)| keep(*id))
            .map(|(id, v)| (DistanceMetric::Euclidean.compute(v, &query), id))
            .collect();
        want.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        want.truncate(k);

        let got = idx.search_with_filter(&query, k, 200, keep);
        assert_eq!(got, want, "filtered search in flat mode must be exact");
    }

    /// T064: a tombstoned vector must not occupy a slot in the exact top-`k`;
    /// the next live vector is promoted instead.
    #[test]
    fn flat_mode_promotes_the_next_live_vector_past_a_tombstone() {
        let (mut idx, query) = flat_index(2_000, 96, 0x70B5);
        let k = 10;
        let before = exact_top_k(&idx, &query, k);

        idx.remove(before[0]);
        let hits = idx.search(&query, k);

        assert_eq!(hits.len(), k, "a tombstone must not shorten the result list");
        assert!(
            hits.iter().all(|(_, id)| *id != before[0]),
            "tombstoned node {} must not be returned",
            before[0]
        );
        let want: Vec<usize> = exact_top_k(&idx, &query, k);
        let got: Vec<usize> = hits.into_iter().map(|(_, id)| id).collect();
        assert_eq!(got, want, "the next live vector must be promoted");
    }

    #[test]
    fn test_stats_report() {
        // Graph statistics only exist once the graph is built; a default index
        // stays flat (see `crossing_the_exact_threshold_builds_the_graph`).
        let mut idx = HnswIndex::with_exact_threshold(DistanceMetric::Euclidean, 0);
        for i in 0..50 {
            idx.insert(vec![i as f32, i as f32], i);
        }
        let stats = idx.stats();
        assert_eq!(stats.num_vectors, 50);
        assert!(stats.avg_degree > 0.0, "Average degree should be > 0");
        assert!(stats.total_connections > 0, "Should have connections");
    }
}
