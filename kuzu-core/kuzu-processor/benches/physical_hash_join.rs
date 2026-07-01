//! PhysicalHashJoin throughput benchmarks at various build/probe sizes.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use kuzu_common::types::PhysicalTypeID;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_processor::physical_operator::{PhysicalHashJoin, PhysicalOperatorExec};

/// Create a single-column Int64 DataChunk with values start..start+count.
fn make_i64_chunk(values: &[i64]) -> DataChunk {
    let n = values.len();
    let mut v = ValueVector::new(PhysicalTypeID::Int64, n);
    v.resize(n);
    for (i, &val) in values.iter().enumerate() {
        v.set_i64(i, val);
    }
    DataChunk::new(vec![v])
}

/// Create a DataChunk with 2 Int64 columns: col_0 = id, col_1 = payload.
fn make_two_col_chunk(ids: &[i64], payload_base: i64) -> DataChunk {
    let n = ids.len();
    let mut c0 = ValueVector::new(PhysicalTypeID::Int64, n);
    c0.resize(n);
    let mut c1 = ValueVector::new(PhysicalTypeID::Int64, n);
    c1.resize(n);
    for (i, &id) in ids.iter().enumerate() {
        c0.set_i64(i, id);
        c1.set_i64(i, payload_base + id);
    }
    DataChunk::new(vec![c0, c1])
}

fn bench_join_small(c: &mut Criterion) {
    let join = PhysicalHashJoin {
        build_columns: vec![0],
        probe_columns: vec![0],
        semi_mask: None,
    };
    let build = make_i64_chunk(&(0..100).collect::<Vec<i64>>());
    let probe = make_i64_chunk(&(0..100).collect::<Vec<i64>>());
    c.bench_function("join/100_build_100_probe", |b| {
        b.iter(|| {
            let result = join.execute(black_box(vec![build.clone(), probe.clone()]));
            black_box(result.unwrap());
        })
    });
}

fn bench_join_medium(c: &mut Criterion) {
    let join = PhysicalHashJoin {
        build_columns: vec![0],
        probe_columns: vec![0],
        semi_mask: None,
    };
    let build = make_i64_chunk(&(0..1_000).collect::<Vec<i64>>());
    let probe = make_i64_chunk(&(0..1_000).collect::<Vec<i64>>());
    c.bench_function("join/1k_build_1k_probe", |b| {
        b.iter(|| {
            let result = join.execute(black_box(vec![build.clone(), probe.clone()]));
            black_box(result.unwrap());
        })
    });
}

fn bench_join_large_build(c: &mut Criterion) {
    let join = PhysicalHashJoin {
        build_columns: vec![0],
        probe_columns: vec![0],
        semi_mask: None,
    };
    let build = make_i64_chunk(&(0..10_000).collect::<Vec<i64>>());
    let probe = make_i64_chunk(&(0..100).collect::<Vec<i64>>());
    c.bench_function("join/10k_build_100_probe", |b| {
        b.iter(|| {
            let result = join.execute(black_box(vec![build.clone(), probe.clone()]));
            black_box(result.unwrap());
        })
    });
}

fn bench_join_large_probe(c: &mut Criterion) {
    let join = PhysicalHashJoin {
        build_columns: vec![0],
        probe_columns: vec![0],
        semi_mask: None,
    };
    let build = make_i64_chunk(&(0..100).collect::<Vec<i64>>());
    let probe = make_i64_chunk(&(0..10_000).collect::<Vec<i64>>());
    c.bench_function("join/100_build_10k_probe", |b| {
        b.iter(|| {
            let result = join.execute(black_box(vec![build.clone(), probe.clone()]));
            black_box(result.unwrap());
        })
    });
}

fn bench_join_multi_column(c: &mut Criterion) {
    let join = PhysicalHashJoin {
        build_columns: vec![0],
        probe_columns: vec![0],
        semi_mask: None,
    };
    let build = make_two_col_chunk(&(0..1_000).collect::<Vec<i64>>(), 1000);
    let probe = make_i64_chunk(&(0..1_000).collect::<Vec<i64>>());
    c.bench_function("join/1k_multi_col_build_1k_probe", |b| {
        b.iter(|| {
            let result = join.execute(black_box(vec![build.clone(), probe.clone()]));
            black_box(result.unwrap());
        })
    });
}

fn bench_join_no_match(c: &mut Criterion) {
    let join = PhysicalHashJoin {
        build_columns: vec![0],
        probe_columns: vec![0],
        semi_mask: None,
    };
    let build = make_i64_chunk(&(0..1_000).collect::<Vec<i64>>());
    let probe = make_i64_chunk(&(100_000..101_000).collect::<Vec<i64>>());
    c.bench_function("join/1k_no_match", |b| {
        b.iter(|| {
            let result = join.execute(black_box(vec![build.clone(), probe.clone()]));
            black_box(result.unwrap());
        })
    });
}

criterion_group!(
    benches,
    bench_join_small,
    bench_join_medium,
    bench_join_large_build,
    bench_join_large_probe,
    bench_join_multi_column,
    bench_join_no_match,
);
criterion_main!(benches);
