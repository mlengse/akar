//! HNSW (Hierarchical Navigable Small World) index for approximate nearest
//! neighbour search.
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

use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Constants (HNSW defaults matching the reference implementation)
// ---------------------------------------------------------------------------

/// Maximum number of connections per element per layer for construction.
const M: usize = 16;

/// Maximum number of connections per element per layer for construction
/// (used only at layer 0).
const M_MAX: usize = 32;

/// Maximum number of candidates to keep during the search for neighbours.
const EF_CONSTRUCTION: usize = 200;

/// Number of iterations for the multi-element pruning procedure.
const MAX_M: usize = M_MAX;

/// Size of the dynamic candidate list during search.
const EF_SEARCH: usize = 50;

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
    pub fn compute(&self, a: &[f64], b: &[f64]) -> f64 {
        match self {
            DistanceMetric::Cosine => {
                let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
                let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
                let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
                if norm_a == 0.0 || norm_b == 0.0 {
                    1.0
                } else {
                    1.0 - dot / (norm_a * norm_b)
                }
            }
            DistanceMetric::Euclidean => {
                a.iter()
                    .zip(b.iter())
                    .map(|(x, y)| (x - y) * (x - y))
                    .sum::<f64>()
                    .sqrt()
            }
            DistanceMetric::L1 => a.iter().zip(b.iter()).map(|(x, y)| (x - y).abs()).sum(),
            DistanceMetric::L2Squared => {
                a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum()
            }
            DistanceMetric::DotProduct => {
                // Dot product: higher = more similar → negate for smaller-distance convention
                -a.iter().zip(b.iter()).map(|(x, y)| x * y).sum::<f64>()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Core data structures
// ---------------------------------------------------------------------------

/// A node in the HNSW graph representing one vector.
#[derive(Debug, Clone)]
struct HnswNode {
    /// The vector data.
    vector: Vec<f64>,
    /// Connections per layer: `connections[level]` = list of node IDs.
    connections: Vec<Vec<usize>>,
}

/// The HNSW index.
///
/// Stores vectors in a multi-layer navigable small world graph for fast
/// approximate nearest neighbour search.
pub struct HnswIndex {
    /// All nodes in the index.
    nodes: Vec<HnswNode>,
    /// Number of layers in the graph.
    max_level: usize,
    /// Entry point — the node ID at the highest layer.
    entry_point: Option<usize>,
    /// Distance metric to use.
    metric: DistanceMetric,
    /// Random state for level generation.
    rng_state: u64,
}

impl HnswIndex {
    /// Create a new HNSW index with the given distance metric.
    pub fn new(metric: DistanceMetric) -> Self {
        Self {
            nodes: Vec::new(),
            max_level: 0,
            entry_point: None,
            metric,
            rng_state: 42,
        }
    }

    /// Number of vectors in the index.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Get the current maximum level of the graph.
    pub fn max_level(&self) -> usize {
        self.max_level
    }

    /// Get the entry point node ID (if any).
    pub fn entry_point(&self) -> Option<usize> {
        self.entry_point
    }

    /// Get a reference to the vectors stored in the index.
    pub fn vectors(&self) -> Vec<&[f64]> {
        self.nodes.iter().map(|n| n.vector.as_slice()).collect()
    }

    // ---- Internal helpers ----

    /// Generate a random level for a new node using the HNSW level
    /// distribution: `floor(-ln(uniform(0,1)) * ML)`.
    fn random_level(&mut self) -> usize {
        // Simple LCG random number generator
        self.rng_state = self.rng_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let r = (self.rng_state >> 33) as f64 / (1u64 << 31) as f64;
        let level = (-r.ln() * ml()).floor() as usize;
        level.min(MAX_M)
    }

    /// Compute distance between the vector at `node_id` and a query vector.
    fn node_distance(&self, node_id: usize, query: &[f64]) -> f64 {
        self.metric.compute(&self.nodes[node_id].vector, query)
    }

    /// Greedy search from a given entry point to find the closest node
    /// to the query at the specified layer. Returns the closest node ID.
    fn greedy_search_at_layer(&self, entry: usize, query: &[f64], layer: usize) -> usize {
        let mut current = entry;
        let mut current_dist = self.node_distance(current, query);

        loop {
            let mut improved = false;
            let neighbors = &self.nodes[current].connections;
            if layer < neighbors.len() {
                for &neighbor in &neighbors[layer] {
                    let d = self.node_distance(neighbor, query);
                    if d < current_dist {
                        current_dist = d;
                        current = neighbor;
                        improved = true;
                    }
                }
            }
            if !improved {
                break;
            }
        }

        current
    }

    /// Search layer 0 for the `k` nearest neighbours, starting from a given
    /// entry point. Uses a simple greedy descent with a candidate set.
    fn search_layer_0(&self, entry: usize, query: &[f64], k: usize) -> Vec<(f64, usize)> {
        let mut candidates = Vec::new();
        let mut visited = HashSet::new();

        let entry_dist = self.node_distance(entry, query);
        candidates.push((entry_dist, entry));
        visited.insert(entry);

        // We'll use a simple beam search: keep at most `ef` candidates
        let ef = EF_SEARCH.max(k);
        let mut results: Vec<(f64, usize)> = vec![(entry_dist, entry)];

        let mut idx = 0;
        while idx < candidates.len() {
            let (dist, node_id) = candidates[idx];
            idx += 1;

            // Prune: if this candidate is worse than the k-th best so far,
            // and we already have k results, skip expansion
            if results.len() >= k && dist > results[k - 1].0 {
                continue;
            }

            let neighbors = &self.nodes[node_id].connections;
            if neighbors.is_empty() {
                continue;
            }
            for &neighbor in &neighbors[0] {
                if visited.contains(&neighbor) {
                    continue;
                }
                visited.insert(neighbor);
                let nd = self.node_distance(neighbor, query);

                // Add to candidates
                candidates.push((nd, neighbor));
                results.push((nd, neighbor));

                // Sort results by distance ascending, keep at most `ef`
                results.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                if results.len() > ef {
                    results.truncate(ef);
                }
            }
        }

        results.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        results.truncate(k);
        results
    }

    /// Select the `M` closest neighbours from a candidate list.
    fn select_neighbors_simple(&self, candidates: &[(f64, usize)], m: usize) -> Vec<usize> {
        let mut sorted: Vec<_> = candidates.to_vec();
        sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        sorted.truncate(m);
        sorted.into_iter().map(|(_, id)| id).collect()
    }

    // ---- Public API ----

    /// Insert a vector into the index with the given ID.
    ///
    /// The ID should uniquely identify this vector within the index.
    pub fn insert(&mut self, vector: Vec<f64>, _id: usize) {
        let new_id = self.nodes.len();
        let new_level = if self.nodes.is_empty() {
            MAX_M // First node goes to max level
        } else {
            self.random_level()
        };

        // Build connection storage for all levels this node will occupy
        let mut connections = vec![Vec::new(); new_level + 1];

        if self.entry_point.is_none() {
            // First node: just add it
            self.nodes.push(HnswNode {
                vector,
                connections,
            });
            self.entry_point = Some(0);
            self.max_level = new_level;
            return;
        }

        let entry = self.entry_point.unwrap();

        // Phase 1: Traverse from top level down to `new_level + 1`
        // to find the best entry point at `new_level`.
        let mut current_entry = entry;
        for level in (new_level + 1..=self.max_level).rev() {
            current_entry = self.greedy_search_at_layer(current_entry, &vector, level);
        }

        // Phase 2: Insert at each level from `min(new_level, max_level)` down to 0
        for level in (0..=new_level.min(self.max_level)).rev() {
            // Find nearest neighbours at this layer
            let ep = self.greedy_search_at_layer(current_entry, &vector, level);
            let candidates = self.search_layer_0(ep, &vector, EF_CONSTRUCTION);

            let m = if level == 0 { M_MAX } else { M };
            let neighbors = self.select_neighbors_simple(&candidates, m);

            // Add connections from new node to neighbours
            connections[level] = neighbors.clone();

            // Add connections from neighbours back to new node
            for &neighbor_id in &neighbors {
                // First, pre-compute distances for ALL existing connections of
                // this neighbor at this level, plus the new connection to new_id.
                // We do this BEFORE taking any mutable borrow.
                let existing_connections: Vec<usize> = if neighbor_id < self.nodes.len()
                    && level < self.nodes[neighbor_id].connections.len()
                {
                    self.nodes[neighbor_id].connections[level].clone()
                } else {
                    Vec::new()
                };

                // Compute distances: existing connections + new_id
                // Use the self.nodes entries for existing connections, and the
                // local `vector` variable for the not-yet-inserted new node.
                let mut dists: Vec<(f64, usize)> = existing_connections
                    .iter()
                    .map(|&nid| {
                        let d = self.node_distance(nid, &self.nodes[neighbor_id].vector);
                        (d, nid)
                    })
                    .collect();
                // Add the new connection — compute distance using the local
                // `vector` (new_id node hasn't been added to self.nodes yet).
                {
                    let d = self.metric.compute(&vector, &self.nodes[neighbor_id].vector);
                    dists.push((d, new_id));
                }

                let max_conn = if level == 0 { M_MAX } else { M };
                dists.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                dists.truncate(max_conn);

                // Now take the mutable borrow and write back
                while self.nodes[neighbor_id].connections.len() <= level {
                    self.nodes[neighbor_id].connections.push(Vec::new());
                }
                self.nodes[neighbor_id].connections[level] = dists.into_iter().map(|(_, id)| id).collect();
            }

            current_entry = ep;
        }

        // Update entry point if the new node has a higher level
        if new_level > self.max_level {
            self.max_level = new_level;
            self.entry_point = Some(new_id);
        }

        // Add the new node
        self.nodes.push(HnswNode {
            vector,
            connections,
        });
    }

    /// Search for the `k` approximate nearest neighbours to `query`.
    ///
    /// Returns a list of `(distance, node_id)` pairs sorted by distance
    /// (closest first).
    pub fn search(&self, query: &[f64], k: usize) -> Vec<(f64, usize)> {
        if self.nodes.is_empty() || self.entry_point.is_none() {
            return Vec::new();
        }

        let entry = self.entry_point.unwrap();

        // Phase 1: Greedy descent from top level to level 1
        let mut current = entry;
        for level in (1..=self.max_level).rev() {
            current = self.greedy_search_at_layer(current, query, level);
        }

        // Phase 2: Search at level 0
        self.search_layer_0(current, query, k)
    }

    /// Get a reference to a specific node's vector.
    pub fn get_vector(&self, id: usize) -> Option<&[f64]> {
        self.nodes.get(id).map(|n| n.vector.as_slice())
    }

    /// Get the degree (number of connections at level 0) for a node.
    pub fn degree(&self, id: usize) -> Option<usize> {
        self.nodes.get(id).and_then(|n| n.connections.first().map(|c| c.len()))
    }

    /// Collect index statistics.
    pub fn stats(&self) -> HnswStats {
        let total_connections: usize = self.nodes.iter().map(|n| n.connections.iter().map(|c| c.len()).sum::<usize>()).sum();
        HnswStats {
            num_vectors: self.nodes.len(),
            max_level: self.max_level,
            total_connections,
            avg_degree: if self.nodes.is_empty() {
                0.0
            } else {
                total_connections as f64 / self.nodes.len() as f64
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
    pub avg_degree: f64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn random_vector(rng: &mut u64, dims: usize) -> Vec<f64> {
        (0..dims)
            .map(|_| {
                *rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((*rng >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0
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
    fn test_single_insert() {
        let mut idx = HnswIndex::new(DistanceMetric::Euclidean);
        idx.insert(vec![1.0, 2.0, 3.0], 0);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.entry_point(), Some(0));
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
            let v = vec![i as f64 * 10.0, i as f64 * 10.0];
            idx.insert(v, i);
        }

        let results = idx.search(&[0.0, 0.0], 5);
        assert_eq!(results.len(), 5, "Should return exactly 5 results");
        // The first 5 closest should be nodes 0, 1, 2, 3, 4
        for i in 0..5 {
            assert_eq!(results[i].1, i, "Result {} should be node {}", i, i);
        }
    }

    #[test]
    fn test_distance_metrics() {
        // Verify compute() produces reasonable values for each metric
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];

        let cos = DistanceMetric::Cosine.compute(&a, &b);
        assert!((cos - 1.0).abs() < 0.01, "Cosine of orthogonal vectors ≈ 1.0, got {cos}");

        let euc = DistanceMetric::Euclidean.compute(&a, &b);
        assert!((euc - 2.0_f64.sqrt()).abs() < 0.01, "Euclidean distance ≈ √2, got {euc}");

        let l2 = DistanceMetric::L2Squared.compute(&a, &b);
        assert!((l2 - 2.0).abs() < 0.01, "L2 squared = 2.0, got {l2}");

        let dot = DistanceMetric::DotProduct.compute(&a, &b);
        assert!((dot - 0.0).abs() < 0.01, "Dot of orthogonal ≈ 0, got {dot}");
    }

    #[test]
    fn test_recall_rate() {
        let mut rng: u64 = 12345;
        let dims = 16;
        let num_vectors = 200;

        let mut idx = HnswIndex::new(DistanceMetric::Euclidean);

        // Insert random vectors
        for i in 0..num_vectors {
            let v = random_vector(&mut rng, dims);
            idx.insert(v, i);
        }

        assert_eq!(idx.len(), num_vectors);

        // Test recall against brute-force for several queries
        let vectors: Vec<Vec<f64>> = (0..num_vectors)
            .map(|i| idx.get_vector(i).unwrap().to_vec())
            .collect();

        for test_idx in 0..10 {
            let query = random_vector(&mut rng, dims);

            // Brute-force top-10
            let mut bf: Vec<(f64, usize)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (DistanceMetric::Euclidean.compute(&query, v), i))
                .collect();
            bf.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            let bf_top10: HashSet<usize> = bf.iter().take(10).map(|(_, id)| *id).collect();

            // HNSW top-10
            let hnsw = idx.search(&query, 10);
            let hnsw_top10: HashSet<usize> = hnsw.iter().map(|(_, id)| *id).collect();

            // Compute recall
            let intersection = bf_top10.intersection(&hnsw_top10).count();
            let recall = intersection as f64 / 10.0;
            assert!(
                recall > 0.7,
                "Recall too low: {recall} (test {test_idx}, expected > 0.7)"
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

        let mut idx = HnswIndex::new(DistanceMetric::Euclidean);

        for i in 0..num_vectors {
            let v = random_vector(&mut rng, dims);
            idx.insert(v, i);
        }

        // Test recall for 20 random queries
        let vectors: Vec<Vec<f64>> = (0..num_vectors)
            .map(|i| idx.get_vector(i).unwrap().to_vec())
            .collect();

        let mut total_recall = 0.0;
        let num_queries = 20;

        for _ in 0..num_queries {
            let query = random_vector(&mut rng, dims);

            let mut bf: Vec<(f64, usize)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (DistanceMetric::Euclidean.compute(&query, v), i))
                .collect();
            bf.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            let bf_top10: HashSet<usize> = bf.iter().take(10).map(|(_, id)| *id).collect();

            let hnsw = idx.search(&query, 10);
            let hnsw_top10: HashSet<usize> = hnsw.iter().map(|(_, id)| *id).collect();

            let intersection = bf_top10.intersection(&hnsw_top10).count();
            total_recall += intersection as f64 / 10.0;
        }

        let avg_recall = total_recall / num_queries as f64;
        assert!(avg_recall > 0.7, "Average recall too low: {avg_recall} (expected > 0.7)");
    }

    #[test]
    fn test_stats_report() {
        let mut idx = HnswIndex::new(DistanceMetric::Euclidean);
        for i in 0..50 {
            idx.insert(vec![i as f64, i as f64], i);
        }
        let stats = idx.stats();
        assert_eq!(stats.num_vectors, 50);
        assert!(stats.avg_degree > 0.0, "Average degree should be > 0");
        assert!(stats.total_connections > 0, "Should have connections");
    }
}
