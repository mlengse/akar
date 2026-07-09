//! PhysicalScan throughput benchmarks at various table sizes.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use kuzu_common::types::{LogicalTypeID, Value};
use kuzu_processor::physical_operator::{PhysicalOperatorExec, PhysicalScan};
use kuzu_common::enums::CompressionType;
use kuzu_storage::table::ColumnDefinition;

/// Generate a Vec<Vec<Value>> table with num_rows rows and the given schema.
fn generate_table(num_rows: usize, columns: &[ColumnDefinition]) -> Vec<Vec<Value>> {
    let num_cols = columns.len();
    let mut data: Vec<Vec<Value>> = (0..num_cols).map(|_| Vec::with_capacity(num_rows)).collect();

    for row in 0..num_rows {
        for (col, def) in columns.iter().enumerate() {
            let val = match def.logical_type {
                LogicalTypeID::Int64 => Value::Int64(row as i64),
                LogicalTypeID::Double => Value::Double(row as f64 * 1.5),
                LogicalTypeID::Bool => Value::Bool(row % 2 == 0),
                LogicalTypeID::String => Value::String(format!("val_{row}")),
                _ => Value::Int64(row as i64),
            };
            data[col].push(val);
        }
    }
    data
}

fn make_scan(table_data: Vec<Vec<Value>>, columns: Vec<ColumnDefinition>) -> PhysicalScan {
    let num_rows = table_data.first().map(|c| c.len()).unwrap_or(0);
    let mut scan = PhysicalScan::new("BenchTable".into(), 0, num_rows.max(1) as u64);
    scan = scan.with_data(table_data, columns);
    scan
}

fn schema() -> Vec<ColumnDefinition> {
    vec![
        ColumnDefinition {
            name: String::from("id"),
            logical_type: LogicalTypeID::Int64,
            is_primary_key: true,
            compression: CompressionType::Uncompressed,
        },
        ColumnDefinition {
            name: String::from("name"),
            logical_type: LogicalTypeID::String,
            is_primary_key: false,
            compression: CompressionType::Uncompressed,
        },
        ColumnDefinition {
            name: String::from("score"),
            logical_type: LogicalTypeID::Double,
            is_primary_key: false,
            compression: CompressionType::Uncompressed,
        },
        ColumnDefinition {
            name: String::from("active"),
            logical_type: LogicalTypeID::Bool,
            is_primary_key: false,
            compression: CompressionType::Uncompressed,
        },
    ]
}

fn bench_scan_100(c: &mut Criterion) {
    let columns = schema();
    let data = generate_table(100, &columns);
    let scan = make_scan(data, columns);
    c.bench_function("scan/100_rows", |b| {
        b.iter(|| {
            let result = scan.execute(black_box(vec![]));
            black_box(result.unwrap());
        })
    });
}

fn bench_scan_1k(c: &mut Criterion) {
    let columns = schema();
    let data = generate_table(1_000, &columns);
    let scan = make_scan(data, columns);
    c.bench_function("scan/1k_rows", |b| {
        b.iter(|| {
            let result = scan.execute(black_box(vec![]));
            black_box(result.unwrap());
        })
    });
}

fn bench_scan_10k(c: &mut Criterion) {
    let columns = schema();
    let data = generate_table(10_000, &columns);
    let scan = make_scan(data, columns);
    c.bench_function("scan/10k_rows", |b| {
        b.iter(|| {
            let result = scan.execute(black_box(vec![]));
            black_box(result.unwrap());
        })
    });
}

fn bench_scan_selective_columns(c: &mut Criterion) {
    let columns = schema();
    let data = generate_table(10_000, &columns);
    let mut scan = make_scan(data, columns);
    scan = scan.with_columns(vec![0, 2]); // only id + score
    c.bench_function("scan/10k_selective_2_of_4_cols", |b| {
        b.iter(|| {
            let result = scan.execute(black_box(vec![]));
            black_box(result.unwrap());
        })
    });
}

criterion_group!(
    benches,
    bench_scan_100,
    bench_scan_1k,
    bench_scan_10k,
    bench_scan_selective_columns,
);
criterion_main!(benches);
