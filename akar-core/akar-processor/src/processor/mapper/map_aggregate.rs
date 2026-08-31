use super::ExecutionContext;
use crate::physical::order_aggregate::resolve_group_by_indices;
use crate::physical_operator::*;
use akar_common::error::ProcessorError;
use akar_common::vector::DataChunk;
use akar_parser::ast::Expression;
use akar_planner::logical_operator::LogicalOperator;

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

            // Resolve GROUP BY expressions to actual column indices using input field_names
            let field_names = current_input.first().map(|c| c.field_names.as_slice()).unwrap_or(&[]);
            let group_by_cols = if a.group_by.is_empty() {
                Vec::new()
            } else {
                resolve_group_by_indices(&a.group_by, field_names)
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
