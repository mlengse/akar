use crate::AlgoResult;
use akar_graph::CSRAdjacency;

use super::rng::SimpleRng;

/// Generate `walks` biased random walks per start node from a seeded RNG.
fn generate_walks(
    csr: &CSRAdjacency,
    p: f64,
    q: f64,
    walks: usize,
    window: usize,
    rng: &mut SimpleRng,
) -> Vec<Vec<usize>> {
    let n = csr.num_nodes();
    let mut all_walks = Vec::with_capacity(n * walks);
    for start in 0..n {
        for _ in 0..walks {
            let mut walk = Vec::with_capacity(window);
            walk.push(start);

            let mut current = start;
            let mut prev = start;

            for _ in 1..window {
                let neighbors = csr.neighbors(current);
                if neighbors.is_empty() {
                    break;
                }

                // Node2Vec biased sampling
                let mut weights = Vec::with_capacity(neighbors.len());
                let mut total_weight = 0.0;

                for (_, dst) in neighbors {
                    let next = dst.offset as usize;
                    let weight = if next == prev {
                        1.0 / p
                    } else if csr.neighbors(prev).iter().any(|(_, d)| d.offset as usize == next) {
                        1.0
                    } else {
                        1.0 / q
                    };
                    weights.push(weight);
                    total_weight += weight;
                }

                let mut r = rng.gen_float() * total_weight;
                let mut next_node = neighbors[0].1.offset as usize;
                for (i, &w) in weights.iter().enumerate() {
                    r -= w;
                    if r <= 0.0 {
                        next_node = neighbors[i].1.offset as usize;
                        break;
                    }
                }

                prev = current;
                current = next_node;
                if current < n {
                    walk.push(current);
                } else {
                    break;
                }
            }
            all_walks.push(walk);
        }
    }
    all_walks
}

/// Skip-gram SGD over the generated walks; mutates `embeddings` in place.
fn train_embeddings(
    embeddings: &mut [f64],
    all_walks: &[Vec<usize>],
    n: usize,
    dimensions: usize,
    window: usize,
    rng: &mut SimpleRng,
) {
    let lr = 0.025;

    for walk in all_walks {
        for (pos, &u) in walk.iter().enumerate() {
            let start = pos.saturating_sub(window / 2);
            let end = (pos + window / 2 + 1).min(walk.len());

            for &v in &walk[start..end] {
                if u == v {
                    continue;
                }

                // Positive sample
                update_embedding(embeddings, u, v, dimensions, 1.0, lr);

                // Negative samples (simplified)
                for _ in 0..5 {
                    let neg = rng.gen_range(n);
                    if neg != u && neg != v {
                        update_embedding(embeddings, u, neg, dimensions, 0.0, lr);
                    }
                }
            }
        }
    }
}

/// Compute Node2Vec graph embedding.
///
/// Since returning full embeddings via AlgoResult (which expects f64 values)
/// requires returning a flat array, we return the flattened embedding matrix.
pub fn compute_node2vec(
    csr: &CSRAdjacency,
    p: f64,
    q: f64,
    dimensions: usize,
    walks: usize,
    window: usize,
) -> AlgoResult {
    let n = csr.num_nodes();
    let mut rng = SimpleRng::new(42);

    let all_walks = generate_walks(csr, p, q, walks, window, &mut rng);

    let mut embeddings = vec![0.0; n * dimensions];
    // Initialize random embeddings
    for val in embeddings.iter_mut() {
        *val = (rng.gen_float() - 0.5) / (dimensions as f64);
    }

    train_embeddings(&mut embeddings, &all_walks, n, dimensions, window, &mut rng);

    AlgoResult {
        name: "node2vec".into(),
        values: embeddings,
        metadata: None,
    }
}

fn update_embedding(embeddings: &mut [f64], u: usize, v: usize, dim: usize, target: f64, lr: f64) {
    let mut dot = 0.0;
    for i in 0..dim {
        dot += embeddings[u * dim + i] * embeddings[v * dim + i];
    }

    // Sigmoid
    let mut prob = 1.0 / (1.0 + (-dot).exp());
    if prob.is_nan() {
        prob = if dot > 0.0 { 1.0 } else { 0.0 };
    }

    let grad = lr * (target - prob);

    for i in 0..dim {
        let update_u = grad * embeddings[v * dim + i];
        let update_v = grad * embeddings[u * dim + i];
        embeddings[u * dim + i] += update_u;
        embeddings[v * dim + i] += update_v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_graph::Edge;

    fn csr_from_edges(edges: &[(u64, u64)], num_nodes: usize) -> CSRAdjacency {
        let edges: Vec<Edge> = edges
            .iter()
            .map(|&(src, dst)| Edge {
                src_offset: src,
                dst_offset: dst,
                rel_id: 0,
                rel_table_id: 0,
            })
            .collect();
        CSRAdjacency::build(&edges, num_nodes)
    }

    #[test]
    fn test_flat_output_dimensions_and_result_metadata() {
        let csr = csr_from_edges(&[(0, 1), (1, 2)], 3);
        for dims in [1usize, 2, 8] {
            let result = compute_node2vec(&csr, 1.0, 1.0, dims, 2, 4);
            assert_eq!(result.values.len(), 3 * dims);
            assert_eq!(result.name, "node2vec");
            assert!(result.metadata.is_none());
        }
    }

    #[test]
    fn test_empty_graph_returns_empty_values() {
        let csr = CSRAdjacency::build(&[], 0);
        let result = compute_node2vec(&csr, 1.0, 1.0, 8, 2, 4);
        assert!(result.values.is_empty());
        assert_eq!(result.name, "node2vec");
    }

    #[test]
    fn test_zero_dimensions_yields_empty_output_without_panicking() {
        let csr = csr_from_edges(&[(0, 1)], 3);
        let result = compute_node2vec(&csr, 1.0, 1.0, 0, 2, 4);
        assert!(result.values.is_empty());
    }

    #[test]
    fn test_single_node_graph_provides_finite_row() {
        let csr = CSRAdjacency::build(&[], 1);
        let result = compute_node2vec(&csr, 1.0, 1.0, 6, 2, 4);
        assert_eq!(result.values.len(), 6);
        assert!(result.values.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn test_isolated_node_still_gets_an_initialized_row() {
        let csr = csr_from_edges(&[(0, 1)], 3);
        let dims = 4;
        let result = compute_node2vec(&csr, 1.0, 1.0, dims, 3, 5);
        assert_eq!(result.values.len(), 3 * dims);
        assert!(result.values.iter().all(|v| v.is_finite()));
        assert!(result.values[2 * dims..3 * dims].iter().any(|&v| v != 0.0));
    }

    #[test]
    fn test_training_on_cycle_keeps_all_values_finite() {
        let csr = csr_from_edges(&[(0, 1), (1, 2), (2, 3), (3, 0)], 4);
        let result = compute_node2vec(&csr, 1.0, 1.0, 8, 4, 5);
        assert_eq!(result.values.len(), 32);
        assert!(result.values.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn test_biased_walk_parameters_produce_finite_embeddings() {
        let csr = csr_from_edges(&[(0, 1), (1, 2), (2, 0)], 3);
        let result = compute_node2vec(&csr, 0.25, 2.0, 8, 4, 5);
        assert_eq!(result.values.len(), 24);
        assert!(result.values.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn test_deterministic_for_same_inputs() {
        // Star graph: center 0 with three leaves exercises the biased sampler and
        // negative sampling; the seeded LCG must reproduce bit-identical output.
        let csr = csr_from_edges(&[(0, 1), (0, 2), (0, 3)], 4);
        let first = compute_node2vec(&csr, 1.0, 4.0, 8, 3, 4).values;
        let second = compute_node2vec(&csr, 1.0, 4.0, 8, 3, 4).values;
        assert_eq!(first, second);
    }

    #[test]
    fn test_window_one_returns_pure_seeded_initialization() {
        // window=1 → every walk is just [start]: no RNG draws during walk
        // generation, no context pairs, so output equals the init sequence.
        let csr = csr_from_edges(&[(0, 1)], 3);
        let dims = 4;
        let result = compute_node2vec(&csr, 1.0, 1.0, dims, 2, 1);

        let mut rng = SimpleRng::new(42);
        let expected: Vec<f64> = (0..3 * dims).map(|_| (rng.gen_float() - 0.5) / dims as f64).collect();
        assert_eq!(result.values, expected);
    }

    #[test]
    fn test_update_embedding_positive_target_pulls_pair_together() {
        // e_u=[1,0], e_v=[0,1]: dot=0, sigmoid(0)=0.5, grad=0.025*0.5=0.0125.
        let mut embeddings = vec![1.0, 0.0, 0.0, 1.0];
        update_embedding(&mut embeddings, 0, 1, 2, 1.0, 0.025);
        assert_eq!(embeddings, vec![1.0, 0.0125, 0.0125, 1.0]);
        let dot = embeddings[0] * embeddings[2] + embeddings[1] * embeddings[3];
        assert!(dot > 0.0);
    }

    #[test]
    fn test_update_embedding_negative_target_pushes_pair_apart() {
        // Same start as above but target=0 → grad=-0.0125 flips the dot sign.
        let mut embeddings = vec![1.0, 0.0, 0.0, 1.0];
        update_embedding(&mut embeddings, 0, 1, 2, 0.0, 0.025);
        assert_eq!(embeddings, vec![1.0, -0.0125, -0.0125, 1.0]);
        let dot = embeddings[0] * embeddings[2] + embeddings[1] * embeddings[3];
        assert!(dot < 0.0);
    }

    fn seeded_init_replay(n: usize, dims: usize) -> Vec<f64> {
        let mut rng = SimpleRng::new(42);
        (0..n * dims).map(|_| (rng.gen_float() - 0.5) / dims as f64).collect()
    }

    #[test]
    fn test_zero_walks_yields_pure_seeded_initialization() {
        // walks=0 → no walks are generated and no RNG draws happen beyond
        // init, so the output must equal the raw seeded init sequence.
        let csr = csr_from_edges(&[(0, 1), (1, 2)], 3);
        let dims = 4;
        let result = compute_node2vec(&csr, 1.0, 1.0, dims, 0, 4);
        assert_eq!(result.values, seeded_init_replay(3, dims));
    }

    #[test]
    fn test_window_zero_yields_pure_seeded_initialization() {
        // window=0 → every walk stops at [start] (`1..0` is empty) and the
        // context slice degenerates to u itself, so training is a no-op.
        let csr = csr_from_edges(&[(0, 1), (1, 2)], 3);
        let dims = 4;
        let result = compute_node2vec(&csr, 1.0, 1.0, dims, 2, 0);
        assert_eq!(result.values, seeded_init_replay(3, dims));
    }

    #[test]
    fn test_walk_generation_invariants() {
        // Every walk starts at its seed node, stays within the window bound,
        // and only ever hops along real CSR edges.
        let csr = csr_from_edges(&[(0, 1), (1, 2), (2, 0)], 3);
        let mut rng = SimpleRng::new(7);
        let walks = generate_walks(&csr, 1.0, 1.0, 4, 5, &mut rng);
        assert_eq!(walks.len(), 3 * 4);
        let mut starts_seen = vec![0usize; 3];
        for walk in walks.iter() {
            starts_seen[walk[0]] += 1;
            assert!(walk.len() <= 5);
            for pair in walk.windows(2) {
                assert!(csr.neighbors(pair[0]).iter().any(|(_, d)| d.offset as usize == pair[1]));
            }
        }
        // Exactly `walks` replicas per seed node.
        assert_eq!(starts_seen, vec![4, 4, 4]);
    }

    #[test]
    fn test_isolated_node_produces_single_step_walks() {
        let csr = csr_from_edges(&[(1, 2)], 3);
        let mut rng = SimpleRng::new(9);
        let walks = generate_walks(&csr, 1.0, 1.0, 2, 4, &mut rng);
        assert_eq!(walks[0], vec![0]);
        assert_eq!(walks[1], vec![0]);
    }

    #[test]
    fn test_train_embeddings_pulls_context_pair_together() {
        // Opposing vectors (dot=-1) must move toward each other under the
        // positive-skipgram update; skipping training leaves dot at exactly -1.
        let mut embeddings = vec![1.0, 0.0, -1.0, 0.0];
        let walks = vec![vec![0usize, 1]];
        let mut rng = SimpleRng::new(3);
        train_embeddings(&mut embeddings, &walks, 2, 2, 2, &mut rng);
        let dot_after = embeddings[0] * embeddings[2] + embeddings[1] * embeddings[3];
        assert!(dot_after > -1.0);
    }

    #[test]
    fn test_extreme_bias_parameters_stay_finite_and_deterministic() {
        // p=q=0 → 1/p and 1/q are +inf; p=q=∞ → weights collapse to 0.
        // Both extremes must stay panic-free, finite, and bit-reproducible.
        let csr = csr_from_edges(&[(0, 1), (1, 2), (2, 0)], 3);
        for &(p, q) in &[(0.0, 0.0), (f64::INFINITY, f64::INFINITY)] {
            let first = compute_node2vec(&csr, p, q, 8, 4, 5);
            let second = compute_node2vec(&csr, p, q, 8, 4, 5);
            assert_eq!(first.values.len(), 24);
            assert!(first.values.iter().all(|v| v.is_finite()));
            assert_eq!(first.values, second.values);
        }
    }

    #[test]
    fn test_self_loop_and_parallel_edges_are_handled() {
        // A self-loop parks the walk on node 0 and parallel edges duplicate
        // CSR neighbor entries; neither may panic nor produce non-finite rows.
        let csr = csr_from_edges(&[(0, 0), (0, 1), (0, 1), (1, 2)], 3);
        let dims = 4;
        let result = compute_node2vec(&csr, 1.0, 1.0, dims, 3, 4);
        assert_eq!(result.values.len(), 3 * dims);
        assert!(result.values.iter().all(|v| v.is_finite()));
    }
}
