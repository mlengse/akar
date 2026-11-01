//! bench_021 — HNSW graph-mode recall accuracy measurement vehicle.
//!
//! Measures top-k overlap against exhaustive search and p95 query latency
//! for HNSW graph-mode search at configurable corpus sizes.
//!
//! Usage:
//!   cargo run --release --bin bench_021 -p akar-vector -- --vectors 10000 --threshold 0 --metric cosine --dim 384 --distribution clustered --clusters 100 --spread 0.15 --ef 200

use akar_vector::hnsw::{DistanceMetric, HnswConfig, HnswIndex};
use std::collections::HashSet;
use std::env;
use std::time::Instant;

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut vectors: usize = 10_000;
    let mut threshold: usize = 0;
    let mut metric_name: String = "cosine".to_string();
    let mut dim: usize = 384;
    let mut distribution_name: String = "clustered".to_string();
    let mut clusters: usize = 100;
    let mut spread: f32 = 0.15;
    let mut queries: usize = 10;
    let mut k: usize = 10;
    let mut ef: usize = 200;
    let mut m: usize = 16;
    let mut m_max: usize = 32;
    let mut ef_construction: usize = 512;
    let mut seed: u64 = 42;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--vectors" => {
                vectors = args[i + 1].parse().unwrap();
                i += 2;
            }
            "--threshold" => {
                threshold = args[i + 1].parse().unwrap();
                i += 2;
            }
            "--metric" => {
                metric_name = args[i + 1].clone();
                i += 2;
            }
            "--dim" => {
                dim = args[i + 1].parse().unwrap();
                i += 2;
            }
            "--distribution" => {
                distribution_name = args[i + 1].clone();
                i += 2;
            }
            "--clusters" => {
                clusters = args[i + 1].parse().unwrap();
                i += 2;
            }
            "--spread" => {
                spread = args[i + 1].parse().unwrap();
                i += 2;
            }
            "--queries" => {
                queries = args[i + 1].parse().unwrap();
                i += 2;
            }
            "--k" => {
                k = args[i + 1].parse().unwrap();
                i += 2;
            }
            "--ef" => {
                ef = args[i + 1].parse().unwrap();
                i += 2;
            }
            "--m" => {
                m = args[i + 1].parse().unwrap();
                i += 2;
            }
            "--m-max" => {
                m_max = args[i + 1].parse().unwrap();
                i += 2;
            }
            "--ef-construction" => {
                ef_construction = args[i + 1].parse().unwrap();
                i += 2;
            }
            "--seed" => {
                seed = args[i + 1].parse().unwrap();
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let metric = match metric_name.as_str() {
        "cosine" => DistanceMetric::Cosine,
        "euclidean" => DistanceMetric::Euclidean,
        "l1" => DistanceMetric::L1,
        "l2squared" => DistanceMetric::L2Squared,
        "dot" => DistanceMetric::DotProduct,
        _ => DistanceMetric::Cosine,
    };

    let config = HnswConfig::new(m, m_max, ef_construction, ef).expect("Valid HnswConfig parameters");

    // Generate corpus based on selected distribution
    let mut rng = seed;
    let (corpus, centroids) = if distribution_name == "uniform" {
        let corp: Vec<Vec<f32>> = (0..vectors).map(|_| random_vector(&mut rng, dim)).collect();
        (corp, Vec::new())
    } else {
        // Clustered
        let num_centroids = clusters.max(1);
        let cents: Vec<Vec<f32>> = (0..num_centroids).map(|_| random_vector(&mut rng, dim)).collect();
        let corp: Vec<Vec<f32>> = (0..vectors)
            .map(|idx| {
                let centroid = &cents[idx % cents.len()];
                clustered_vector(&mut rng, centroid, spread)
            })
            .collect();
        (corp, cents)
    };

    // Build index with custom config and threshold
    let mut index = HnswIndex::with_config(metric, threshold, config);
    for (id, v) in corpus.iter().enumerate() {
        index.insert(v.clone(), id);
    }

    // Generate queries with a derived seed matching active distribution
    rng = seed.wrapping_add(1);
    let query_vecs: Vec<Vec<f32>> = if distribution_name == "uniform" {
        (0..queries).map(|_| random_vector(&mut rng, dim)).collect()
    } else {
        (0..queries)
            .map(|idx| {
                let centroid = &centroids[idx % centroids.len()];
                clustered_vector(&mut rng, centroid, spread * 0.5)
            })
            .collect()
    };

    // Measure overlap and latency per query
    let mut overlaps = Vec::new();
    let mut latencies_ns = Vec::new();

    for query in &query_vecs {
        // Exact top-k by full scan — ground truth.
        let exact = exact_top_k(&index, query, k);
        let exact_set: HashSet<usize> = exact.iter().copied().collect();

        // Graph search with timing using ef.
        let start = Instant::now();
        let graph_hits = index.search_with_ef(query, k, ef);
        let elapsed = start.elapsed();
        latencies_ns.push(elapsed.as_nanos());

        let graph_set: HashSet<usize> = graph_hits.iter().map(|(_, id)| *id).collect();
        let hit_count = graph_set.intersection(&exact_set).count();
        let overlap = hit_count as f64 / k.max(1) as f64;
        overlaps.push(overlap);
    }

    // Aggregate overlap.
    let avg_overlap: f64 = overlaps.iter().sum::<f64>() / overlaps.len().max(1) as f64;
    let min_overlap = overlaps.iter().fold(f64::INFINITY, |a, &b| a.min(b));
    let max_overlap = overlaps.iter().fold(0.0f64, |a, &b| a.max(b));

    // Aggregate latency (ms).
    let mut latencies_ms: Vec<f64> = latencies_ns.iter().map(|&n| n as f64 / 1_000_000.0).collect();
    latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95_idx = ((latencies_ms.len() as f64) * 0.95).round() as usize;
    let p95_latency_ms = if latencies_ms.is_empty() {
        0.0
    } else {
        latencies_ms[p95_idx.saturating_sub(1).min(latencies_ms.len() - 1)]
    };
    let avg_latency_ms: f64 = latencies_ms.iter().sum::<f64>() / latencies_ms.len().max(1) as f64;

    // Report.
    println!("=== bench_021: HNSW Graph-Mode Recall Accuracy ===");
    println!("Corpus vectors: {}", vectors);
    println!("Dimensions: {}", dim);
    if distribution_name == "uniform" {
        println!("Distribution: uniform");
    } else {
        println!("Distribution: clustered (clusters: {}, spread: {})", clusters, spread);
    }
    println!("Metric: {:?}", metric);
    println!("Exact threshold: {}", threshold);
    println!(
        "Graph config: M={}, M_max={}, ef_construction={}",
        m, m_max, ef_construction
    );
    println!("Graph built: {}", index.is_graph_built());
    println!("Max level: {}", index.max_level());
    println!("Queries: {}", queries);
    println!("k: {}, ef: {}", k, ef);
    println!();
    println!("Overlap @{} vs exhaustive:", k);
    println!("  average: {:.4}", avg_overlap);
    println!("  min:     {:.4}", min_overlap);
    println!("  max:     {:.4}", max_overlap);
    println!();
    println!("Query latency:");
    println!("  average: {:.3} ms", avg_latency_ms);
    println!("  p95:     {:.3} ms", p95_latency_ms);
    println!();

    // Target checks.
    if avg_overlap >= 0.90 {
        println!("✓ OVERLAP @{} >= 0.90: PASS ({:.4})", k, avg_overlap);
    } else {
        println!("✗ OVERLAP @{} >= 0.90: FAIL ({:.4})", k, avg_overlap);
    }

    if p95_latency_ms <= 5.0 {
        println!("✓ P95 LATENCY <= 5.0ms: PASS ({:.3} ms)", p95_latency_ms);
    } else {
        println!("✗ P95 LATENCY <= 5.0ms: FAIL ({:.3} ms)", p95_latency_ms);
    }
}

/// Generate a Gaussian perturbation around `centroid` and normalize to unit length (L2 norm = 1.0).
fn clustered_vector(rng: &mut u64, centroid: &[f32], spread: f32) -> Vec<f32> {
    let mut v = Vec::with_capacity(centroid.len());
    let mut i = 0;
    while i < centroid.len() {
        let u1 = random_uniform_01(rng);
        let u2 = random_uniform_01(rng);
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = 2.0 * std::f32::consts::PI * u2;
        let z0 = r * theta.cos();
        let z1 = r * theta.sin();

        v.push(centroid[i] + spread * z0);
        if i + 1 < centroid.len() {
            v.push(centroid[i + 1] + spread * z1);
        }
        i += 2;
    }
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

fn random_uniform_01(rng: &mut u64) -> f32 {
    *rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (((*rng >> 33) as f32 + 1.0) / (1u64 << 31) as f32).clamp(1e-7, 1.0)
}

/// Generate a random normalized vector using LCG (matches hnsw test pattern).
fn random_vector(rng: &mut u64, dims: usize) -> Vec<f32> {
    let mut v = Vec::with_capacity(dims);
    for _ in 0..dims {
        *rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        v.push(((*rng >> 33) as f32 / (1u32 << 31) as f32) - 0.5);
    }
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Exact top-k by full scan — ground truth the ANN must approach.
fn exact_top_k(idx: &HnswIndex, query: &[f32], k: usize) -> Vec<usize> {
    let mut all: Vec<(f32, usize)> = idx
        .nodes()
        .into_iter()
        .map(|(id, v)| (idx.metric().compute(v, query), id))
        .collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    all.truncate(k);
    all.into_iter().map(|(_, id)| id).collect()
}
