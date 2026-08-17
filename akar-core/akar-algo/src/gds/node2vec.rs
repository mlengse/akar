use crate::AlgoResult;
use akar_graph::CSRAdjacency;

struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
    fn next_u32(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.state >> 32) as u32
    }
    fn gen_range(&mut self, bound: usize) -> usize {
        (self.next_u32() as usize) % bound
    }
    fn gen_float(&mut self) -> f64 {
        (self.next_u32() as f64) / (u32::MAX as f64)
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

    // 1. Generate biased random walks
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

    // 2. Simple Skip-gram optimization (stochastic gradient descent)
    let mut embeddings = vec![0.0; n * dimensions];
    // Initialize random embeddings
    for val in embeddings.iter_mut() {
        *val = (rng.gen_float() - 0.5) / (dimensions as f64);
    }

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
                update_embedding(&mut embeddings, u, v, dimensions, 1.0, lr);

                // Negative samples (simplified)
                for _ in 0..5 {
                    let neg = rng.gen_range(n);
                    if neg != u && neg != v {
                        update_embedding(&mut embeddings, u, neg, dimensions, 0.0, lr);
                    }
                }
            }
        }
    }

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
