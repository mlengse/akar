//! PhysicalFilter throughput benchmarks at various selectivities.

use criterion::{Criterion, BenchmarkGroup, criterion_group, criterion_main, measurement::WallTime};
use std::hint::black_box;
use std::sync::{Arc, Mutex};
use kuzu_common::types::PhysicalTypeID;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_function::registry::FunctionRegistry;
use kuzu_parser::ast::{BinaryOp, Constant, Expression};
use kuzu_processor::expression_evaluator::ExpressionEvaluator;
use kuzu_processor::physical_operator::{PhysicalFilter, PhysicalOperatorExec};

/// Create a DataChunk with a single Int64 column containing values 0..num_rows.
fn make_int64_chunk(num_rows: usize) -> DataChunk {
    let mut v = ValueVector::new(PhysicalTypeID::Int64, num_rows);
    v.resize(num_rows);
    for i in 0..num_rows {
        v.set_i64(i, i as i64);
    }
    DataChunk::from_legacy(vec![v])
}

/// Create a DataChunk with num_rows rows and num_cols columns of Int64.
fn make_multi_col_chunk(num_rows: usize, num_cols: usize) -> DataChunk {
    let mut fields = Vec::with_capacity(num_cols);
    for col in 0..num_cols {
        let mut v = ValueVector::new(PhysicalTypeID::Int64, num_rows);
        v.resize(num_rows);
        for i in 0..num_rows {
            v.set_i64(i, (i * (col + 1)) as i64);
        }
        fields.push(v);
    }
    DataChunk::from_legacy(fields)
}

/// Filter that passes all rows (constant true).
fn bench_filter_true(c: &mut Criterion) {
    let filter = PhysicalFilter::new(Expression::Constant(Constant::Bool(true)));
    let chunk = make_int64_chunk(10_000);
    c.bench_function("filter/pass_all_10k", |b| {
        b.iter(|| {
            let result = filter.execute(black_box(vec![chunk.clone()]));
            black_box(result.unwrap());
        })
    });
}

/// Filter that removes all rows (constant false).
fn bench_filter_false(c: &mut Criterion) {
    let filter = PhysicalFilter::new(Expression::Constant(Constant::Bool(false)));
    let chunk = make_int64_chunk(10_000);
    c.bench_function("filter/remove_all_10k", |b| {
        b.iter(|| {
            let result = filter.execute(black_box(vec![chunk.clone()]));
            black_box(result.unwrap());
        })
    });
}

/// Filter with a property expression on the first field (non-null check = passes all).
fn bench_filter_property(c: &mut Criterion) {
    let filter = PhysicalFilter::new(Expression::Variable("id".into()));
    let chunk = make_int64_chunk(10_000);
    c.bench_function("filter/property_check_10k", |b| {
        b.iter(|| {
            let result = filter.execute(black_box(vec![chunk.clone()]));
            black_box(result.unwrap());
        })
    });
}

/// Multiple chunks (batch processing).
fn bench_filter_batch(c: &mut Criterion) {
    let filter = PhysicalFilter::new(Expression::Constant(Constant::Bool(true)));
    let chunks: Vec<DataChunk> = (0..10).map(|_| make_int64_chunk(1_000)).collect();
    c.bench_function("filter/batch_10x1k_chunks", |b| {
        b.iter(|| {
            let result = filter.execute(black_box(chunks.clone()));
            black_box(result.unwrap());
        })
    });
}

/// Multi-column chunk processing.
fn bench_filter_multi_column(c: &mut Criterion) {
    let filter = PhysicalFilter::new(Expression::Variable("c0".into()));
    let chunk = make_multi_col_chunk(10_000, 8);
    c.bench_function("filter/multi_col_8_fields_10k", |b| {
        b.iter(|| {
            let result = filter.execute(black_box(vec![chunk.clone()]));
            black_box(result.unwrap());
        })
    });
}

/// Filter with age > 30 comparison (≈70% selectivity) using evaluator + Arrow kernels.
fn bench_filter_age_comparison(c: &mut Criterion) {
    let mut group: BenchmarkGroup<WallTime> = c.benchmark_group("filter");
    group.throughput(criterion::Throughput::Elements(10_000));

    let reg = Arc::new(Mutex::new(FunctionRegistry::new()));
    let eval = Arc::new(Mutex::new(ExpressionEvaluator::new(reg)));

    let expr = Expression::BinaryOp(
        BinaryOp::GreaterThan,
        Box::new(Expression::Variable("age".into())),
        Box::new(Expression::Constant(Constant::Integer(30))),
    );
    let filter = PhysicalFilter::with_evaluator(expr, eval);

    let mut v = ValueVector::new(PhysicalTypeID::Int64, 10_000);
    v.resize(10_000);
    for i in 0..10_000 {
        v.set_i64(i, (i % 101) as i64);
    }
    let chunk = DataChunk::from_legacy(vec![v]).with_names(vec!["age".into()]);

    group.bench_function("age_gt_30_10k", |b| {
        b.iter(|| {
            let result = filter.execute(black_box(vec![chunk.clone()]));
            black_box(result.unwrap());
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_filter_true,
    bench_filter_false,
    bench_filter_property,
    bench_filter_batch,
    bench_filter_multi_column,
    bench_filter_age_comparison,
);
criterion_main!(benches);
