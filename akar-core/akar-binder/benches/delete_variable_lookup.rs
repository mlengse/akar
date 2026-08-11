//! Benchmark: resolving DELETE expression variables against the in-scope
//! `Vec<BoundVariable>` list.
//!
//! The historical implementation scanned the whole variable list with
//! `variables.iter().find(|v| v.name == *var_name)` for every DELETE
//! expression — O(N*M) in the worst case (N bound variables, M delete
//! expressions). The optimized version indexes variables by name in a
//! HashMap once (O(N)) and then resolves each expression in O(1).
//!
//! Scenarios:
//!   - `typical_small`   : N=20 vars, M=3 deletes (a normal MATCH+DELETE).
//!   - `wide_scope`      : N=10k vars, M=3 deletes (huge scope, tiny delete).
//!   - `large_worst_case`: N=10k vars, M=10k deletes (adversarial).

use akar_binder::bound_statement::{BoundDeleteItem, BoundVariable};
use akar_parser::ast::Expression;
use criterion::{BenchmarkGroup, Criterion, criterion_group, criterion_main, measurement::WallTime};
use std::collections::HashMap;
use std::hint::black_box;

/// Historical implementation: linear scan per expression (O(N*M)).
fn resolve_linear(variables: &[BoundVariable], exprs: &[Expression]) -> Result<Vec<BoundDeleteItem>, String> {
    let mut items = Vec::new();
    for expr in exprs {
        match expr {
            Expression::Variable(var_name) => {
                let var = variables
                    .iter()
                    .find(|v| v.name == *var_name)
                    .ok_or_else(|| format!("Variable '{}' not found in scope for DELETE", var_name))?;
                items.push(BoundDeleteItem {
                    expression: expr.clone(),
                    table_name: var.label.clone().unwrap_or_default(),
                    table_id: var.table_id,
                    primary_key_column: String::new(),
                    is_node: var.is_node,
                });
            }
            _ => return Err(format!("DELETE only supports variable references, got: {:?}", expr)),
        }
    }
    Ok(items)
}

/// Optimized implementation: index variables by name once (O(N)), then O(1)
/// lookups per expression. First occurrence wins to match the scan semantics.
fn resolve_hash(variables: &[BoundVariable], exprs: &[Expression]) -> Result<Vec<BoundDeleteItem>, String> {
    let mut by_name: HashMap<&str, &BoundVariable> = HashMap::with_capacity(variables.len());
    for v in variables {
        by_name.entry(v.name.as_str()).or_insert(v);
    }
    let mut items = Vec::new();
    for expr in exprs {
        match expr {
            Expression::Variable(var_name) => {
                let var = by_name
                    .get(var_name.as_str())
                    .ok_or_else(|| format!("Variable '{}' not found in scope for DELETE", var_name))?;
                items.push(BoundDeleteItem {
                    expression: expr.clone(),
                    table_name: var.label.clone().unwrap_or_default(),
                    table_id: var.table_id,
                    primary_key_column: String::new(),
                    is_node: var.is_node,
                });
            }
            _ => return Err(format!("DELETE only supports variable references, got: {:?}", expr)),
        }
    }
    Ok(items)
}

/// The shipped adaptive implementation: linear scan for the common case, the
/// HashMap index only past the measured break-even (M >= 8 deletes AND N >= 256
/// variables in scope).
fn resolve_adaptive(variables: &[BoundVariable], exprs: &[Expression]) -> Result<Vec<BoundDeleteItem>, String> {
    let use_index = exprs.len() >= 8 && variables.len() >= 256;
    let index = if use_index {
        let mut map: HashMap<&str, &BoundVariable> = HashMap::with_capacity(variables.len());
        for v in variables {
            map.entry(v.name.as_str()).or_insert(v);
        }
        Some(map)
    } else {
        None
    };

    let mut items = Vec::new();
    for expr in exprs {
        match expr {
            Expression::Variable(var_name) => {
                let var = match &index {
                    Some(map) => map.get(var_name.as_str()).copied(),
                    None => variables.iter().find(|v| v.name == *var_name),
                }
                .ok_or_else(|| format!("Variable '{}' not found in scope for DELETE", var_name))?;
                items.push(BoundDeleteItem {
                    expression: expr.clone(),
                    table_name: var.label.clone().unwrap_or_default(),
                    table_id: var.table_id,
                    primary_key_column: String::new(),
                    is_node: var.is_node,
                });
            }
            _ => return Err(format!("DELETE only supports variable references, got: {:?}", expr)),
        }
    }
    Ok(items)
}

fn make_variables(n: usize) -> Vec<BoundVariable> {
    (0..n)
        .map(|i| BoundVariable {
            name: format!("v{i}"),
            table_id: i as u64,
            label: Some(format!("Table{i}")),
            is_node: i % 2 == 0,
        })
        .collect()
}

/// M delete expressions that all reference the *last* variable in scope —
/// the worst case for the linear scan.
fn make_exprs_last_var(m: usize, n: usize) -> Vec<Expression> {
    let name = format!("v{}", n - 1);
    vec![Expression::Variable(name); m]
}

fn bench_delete_lookup(c: &mut Criterion) {
    let mut group: BenchmarkGroup<WallTime> = c.benchmark_group("delete_variable_lookup");

    for (scenario, n, m) in [
        ("typical_small", 20usize, 3usize),
        ("wide_scope", 10_000, 3),
        ("break_even_probe", 100, 8),
        ("mid_scope", 1_000, 8),
        ("large_worst_case", 10_000, 10_000),
    ] {
        let variables = make_variables(n);
        let exprs = make_exprs_last_var(m, n);

        group.bench_function(format!("{scenario}/linear_scan_n{n}_m{m}"), |b| {
            b.iter(|| black_box(resolve_linear(black_box(&variables), black_box(&exprs)).unwrap()))
        });
        group.bench_function(format!("{scenario}/hash_map_n{n}_m{m}"), |b| {
            b.iter(|| black_box(resolve_hash(black_box(&variables), black_box(&exprs)).unwrap()))
        });
        group.bench_function(format!("{scenario}/adaptive_n{n}_m{m}"), |b| {
            b.iter(|| black_box(resolve_adaptive(black_box(&variables), black_box(&exprs)).unwrap()))
        });
    }

    group.finish();
}

criterion_group!(benches, bench_delete_lookup);
criterion_main!(benches);
