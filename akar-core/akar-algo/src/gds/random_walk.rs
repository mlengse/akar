use crate::AlgoResult;
use akar_graph::CSRAdjacency;

/// A simple Linear Congruential Generator for random walks without extra dependencies
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
}

/// Compute random walk on a graph.
///
/// Returns the hit count for each node across all walks.
pub fn compute_random_walk(
    csr: &CSRAdjacency,
    start_node: Option<usize>,
    steps: usize,
    walks_per_node: usize,
) -> AlgoResult {
    let n = csr.num_nodes();
    let mut hit_counts = vec![0.0; n];
    let mut rng = SimpleRng::new(42);

    let start_nodes = match start_node {
        Some(node) => vec![node],
        None => (0..n).collect(),
    };

    for &start in &start_nodes {
        if start >= n {
            continue;
        }

        for _ in 0..walks_per_node {
            let mut current = start;
            hit_counts[current] += 1.0;

            for _ in 0..steps {
                let neighbors = csr.neighbors(current);
                if neighbors.is_empty() {
                    break;
                }

                let idx = rng.gen_range(neighbors.len());
                current = neighbors[idx].1.offset as usize;

                if current < n {
                    hit_counts[current] += 1.0;
                } else {
                    break;
                }
            }
        }
    }

    AlgoResult {
        name: "random_walk".into(),
        values: hit_counts,
    }
}
