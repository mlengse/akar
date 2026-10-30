use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use akar_parser::ast::{Constant, Expression};
use ahash::AHashMap;

fn lookup_linear<'a>(
    cols: &'a [String],
    props: &'a [(String, Expression)],
) -> Vec<Option<&'a Expression>> {
    let mut res = Vec::with_capacity(cols.len());
    for col in cols {
        if let Some((_, expr)) = props.iter().find(|(n, _)| n == col) {
            res.push(Some(expr));
        } else {
            res.push(None);
        }
    }
    res
}

fn lookup_indexed<'a>(
    cols: &'a [String],
    props: &'a [(String, Expression)],
) -> Vec<Option<&'a Expression>> {
    let prop_map: AHashMap<&str, &Expression> =
        props.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let mut res = Vec::with_capacity(cols.len());
    for col in cols {
        if let Some(expr) = prop_map.get(col.as_str()) {
            res.push(Some(*expr));
        } else {
            res.push(None);
        }
    }
    res
}

fn bench_merge_property_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("merge_property_lookup");

    for &(num_cols, num_props) in &[(10, 5), (50, 25), (100, 50)] {
        let cols: Vec<String> = (0..num_cols).map(|i| format!("col_{i}")).collect();
        let props: Vec<(String, Expression)> = (0..num_props)
            .map(|i| {
                let name = format!("col_{}", i * 2);
                let expr = Expression::Constant(Constant::Integer(i as i64));
                (name, expr)
            })
            .collect();

        group.bench_with_input(
            BenchmarkId::new("linear", format!("{num_cols}cols_{num_props}props")),
            &(num_cols, num_props),
            |b, _| b.iter(|| lookup_linear(&cols, &props)),
        );

        group.bench_with_input(
            BenchmarkId::new("indexed", format!("{num_cols}cols_{num_props}props")),
            &(num_cols, num_props),
            |b, _| b.iter(|| lookup_indexed(&cols, &props)),
        );
    }
    group.finish();
}

criterion_group!(benches, bench_merge_property_lookup);
criterion_main!(benches);
