//! Vector benchmarks for feature 021 (SC-002, SC-003, SC-004).
//!
//! ```text
//! cargo run -p akar-vector --release --example bench_021
//! cargo run -p akar-vector --release --example bench_021 -- threshold=0
//! ```
//!
//! The default run measures the shipped path (exact scan at or below
//! `EXACT_SEARCH_MAX_NODES`). Passing `threshold=<n>` changes the exact-scan
//! cutoff: `threshold=0` measures the incremental HNSW path, and
//! `threshold=<n>` just below the corpus size measures the one-pass relink that
//! happens when a flat store crosses the threshold.

use akar_vector::hnsw::{DistanceMetric, EXACT_SEARCH_MAX_NODES, HnswIndex};
use std::time::{Duration, Instant};

const DIM: usize = 384;
const QUERIES: usize = 50;
const K: usize = 10;

struct Lcg(u64);

impl Lcg {
    fn next_f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0
    }

    fn vector(&mut self) -> Vec<f32> {
        (0..DIM).map(|_| self.next_f32()).collect()
    }
}

fn p95(samples: &mut [Duration]) -> Duration {
    samples.sort_unstable();
    let rank = ((samples.len() as f64) * 0.95).ceil() as usize;
    samples[rank.saturating_sub(1).min(samples.len() - 1)]
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn overlap(approx: &[(f32, usize)], exact: &[(f32, usize)]) -> f64 {
    let exact_ids: Vec<usize> = exact.iter().map(|(_, id)| *id).collect();
    let hits = approx.iter().filter(|(_, id)| exact_ids.contains(id)).count();
    hits as f64 / K as f64
}

fn main() {
    let metric = DistanceMetric::Cosine;
    let threshold = std::env::args()
        .skip(1)
        .find_map(|arg| arg.strip_prefix("threshold=").and_then(|value| value.parse().ok()))
        .unwrap_or(EXACT_SEARCH_MAX_NODES);
    for corpus in [6_000usize, 10_000] {
        let mut rng = Lcg(0x5eed_2021);
        let mut index = HnswIndex::with_exact_threshold(metric, threshold);
        let build = Instant::now();
        for id in 0..corpus {
            index.insert(rng.vector(), id);
        }
        let build = build.elapsed();
        println!(
            "SC-001 corpus={corpus} build_ms={:.1} mode={} exact_threshold={threshold}",
            ms(build),
            if index.is_graph_built() { "hnsw" } else { "exact" }
        );

        let nodes = index.nodes();
        let queries: Vec<Vec<f32>> = (0..QUERIES).map(|_| rng.vector()).collect();

        let exact_sets: Vec<Vec<(f32, usize)>> = queries
            .iter()
            .map(|query| {
                let mut scored: Vec<(f32, usize)> = nodes
                    .iter()
                    .map(|(id, vector)| (metric.compute(query, vector), *id))
                    .collect();
                scored.sort_by(|a, b| a.0.total_cmp(&b.0));
                scored.truncate(K);
                scored
            })
            .collect();

        for ef in [16usize, 64, 128] {
            let mut samples = Vec::with_capacity(QUERIES);
            let mut overlaps = Vec::with_capacity(QUERIES);
            for (query, exact) in queries.iter().zip(&exact_sets) {
                let started = Instant::now();
                let approx = index.search_with_ef(query, K, ef);
                samples.push(started.elapsed());
                overlaps.push(overlap(&approx, exact));
            }
            let mean_overlap: f64 = overlaps.iter().sum::<f64>() / overlaps.len() as f64;
            println!(
                "SC-002 corpus={corpus} ef={ef} overlap_at_10={mean_overlap:.3} p95_ms={:.3}",
                ms(p95(&mut samples))
            );
        }

        let mut samples = Vec::with_capacity(QUERIES);
        for query in &queries {
            let started = Instant::now();
            let _ = index.search(query, K);
            samples.push(started.elapsed());
        }
        println!(
            "SC-003 corpus={corpus} default_ef p95_top10_ms={:.3} build_ms={:.1}",
            ms(p95(&mut samples)),
            ms(build)
        );

        let f32_bytes = corpus * DIM * std::mem::size_of::<f32>();
        let f64_bytes = corpus * DIM * std::mem::size_of::<f64>();
        println!(
            "SC-004 corpus={corpus} f32_bytes={f32_bytes} f64_bytes={f64_bytes} ratio={:.2}",
            f32_bytes as f64 / f64_bytes as f64
        );
    }

    let mut rng = Lcg(0xfa11_2021);
    let mut max_error = 0.0f64;
    for _ in 0..1_000 {
        let a = rng.vector();
        let b = rng.vector();
        let f32_distance = metric.compute(&a, &b) as f64;
        let f64_distance = f64_cosine(&a, &b);
        max_error = max_error.max((f32_distance - f64_distance).abs());
    }
    println!("SC-004 max_abs_distance_error_vs_f64={max_error:.3e}");
}

fn f64_cosine(a: &[f32], b: &[f32]) -> f64 {
    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;
    for (x, y) in a.iter().zip(b) {
        let x = *x as f64;
        let y = *y as f64;
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 1.0;
    }
    1.0 - dot / (norm_a.sqrt() * norm_b.sqrt())
}
