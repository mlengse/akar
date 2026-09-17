use akar_parser::ast::Expression;
use akar_planner::logical_operator::LogicalOperator;
use std::collections::HashSet;

/// Column references collected from the operators that consume an Extend's
/// output (the tail of the flat plan after the Extend).
///
/// PhysicalExtend by default duplicates EVERY input/rel/dest column for each
/// produced row. For a `Memory`/`Connected` full scan that is ~46 columns ×
/// 53k edges — including per-row copies of node embedding List values — which
/// is the dominant memory blow-up (1.1 GB OOM). Pruning the output down to the
/// columns actually referenced downstream keeps correctness while making full
/// scans of big relationship tables cheap.
///
/// Collection is conservative: any operator in the tail we cannot reason about
/// (joins, unions, flatten, partitioner, recursive extend, optional extend,
/// writes, ...) disables pruning entirely (`None`). Pruning only ever drops
/// columns — name-based consumers in the whitelist are unaffected, and we
/// always retain `{var}.id` / `{var}._id` identity columns so later extends,
/// joins and aggregates still work.
#[derive(Debug, Clone, Default)]
pub struct ExtendPrune {
    /// Exact qualified field names referenced downstream, e.g. `a.id`,
    /// `b.id`, `r.weight`.
    pub fields: HashSet<String>,
    /// Bare variables referenced as a whole (e.g. `RETURN a`), meaning keep
    /// ALL columns prefixed `a.` plus the bare column `a` if present.
    /// Over-conservative on purpose: `Variable` may also be an output alias.
    pub whole_vars: HashSet<String>,
}

/// Collect referenced columns from a plan tail. Returns `None` when the tail
/// contains an operator whose column consumption cannot be proven name-based
/// (disables pruning for that Extend).
pub fn collect_extend_prune(ops: &[LogicalOperator]) -> Option<ExtendPrune> {
    let mut p = ExtendPrune::default();
    for op in ops {
        match op {
            LogicalOperator::Projection(proj) => {
                for e in &proj.expressions {
                    walk_expr(&e.expression, &mut p)?;
                }
            }
            LogicalOperator::Filter(f) => walk_expr(&f.expression, &mut p)?,
            LogicalOperator::OrderBy(o) => {
                for (expr, _asc) in &o.sort_keys {
                    walk_expr(expr, &mut p)?;
                }
            }
            LogicalOperator::TopK(t) => {
                for (expr, _asc) in &t.sort_keys {
                    walk_expr(expr, &mut p)?;
                }
            }
            LogicalOperator::Aggregate(a) => {
                for g in &a.group_by {
                    walk_expr(g, &mut p)?;
                }
                for (_name, args) in &a.aggregates {
                    for arg in args {
                        // `count(*)` collapses to one row and references no
                        // specific column — unlike a bare `RETURN *`.
                        if matches!(arg, Expression::Star) {
                            continue;
                        }
                        walk_expr(arg, &mut p)?;
                    }
                }
            }
            LogicalOperator::Unwind(u) => walk_expr(&u.expression, &mut p)?,
            LogicalOperator::Limit(_) | LogicalOperator::Skip(_) | LogicalOperator::EmptyResult(_) => {}
            // Anything else (joins, unions, flatten, partitioner, optional /
            // recursive / further extends, scan nodes, writes, DDL) may consume
            // columns positionally or by an unknown relationship — bail.
            _ => return None,
        }
    }
    Some(p)
}

/// Walk an expression, recording column references. Returns `None` when the
/// expression requires columns we cannot name (e.g. `RETURN *`).
fn walk_expr(e: &Expression, p: &mut ExtendPrune) -> Option<()> {
    match e {
        Expression::Constant(_) | Expression::Parameter(_) => {}
        Expression::Variable(v) => {
            p.whole_vars.insert(v.clone());
        }
        Expression::PropertyAccess(base, name) => match base.as_ref() {
            Expression::Variable(var) => {
                p.fields.insert(format!("{}.{}", var, name));
            }
            // Property on a non-variable base (function result, nested
            // access) — walk the base so its variable refs are kept.
            other => walk_expr(other, p)?,
        },
        Expression::FunctionCall(_, args) => {
            let mut saw_star = false;
            for a in args {
                // `count(*)` / `collect(*)` reference no specific column.
                if matches!(a, Expression::Star) {
                    saw_star = true;
                    continue;
                }
                walk_expr(a, p)?;
            }
            // A function with ONLY `*` and no other refs is fine to prune;
            // `saw_star` alone needs no columns either.
            let _ = saw_star;
        }
        Expression::BinaryOp(_, l, r) => {
            walk_expr(l, p)?;
            walk_expr(r, p)?;
        }
        Expression::UnaryOp(_, x) => walk_expr(x, p)?,
        Expression::List(xs) => {
            for x in xs {
                walk_expr(x, p)?;
            }
        }
        Expression::Map(kvs) => {
            for (_k, v) in kvs {
                walk_expr(v, p)?;
            }
        }
        Expression::ExistsSubquery(_) => return None,
        Expression::Case(c) => {
            if let Some(subj) = &c.subject {
                walk_expr(subj, p)?;
            }
            for alt in &c.alternatives {
                walk_expr(&alt.when, p)?;
                walk_expr(&alt.then, p)?;
            }
            if let Some(els) = &c.else_expr {
                walk_expr(els, p)?;
            }
        }
        // Bare `RETURN *` — every column is needed.
        Expression::Star => return None,
        Expression::ListPredicate { list, predicate, .. } => {
            walk_expr(list, p)?;
            walk_expr(predicate, p)?;
        }
        Expression::Lambda { body, .. } => walk_expr(body, p)?,
    }
    Some(())
}

/// Decide whether a qualified/bare column name must be kept.
///
/// Always keeps identity columns (`{var}.id`, `{var}._id`) — extends, joins,
/// aggregates and OCC resolution depend on them. Everything else is kept only
/// when referenced exactly, or when its whole variable is referenced.
pub fn keep_column(p: &ExtendPrune, name: &str) -> bool {
    if p.fields.contains(name) {
        return true;
    }
    if let Some(dot) = name.find('.') {
        let var = &name[..dot];
        if p.whole_vars.contains(var) {
            return true;
        }
        let prop = &name[dot + 1..];
        // Identity columns are cheap and always required.
        prop == "_id" || prop == "id"
    } else {
        // Bare, unprefixed column (e.g. `distance` from a vector scan or a
        // raw `_id`): keep unless we know it is not needed. Can only be
        // needed via a Variable ref, which lands in whole_vars; keep by
        // default since bare names are rare in extend outputs.
        p.whole_vars.contains(name) || name == "_id"
    }
}
