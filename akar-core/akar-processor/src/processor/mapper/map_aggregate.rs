use super::ExecutionContext;
use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical::order_aggregate::resolve_group_by_indices;
use crate::physical_operator::*;
use akar_common::error::ProcessorError;
use akar_common::vector::DataChunk;
use akar_parser::ast::Expression;
use akar_planner::logical_operator::LogicalOperator;

/// Prefix of the synthetic trailing columns that carry pre-evaluated computed
/// aggregate arguments (P126 / F13).
const COMPUTED_AGG_COL_PREFIX: &str = "__akar_agg_arg_";

pub fn map_and_execute_aggregate(
    op: &LogicalOperator,
    current_input: Vec<DataChunk>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, ProcessorError> {
    match op {
        LogicalOperator::Aggregate(a) => {
            // P88 DISTINCT aggregates: the parser encodes `COUNT(DISTINCT x)`
            // as function name `COUNT_DISTINCT`; split back into the base
            // function + a per-function distinct flag.
            let mut funcs: Vec<akar_function::AggregateFunction> = Vec::with_capacity(a.aggregates.len());
            let mut distinct_flags: Vec<bool> = Vec::with_capacity(a.aggregates.len());
            for (n, args) in &a.aggregates {
                let (base, distinct) = split_distinct_name(n);
                distinct_flags.push(distinct);
                // Detect COUNT(*) from Star arg: override name to get CountStar
                let effective_name = if base == "COUNT" && args.iter().any(|e| matches!(e, Expression::Star)) {
                    "COUNT(*)"
                } else {
                    base
                };
                funcs.push(crate::physical::order_aggregate::parse_aggregate_function(
                    effective_name,
                ));
            }
            let agg_expressions: Vec<Vec<Expression>> = a.aggregates.iter().map(|(_, args)| args.clone()).collect();

            // P126 (F13): an aggregate argument that is not a plain column
            // (`SUM(s.bridges * 2)`, `SUM(abs(x))`, `SUM(CASE ...)`) resolves to
            // no column index at all, so the physical aggregate receives no
            // values and silently returns NULL. Evaluate every such argument
            // per chunk, append the result as a trailing column, and rewrite the
            // argument into a reference to that column. The physical aggregate
            // then sees a plain column, so every existing fast path (COUNT,
            // Sum/Min/Max/Avg/Collect and their DISTINCT / GROUP BY variants)
            // stays on its normal route. Nothing happens at all — no chunk copy,
            // no evaluator — when no aggregate argument is computed.
            let mut computed_agg_args: Vec<Expression> = Vec::new();
            let agg_expressions: Vec<Vec<Expression>> = agg_expressions
                .into_iter()
                .map(|args| match computed_aggregate_argument(&args) {
                    Some(expr) => {
                        let name = computed_agg_col_name(computed_agg_args.len());
                        computed_agg_args.push(expr.clone());
                        vec![Expression::Variable(name)]
                    }
                    None => args,
                })
                .collect();

            // Resolve GROUP BY expressions to actual column indices using input field_names
            let field_names = current_input.first().map(|c| c.field_names.as_slice()).unwrap_or(&[]);
            let group_by_cols = if a.group_by.is_empty() {
                Vec::new()
            } else {
                resolve_group_by_indices(&a.group_by, field_names)
            };

            // The synthetic argument columns are appended *after* the group-by
            // indices were resolved, so those indices keep pointing at the
            // original columns.
            let current_input = if computed_agg_args.is_empty() {
                current_input
            } else {
                append_computed_aggregate_columns(current_input, &computed_agg_args, ctx)?
            };

            let shared_state = std::sync::Arc::new(crate::physical::order_aggregate::SharedAggregateState::new(
                funcs,
                group_by_cols,
                agg_expressions,
                distinct_flags,
            ));

            let agg_scan = crate::physical::order_aggregate::PhysicalAggregateScan {
                shared_state: shared_state.clone(),
            };
            let agg_finalize = crate::physical::order_aggregate::PhysicalAggregateFinalize { shared_state };

            // Phase 1: Scan and accumulate (returns empty chunk in sequential push-down)
            let _ = agg_scan.execute(current_input)?;

            // Phase 2: Finalize and yield grouped chunks
            let result = agg_finalize.execute(vec![])?;

            // P52.56: the aggregate output chunks carried no field_names, so
            // result columns were positional-only and an alias (`AS cnt`) never
            // reached the result. Propagate group-by variable names + aggregate
            // function names onto the output chunks. A Projection above the
            // aggregate still resolves positionally when a name doesn't match,
            // so this is safe.
            let names = aggregate_field_names(a);
            let result: Vec<DataChunk> = result
                .into_iter()
                .map(|chunk| chunk.with_names(names.clone()))
                .collect();

            Ok(result)
        }
        LogicalOperator::CountRelTable(crt) => {
            let physical = PhysicalCountRelTable {
                table_name: crt.table_name.clone(),
                table_id: crt.table_id,
                table_catalog: ctx.table_catalog.clone(),
            };
            let result = physical.execute(vec![])?;
            Ok(result)
        }
        _ => Err(format!("Not an aggregate operator: {:?}", op).into()),
    }
}

/// Name of the synthetic trailing column holding the pre-evaluated value of the
/// `idx`-th computed aggregate argument (P126 / F13).
fn computed_agg_col_name(idx: usize) -> String {
    format!("{COMPUTED_AGG_COL_PREFIX}{idx}")
}

/// Return the aggregate argument that must be evaluated before aggregation.
///
/// `resolve_agg_col_indices` maps an aggregate argument to an input column
/// index and understands exactly three shapes: `Variable` and `PropertyAccess`
/// (resolved by name) and `Star` (`COUNT(*)`, which deliberately needs no
/// column). Every other shape — `SUM(s.bridges * 2)`, `SUM(abs(x))`,
/// `SUM(CASE ...)` — previously fell through to "no column needed" and produced
/// NULL (F13). Aggregate functions take a single argument
/// (`parse_aggregate_function`), so the match below is unambiguous.
///
/// `Variable` / `PropertyAccess` are left alone even when they fail to resolve
/// to a column, because their "no column" outcome predates F13 and is handled by
/// its own fallback (`COUNT` counts all active rows).
fn computed_aggregate_argument(args: &[Expression]) -> Option<&Expression> {
    match args {
        [arg]
            if !matches!(
                arg,
                Expression::Variable(_) | Expression::PropertyAccess(_, _) | Expression::Star
            ) =>
        {
            Some(arg)
        }
        _ => None,
    }
}

/// Append the pre-evaluated values of computed aggregate arguments as trailing
/// columns (P126 / F13), named so that `resolve_agg_col_indices` maps the
/// rewritten argument back to them.
fn append_computed_aggregate_columns(
    input: Vec<DataChunk>,
    computed: &[Expression],
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, ProcessorError> {
    let registry = ctx
        .function_registry
        .clone()
        .ok_or_else(|| "No function registry available for a computed aggregate argument".to_string())?;
    let mut eval = ExpressionEvaluator::new(registry);
    if let Some(ref seq_fn) = ctx.sequence_fn {
        eval = eval.with_sequence_fn(seq_fn.clone());
    }
    if let Some(ref subquery_fn) = ctx.subquery_fn {
        eval = eval.with_subquery_fn(subquery_fn.clone());
    }

    let mut augmented = Vec::with_capacity(input.len());
    for chunk in input {
        let mut fields = chunk.fields.clone();
        let mut field_types = chunk.field_types.clone();
        let mut field_names = chunk.field_names.clone();
        for (i, expr) in computed.iter().enumerate() {
            // `evaluate_arrow` yields one slot per physical row of the chunk, so
            // the appended column stays aligned with the existing ones.
            let vv = eval.evaluate_arrow(expr, &chunk)?;
            fields.push(vv.array);
            field_types.push(vv.physical_type);
            field_names.push(computed_agg_col_name(i));
        }
        augmented.push(DataChunk {
            fields,
            field_types,
            size: chunk.size,
            field_names,
            sel_vector: chunk.sel_vector.clone(),
        });
    }
    Ok(augmented)
}

/// Split an aggregate name that may carry the parser's DISTINCT encoding
/// (P88): `COUNT_DISTINCT` → (`COUNT`, true). Aggregate names reach the
/// processor uppercased by aggregate_detection.
fn split_distinct_name(name: &str) -> (&str, bool) {
    match name.strip_suffix("_DISTINCT") {
        Some(base) => (base, true),
        None => (name, false),
    }
}

/// Build output field names for an aggregate result: the group-by variable
/// names followed by the aggregate function names (P52.56). Group-by naming
/// mirrors `expression_field_name` in map_projection.rs so the projection above
/// the aggregate resolves columns by name (P53.16).
fn aggregate_field_names(a: &akar_planner::logical_operator::LogicalAggregate) -> Vec<String> {
    let mut names: Vec<String> = a
        .group_by
        .iter()
        .map(|e| match e {
            Expression::Variable(v) => v.clone(),
            Expression::PropertyAccess(obj, prop) => {
                if let Expression::Variable(var) = &**obj {
                    format!("{var}.{prop}")
                } else {
                    prop.clone()
                }
            }
            other => format!("{other:?}"),
        })
        .collect();
    for (fname, args) in &a.aggregates {
        let (fname, _) = split_distinct_name(fname);
        let effective = if fname == "COUNT" && args.iter().any(|e| matches!(e, Expression::Star)) {
            "COUNT(*)".to_string()
        } else if args.len() == 1 {
            match &args[0] {
                Expression::Variable(v) => format!("{fname}({v})"),
                Expression::PropertyAccess(obj, prop) => {
                    if let Expression::Variable(base) = &**obj {
                        format!("{fname}({base}.{prop})")
                    } else {
                        format!("{fname}({prop})")
                    }
                }
                Expression::Star => format!("{fname}(*)"),
                _ => fname.to_string(),
            }
        } else {
            fname.to_string()
        };
        names.push(effective);
    }
    names
}
