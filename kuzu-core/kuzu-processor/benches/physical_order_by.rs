//! PhysicalOrderBy throughput benchmarks — single-key vs multi-key sorting.

use std::hint::black_box;
use criterion::{Criterion, criterion_group, criterion_main};
use kuzu_common::types::PhysicalTypeID;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_processor::physical_operator::{PhysicalOperatorExec, PhysicalOrderBy};

/// Create a single-column Int64 DataChunk with num_rows rows.
fn make_i64_chunk(values: &[i64]) -> DataChunk {
    let n = values.len();
    let mut v = ValueVector::new(PhysicalTypeID::Int64, n);
    v.resize(n);
    for (i, &val) in values.iter().enumerate() {
        v.set_i64(i, val);
    }
    DataChunk::from_legacy(vec![v])
}

/// Create a multi-column DataChunk: col_0 = Int64 key, col_1 = String label, col_2 = Double score.
fn make_multi_col_chunk(keys: &[i64], labels: &[&str], scores: &[f64]) -> DataChunk {
    let n = keys.len();
    // Column 0: Int64
    let mut c0 = ValueVector::new(PhysicalTypeID::Int64, n);
    c0.resize(n);
    for (i, &k) in keys.iter().enumerate() {
        c0.set_i64(i, k);
    }
    // Column 1: String (inlined as 16-byte prefix)
    let mut c1 = ValueVector::new(PhysicalTypeID::String, n);
    c1.resize(n);
    for (i, &label) in labels.iter().enumerate() {
        let bytes = label.as_bytes();
        let len = bytes.len().min(15) as u8;
        let offset = i * 16;
        c1.data_mut()[offset] = len;
        c1.data_mut()[offset + 1..offset + 1 + bytes.len().min(15)].copy_from_slice(&bytes[..bytes.len().min(15)]);
        c1.set_null(i, false);
    }
    // Column 2: Double
    let mut c2 = ValueVector::new(PhysicalTypeID::Double, n);
    c2.resize(n);
    for (i, &s) in scores.iter().enumerate() {
        c2.set_double(i, s);
    }
    DataChunk::from_legacy(vec![c0, c1, c2])
}

/// Generate shuffled values 0..n-1 for unsorted input.
fn shuffled(n: usize) -> Vec<i64> {
    let mut v: Vec<i64> = (0..n as i64).collect();
    // Simple pseudo-random shuffle using a fixed seed for reproducibility
    let mut seed: u64 = 42;
    for i in (1..v.len()).rev() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let j = (seed >> 33) as usize % (i + 1);
        v.swap(i, j);
    }
    v
}

fn bench_order_by_single_key_100(c: &mut Criterion) {
    let order = PhysicalOrderBy {
        sort_keys: vec![(0, true)],
    };
    let vals = shuffled(100);
    let chunk = make_i64_chunk(&vals);
    c.bench_function("order_by/single_key_100", |b| {
        b.iter(|| {
            let result = order.execute(black_box(vec![chunk.clone()]));
            black_box(result.unwrap());
        })
    });
}

fn bench_order_by_single_key_1k(c: &mut Criterion) {
    let order = PhysicalOrderBy {
        sort_keys: vec![(0, true)],
    };
    let vals = shuffled(1_000);
    let chunk = make_i64_chunk(&vals);
    c.bench_function("order_by/single_key_1k", |b| {
        b.iter(|| {
            let result = order.execute(black_box(vec![chunk.clone()]));
            black_box(result.unwrap());
        })
    });
}

fn bench_order_by_single_key_10k(c: &mut Criterion) {
    let order = PhysicalOrderBy {
        sort_keys: vec![(0, true)],
    };
    let vals = shuffled(10_000);
    let chunk = make_i64_chunk(&vals);
    c.bench_function("order_by/single_key_10k", |b| {
        b.iter(|| {
            let result = order.execute(black_box(vec![chunk.clone()]));
            black_box(result.unwrap());
        })
    });
}

fn bench_order_by_multi_key(c: &mut Criterion) {
    let order = PhysicalOrderBy {
        sort_keys: vec![(0, true), (2, false)],
    };
    let n = 1_000;
    let keys = shuffled(n);
    let labels: Vec<&str> = (0..n)
        .map(|i| match i % 4 {
            0 => "alpha",
            1 => "beta",
            2 => "gamma",
            _ => "delta",
        })
        .collect();
    let scores: Vec<f64> = (0..n).map(|i| (n - i) as f64 * 0.5).collect();
    let chunk = make_multi_col_chunk(&keys, &labels, &scores);
    c.bench_function("order_by/multi_key_1k", |b| {
        b.iter(|| {
            let result = order.execute(black_box(vec![chunk.clone()]));
            black_box(result.unwrap());
        })
    });
}

fn bench_order_by_descending(c: &mut Criterion) {
    let order = PhysicalOrderBy {
        sort_keys: vec![(0, false)],
    };
    let vals = shuffled(1_000);
    let chunk = make_i64_chunk(&vals);
    c.bench_function("order_by/descending_1k", |b| {
        b.iter(|| {
            let result = order.execute(black_box(vec![chunk.clone()]));
            black_box(result.unwrap());
        })
    });
}

criterion_group!(
    benches,
    bench_order_by_single_key_100,
    bench_order_by_single_key_1k,
    bench_order_by_single_key_10k,
    bench_order_by_multi_key,
    bench_order_by_descending,
);
criterion_main!(benches);
