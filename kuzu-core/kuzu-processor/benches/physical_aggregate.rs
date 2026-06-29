//! PhysicalAggregate throughput benchmarks — COUNT, SUM, AVG with/without GROUP BY.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use kuzu_common::types::PhysicalTypeID;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_processor::physical_operator::{PhysicalAggregate, PhysicalOperatorExec};

/// Create a single-column Int64 DataChunk with values 0..num_rows.
fn make_i64_chunk(values: &[i64]) -> DataChunk {
    let n = values.len();
    let mut v = ValueVector::new(PhysicalTypeID::Int64, n);
    v.resize(n);
    for (i, &val) in values.iter().enumerate() {
        v.set_i64(i, val);
    }
    DataChunk::new(vec![v])
}

/// Create a two-column DataChunk: col_0 = group key (Int64), col_1 = value (Int64).
fn make_grouped_chunk(keys: &[i64], values: &[i64]) -> DataChunk {
    let n = keys.len();
    let mut c0 = ValueVector::new(PhysicalTypeID::Int64, n);
    c0.resize(n);
    let mut c1 = ValueVector::new(PhysicalTypeID::Int64, n);
    c1.resize(n);
    for i in 0..n {
        c0.set_i64(i, keys[i]);
        c1.set_i64(i, values[i]);
    }
    DataChunk::new(vec![c0, c1])
}

/// Create a three-column DataChunk for multi-key GROUP BY.
fn make_multi_key_chunk(key_a: &[i64], key_b: &[i64], values: &[i64]) -> DataChunk {
    let n = key_a.len();
    let mut c0 = ValueVector::new(PhysicalTypeID::Int64, n);
    c0.resize(n);
    let mut c1 = ValueVector::new(PhysicalTypeID::Int64, n);
    c1.resize(n);
    let mut c2 = ValueVector::new(PhysicalTypeID::Int64, n);
    c2.resize(n);
    for i in 0..n {
        c0.set_i64(i, key_a[i]);
        c1.set_i64(i, key_b[i]);
        c2.set_i64(i, values[i]);
    }
    DataChunk::new(vec![c0, c1, c2])
}

// ==================== Scalar aggregates (no GROUP BY) ====================

fn bench_count_100(c: &mut Criterion) {
    let agg = PhysicalAggregate {
        group_by_cols: vec![],
        aggregate_functions: vec!["COUNT".into()],
    };
    let chunk = make_i64_chunk(&(0..100).collect::<Vec<i64>>());
    c.bench_function("aggregate/count_100", |b| {
        b.iter(|| {
            black_box(agg.execute(black_box(vec![chunk.clone()])).unwrap());
        })
    });
}

fn bench_count_10k(c: &mut Criterion) {
    let agg = PhysicalAggregate {
        group_by_cols: vec![],
        aggregate_functions: vec!["COUNT".into()],
    };
    let chunk = make_i64_chunk(&(0..10_000).collect::<Vec<i64>>());
    c.bench_function("aggregate/count_10k", |b| {
        b.iter(|| {
            black_box(agg.execute(black_box(vec![chunk.clone()])).unwrap());
        })
    });
}

fn bench_sum_10k(c: &mut Criterion) {
    let agg = PhysicalAggregate {
        group_by_cols: vec![],
        aggregate_functions: vec!["SUM".into()],
    };
    let chunk = make_i64_chunk(&(0..10_000).collect::<Vec<i64>>());
    c.bench_function("aggregate/sum_10k", |b| {
        b.iter(|| {
            black_box(agg.execute(black_box(vec![chunk.clone()])).unwrap());
        })
    });
}

fn bench_avg_10k(c: &mut Criterion) {
    let agg = PhysicalAggregate {
        group_by_cols: vec![],
        aggregate_functions: vec!["AVG".into()],
    };
    let chunk = make_i64_chunk(&(0..10_000).collect::<Vec<i64>>());
    c.bench_function("aggregate/avg_10k", |b| {
        b.iter(|| {
            black_box(agg.execute(black_box(vec![chunk.clone()])).unwrap());
        })
    });
}

fn bench_multi_agg_10k(c: &mut Criterion) {
    let agg = PhysicalAggregate {
        group_by_cols: vec![],
        aggregate_functions: vec!["COUNT".into(), "SUM".into(), "AVG".into(), "MIN".into(), "MAX".into()],
    };
    let chunk = make_i64_chunk(&(0..10_000).collect::<Vec<i64>>());
    c.bench_function("aggregate/multi_func_10k", |b| {
        b.iter(|| {
            black_box(agg.execute(black_box(vec![chunk.clone()])).unwrap());
        })
    });
}

// ==================== GROUP BY aggregates ====================

fn bench_group_by_few(c: &mut Criterion) {
    let agg = PhysicalAggregate {
        group_by_cols: vec![0],
        aggregate_functions: vec!["COUNT".into(), "SUM".into()],
    };
    // 10k rows, 10 groups (key = row % 10)
    let keys: Vec<i64> = (0..10_000).map(|i| (i % 10) as i64).collect();
    let vals: Vec<i64> = (0..10_000).collect();
    let chunk = make_grouped_chunk(&keys, &vals);
    c.bench_function("aggregate/group_by_10_groups_10k", |b| {
        b.iter(|| {
            black_box(agg.execute(black_box(vec![chunk.clone()])).unwrap());
        })
    });
}

fn bench_group_by_many(c: &mut Criterion) {
    let agg = PhysicalAggregate {
        group_by_cols: vec![0],
        aggregate_functions: vec!["COUNT".into()],
    };
    // 10k rows, 1k groups (key = row % 1000)
    let keys: Vec<i64> = (0..10_000).map(|i| (i % 1000) as i64).collect();
    let vals: Vec<i64> = (0..10_000).collect();
    let chunk = make_grouped_chunk(&keys, &vals);
    c.bench_function("aggregate/group_by_1k_groups_10k", |b| {
        b.iter(|| {
            black_box(agg.execute(black_box(vec![chunk.clone()])).unwrap());
        })
    });
}

fn bench_group_by_multi_key(c: &mut Criterion) {
    let agg = PhysicalAggregate {
        group_by_cols: vec![0, 1],
        aggregate_functions: vec!["COUNT".into()],
    };
    // 10k rows, 100 groups (key_a = row % 10, key_b = row % 10)
    let key_a: Vec<i64> = (0..10_000).map(|i| (i % 10) as i64).collect();
    let key_b: Vec<i64> = (0..10_000).map(|i| ((i / 10) % 10) as i64).collect();
    let vals: Vec<i64> = (0..10_000).collect();
    let chunk = make_multi_key_chunk(&key_a, &key_b, &vals);
    c.bench_function("aggregate/multi_key_group_by_10k", |b| {
        b.iter(|| {
            black_box(agg.execute(black_box(vec![chunk.clone()])).unwrap());
        })
    });
}

fn bench_group_by_string_key(c: &mut Criterion) {
    let agg = PhysicalAggregate {
        group_by_cols: vec![0],
        aggregate_functions: vec!["COUNT".into()],
    };
    // String key column with 10 distinct values
    let n = 10_000;
    let mut c0 = ValueVector::new(PhysicalTypeID::String, n);
    c0.resize(n);
    let mut c1 = ValueVector::new(PhysicalTypeID::Int64, n);
    c1.resize(n);
    let labels = [
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
    ];
    for i in 0..n {
        let label = labels[i % 10];
        let bytes = label.as_bytes();
        let offset = i * 16;
        c0.data_mut()[offset] = bytes.len() as u8;
        c0.data_mut()[offset + 1..offset + 1 + bytes.len()].copy_from_slice(bytes);
        c0.set_null(i, false);
        c1.set_i64(i, i as i64);
    }
    let chunk = DataChunk::new(vec![c0, c1]);
    c.bench_function("aggregate/group_by_string_key_10k", |b| {
        b.iter(|| {
            black_box(agg.execute(black_box(vec![chunk.clone()])).unwrap());
        })
    });
}

criterion_group!(
    benches,
    bench_count_100,
    bench_count_10k,
    bench_sum_10k,
    bench_avg_10k,
    bench_multi_agg_10k,
    bench_group_by_few,
    bench_group_by_many,
    bench_group_by_multi_key,
    bench_group_by_string_key,
);
criterion_main!(benches);
