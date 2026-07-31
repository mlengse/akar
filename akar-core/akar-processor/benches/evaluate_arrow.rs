//! Micro-benchmark comparing old (evaluate → ValueVector → mask) vs new
//! (evaluate_to_arrow → Arrow kernels → boolean_array_to_selection) paths.
//!
//! Measures the per-row Value enum boxing elimination and Arrow compute
//! kernel vectorization gains.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::{Arc, Mutex};

use akar_common::selection::SelectionVector;
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};
use akar_function::registry::FunctionRegistry;
use akar_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};
use akar_processor::expression_evaluator::ExpressionEvaluator;
use arrow::array::Array;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_registry() -> Arc<Mutex<FunctionRegistry>> {
    Arc::new(Mutex::new(FunctionRegistry::new()))
}

fn make_int64_chunk(name: &str, values: &[i64]) -> DataChunk {
    let mut v = ValueVector::new(PhysicalTypeID::Int64, values.len());
    v.resize(values.len());
    for (i, val) in values.iter().enumerate() {
        v.set_i64(i, *val);
    }
    let mut chunk = DataChunk::from_legacy(vec![v]);
    chunk.field_names.push(name.into());
    chunk
}

fn make_multi_chunk(names: &[&str], cols: &[&[i64]]) -> DataChunk {
    let fields: Vec<ValueVector> = cols
        .iter()
        .map(|vals| {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, vals.len());
            v.resize(vals.len());
            for (i, val) in vals.iter().enumerate() {
                v.set_i64(i, *val);
            }
            v
        })
        .collect();
    let mut chunk = DataChunk::from_legacy(fields);
    for name in names {
        chunk.field_names.push(name.to_string());
    }
    chunk
}

// ---------------------------------------------------------------------------
// Old path helpers (pre-Phase 2)
// ---------------------------------------------------------------------------

/// Old-style mask building from a ValueVector: iterate every row, extract Value, check truthiness.
fn mask_from_valuevector(vec: &ValueVector) -> Vec<bool> {
    let size = vec.size();
    let mut mask = Vec::with_capacity(size);
    for i in 0..size {
        if vec.is_null(i) {
            mask.push(false);
        } else if let Some(Value::Bool(b)) = vec.get_value(i) {
            mask.push(b);
        } else {
            mask.push(!vec.is_null(i));
        }
    }
    mask
}

fn old_selection_from_mask(mask: &[bool]) -> SelectionVector {
    let count = mask.iter().filter(|&v| *v).count();
    let mut sel = SelectionVector::new(count);
    for (i, &keep) in mask.iter().enumerate() {
        if keep {
            sel.push(i as u32);
        }
    }
    sel
}

// ---------------------------------------------------------------------------
// Benchmark cases
// ---------------------------------------------------------------------------

/// Constant true — old vs new.
fn bench_constant_true(c: &mut Criterion) {
    let eval = ExpressionEvaluator::new(make_registry());
    let expr = Expression::Constant(Constant::Bool(true));
    let chunk = make_int64_chunk("x", &vec![0i64; 10_000]);

    let mut group = c.benchmark_group("evaluate/constant_true_10k");
    group.measurement_time(std::time::Duration::from_secs(10));

    group.bench_function("old_evaluate", |b| {
        b.iter(|| {
            let vv = eval.evaluate(black_box(&expr), black_box(&chunk)).unwrap();
            let mask = mask_from_valuevector(black_box(&vv));
            let sel = old_selection_from_mask(black_box(&mask));
            black_box(sel);
        })
    });

    group.bench_function("new_evaluate_arrow", |b| {
        b.iter(|| {
            let av = eval.evaluate_to_arrow(black_box(&expr), black_box(&chunk)).unwrap();
            let bool_arr = av.array.as_any().downcast_ref::<arrow::array::BooleanArray>().unwrap();
            let sel = boolean_array_to_selection(black_box(bool_arr));
            black_box(sel);
        })
    });

    group.finish();
}

/// Variable reference — old vs new. The new Arrow path reads the chunk field
/// directly (an Arc clone of the ArrayRef), so this measures the dispatch
/// overhead eliminated by native Arrow arrays in DataChunk.
fn bench_variable(c: &mut Criterion) {
    let eval = ExpressionEvaluator::new(make_registry());
    let expr = Expression::Variable("x".into());
    let data: Vec<i64> = (0..10_000).collect();
    let chunk = make_int64_chunk("x", &data);

    let mut group = c.benchmark_group("evaluate/variable_10k");
    group.measurement_time(std::time::Duration::from_secs(10));

    group.bench_function("old_evaluate", |b| {
        b.iter(|| {
            let vv = eval.evaluate(black_box(&expr), black_box(&chunk)).unwrap();
            black_box(vv);
        })
    });

    group.bench_function("new_evaluate_arrow", |b| {
        b.iter(|| {
            let av = eval.evaluate_to_arrow(black_box(&expr), black_box(&chunk)).unwrap();
            black_box(av);
        })
    });

    group.finish();
}

/// Binary comparison `x > 5` — old (per-row evaluate_scalar) vs new (Arrow cmp kernel).
fn bench_cmp_gt(c: &mut Criterion) {
    let eval = ExpressionEvaluator::new(make_registry());
    let left = Box::new(Expression::Variable("x".into()));
    let right = Box::new(Expression::Constant(Constant::Integer(5)));
    let expr = Expression::BinaryOp(BinaryOp::GreaterThan, left, right);
    let data: Vec<i64> = (0..10_000).collect();
    let chunk = make_int64_chunk("x", &data);

    let mut group = c.benchmark_group("evaluate/cmp_x_gt_5_10k");
    group.measurement_time(std::time::Duration::from_secs(15));

    group.bench_function("old_evaluate", |b| {
        b.iter(|| {
            let vv = eval.evaluate(black_box(&expr), black_box(&chunk)).unwrap();
            let mask = mask_from_valuevector(black_box(&vv));
            let sel = old_selection_from_mask(black_box(&mask));
            black_box(sel);
        })
    });

    group.bench_function("new_evaluate_arrow", |b| {
        b.iter(|| {
            let av = eval.evaluate_to_arrow(black_box(&expr), black_box(&chunk)).unwrap();
            let bool_arr = av.array.as_any().downcast_ref::<arrow::array::BooleanArray>().unwrap();
            let sel = boolean_array_to_selection(black_box(bool_arr));
            black_box(sel);
        })
    });

    group.finish();
}

/// Binary arithmetic `x + y` — old (per-row evaluate_scalar) vs new (Arrow numeric kernel).
fn bench_arith_add(c: &mut Criterion) {
    let eval = ExpressionEvaluator::new(make_registry());
    let left = Box::new(Expression::Variable("x".into()));
    let right = Box::new(Expression::Variable("y".into()));
    let expr = Expression::BinaryOp(BinaryOp::Add, left, right);
    let data: Vec<i64> = (0..10_000).collect();
    let chunk = make_multi_chunk(&["x", "y"], &[&data, &data]);

    let mut group = c.benchmark_group("evaluate/arith_x_add_y_10k");
    group.measurement_time(std::time::Duration::from_secs(15));

    group.bench_function("old_evaluate", |b| {
        b.iter(|| {
            let vv = eval.evaluate(black_box(&expr), black_box(&chunk)).unwrap();
            black_box(vv);
        })
    });

    group.bench_function("new_evaluate_arrow", |b| {
        b.iter(|| {
            let av = eval.evaluate_to_arrow(black_box(&expr), black_box(&chunk)).unwrap();
            black_box(av);
        })
    });

    group.finish();
}

/// Compound `x > 5 AND y < 10` — old vs new (boolean kernel composition).
fn bench_cmp_and(c: &mut Criterion) {
    let eval = ExpressionEvaluator::new(make_registry());
    let x_gt_5 = Expression::BinaryOp(
        BinaryOp::GreaterThan,
        Box::new(Expression::Variable("x".into())),
        Box::new(Expression::Constant(Constant::Integer(5))),
    );
    let y_lt_10 = Expression::BinaryOp(
        BinaryOp::LessThan,
        Box::new(Expression::Variable("y".into())),
        Box::new(Expression::Constant(Constant::Integer(10))),
    );
    let expr = Expression::BinaryOp(BinaryOp::And, Box::new(x_gt_5), Box::new(y_lt_10));
    let data_x: Vec<i64> = (0..10_000).collect();
    let data_y: Vec<i64> = (0..10_000).rev().collect();
    let chunk = make_multi_chunk(&["x", "y"], &[&data_x, &data_y]);

    let mut group = c.benchmark_group("evaluate/cmp_and_x_gt_5_and_y_lt_10_10k");
    group.measurement_time(std::time::Duration::from_secs(15));

    group.bench_function("old_evaluate", |b| {
        b.iter(|| {
            let vv = eval.evaluate(black_box(&expr), black_box(&chunk)).unwrap();
            let mask = mask_from_valuevector(black_box(&vv));
            let sel = old_selection_from_mask(black_box(&mask));
            black_box(sel);
        })
    });

    group.bench_function("new_evaluate_arrow", |b| {
        b.iter(|| {
            let av = eval.evaluate_to_arrow(black_box(&expr), black_box(&chunk)).unwrap();
            let bool_arr = av.array.as_any().downcast_ref::<arrow::array::BooleanArray>().unwrap();
            let sel = boolean_array_to_selection(black_box(bool_arr));
            black_box(sel);
        })
    });

    group.finish();
}

/// NOT unary `NOT (x > 5)` — old vs new (Arrow boolean not kernel).
fn bench_not(c: &mut Criterion) {
    let eval = ExpressionEvaluator::new(make_registry());
    let inner = Expression::BinaryOp(
        BinaryOp::GreaterThan,
        Box::new(Expression::Variable("x".into())),
        Box::new(Expression::Constant(Constant::Integer(5))),
    );
    let expr = Expression::UnaryOp(UnaryOp::Not, Box::new(inner));
    let data: Vec<i64> = (0..10_000).collect();
    let chunk = make_int64_chunk("x", &data);

    let mut group = c.benchmark_group("evaluate/not_x_gt_5_10k");
    group.measurement_time(std::time::Duration::from_secs(15));

    group.bench_function("old_evaluate", |b| {
        b.iter(|| {
            let vv = eval.evaluate(black_box(&expr), black_box(&chunk)).unwrap();
            let mask = mask_from_valuevector(black_box(&vv));
            let sel = old_selection_from_mask(black_box(&mask));
            black_box(sel);
        })
    });

    group.bench_function("new_evaluate_arrow", |b| {
        b.iter(|| {
            let av = eval.evaluate_to_arrow(black_box(&expr), black_box(&chunk)).unwrap();
            let bool_arr = av.array.as_any().downcast_ref::<arrow::array::BooleanArray>().unwrap();
            let sel = boolean_array_to_selection(black_box(bool_arr));
            black_box(sel);
        })
    });

    group.finish();
}

/// IS NULL `x IS NULL` — old vs new (Arrow is_null kernel).
fn bench_is_null(c: &mut Criterion) {
    let eval = ExpressionEvaluator::new(make_registry());
    let expr = Expression::UnaryOp(UnaryOp::IsNull, Box::new(Expression::Variable("x".into())));
    let mut v = ValueVector::new(PhysicalTypeID::Int64, 10_000);
    v.resize(10_000);
    for i in 0..10_000 {
        if i % 3 == 0 {
            v.set_null(i, true);
        } else {
            v.set_i64(i, i as i64);
        }
    }
    let mut chunk = DataChunk::from_legacy(vec![v]);
    chunk.field_names.push("x".into());

    let mut group = c.benchmark_group("evaluate/is_null_x_10k");
    group.measurement_time(std::time::Duration::from_secs(15));

    group.bench_function("old_evaluate", |b| {
        b.iter(|| {
            let vv = eval.evaluate(black_box(&expr), black_box(&chunk)).unwrap();
            let mask = mask_from_valuevector(black_box(&vv));
            let sel = old_selection_from_mask(black_box(&mask));
            black_box(sel);
        })
    });

    group.bench_function("new_evaluate_arrow", |b| {
        b.iter(|| {
            let av = eval.evaluate_to_arrow(black_box(&expr), black_box(&chunk)).unwrap();
            let bool_arr = av.array.as_any().downcast_ref::<arrow::array::BooleanArray>().unwrap();
            let sel = boolean_array_to_selection(black_box(bool_arr));
            black_box(sel);
        })
    });

    group.finish();
}

/// Selection building alone: old (mask_to_selection) vs new (boolean_array_to_selection).
fn bench_selection_building(c: &mut Criterion) {
    use arrow::array::BooleanBuilder;

    let size = 10_000;
    // Build a BooleanArray with ~50% selectivity
    let mut builder = BooleanBuilder::with_capacity(size);
    for i in 0..size {
        builder.append_value(i % 2 == 0);
    }
    let bool_arr = builder.finish();

    // Build a Vec<bool> for the old path
    let mask: Vec<bool> = (0..size).map(|i| i % 2 == 0).collect();

    let mut group = c.benchmark_group("selection_building/10k_50pct");
    group.measurement_time(std::time::Duration::from_secs(10));

    group.bench_function("old_mask_to_selection", |b| {
        b.iter(|| {
            let sel = old_selection_from_mask(black_box(&mask));
            black_box(sel);
        })
    });

    group.bench_function("new_boolean_array_to_selection", |b| {
        b.iter(|| {
            let sel = boolean_array_to_selection(black_box(&bool_arr));
            black_box(sel);
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Shared helper (used by benchmark code and must be accessible)
// ---------------------------------------------------------------------------

fn boolean_array_to_selection(bool_arr: &arrow::array::BooleanArray) -> SelectionVector {
    let len = bool_arr.len();
    let count = (0..len).filter(|&i| bool_arr.is_valid(i) && bool_arr.value(i)).count();
    let mut sel = SelectionVector::new(count);
    for i in 0..len {
        if bool_arr.is_valid(i) && bool_arr.value(i) {
            sel.push(i as u32);
        }
    }
    sel
}

// ---------------------------------------------------------------------------
// Criterion glue
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_constant_true,
    bench_variable,
    bench_cmp_gt,
    bench_arith_add,
    bench_cmp_and,
    bench_not,
    bench_is_null,
    bench_selection_building,
);
criterion_main!(benches);
