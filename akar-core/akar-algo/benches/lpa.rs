//! Label Propagation (LPA) benchmarks on synthetic community graphs.

use std::hint::black_box;
use std::time::Duration;

use akar_algo::compute_lpa;
use akar_graph::{CSRAdjacency, Edge};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Deterministic synthetic graph: nodes grouped in 256-node communities,
/// 90% intra-community edges + 10% cross-community noise, stored both
/// directions (undirected semantics).
fn build_csr(num_nodes: usize, avg_degree: usize) -> CSRAdjacency {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    const BLOCK: usize = 256;
    let mut edges: Vec<Edge> = Vec::new();
    let mut rel_id = 0u64;
    for v in 0..num_nodes {
        for _ in 0..(avg_degree / 2) {
            let w = if rng.next() % 100 < 90 {
                let lo = (v / BLOCK) * BLOCK;
                let hi = (lo + BLOCK).min(num_nodes);
                lo + (rng.next() as usize) % (hi - lo)
            } else {
                (rng.next() as usize) % num_nodes
            };
            if w == v {
                continue;
            }
            for &(s, d) in &[(v, w), (w, v)] {
                edges.push(Edge {
                    src_offset: s as u64,
                    dst_offset: d as u64,
                    rel_id,
                    rel_table_id: 0,
                });
                rel_id += 1;
            }
        }
    }
    CSRAdjacency::build(&edges, num_nodes)
}

fn bench_lpa(c: &mut Criterion) {
    let mut group = c.benchmark_group("lpa");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(4));
    for &size in &[2_000usize, 10_000, 40_000] {
        let csr = build_csr(size, 16);
        group.bench_with_input(BenchmarkId::new("compute_lpa", size), &csr, |b, csr| {
            b.iter(|| compute_lpa(black_box(csr), black_box(30)))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_lpa);
criterion_main!(benches);
