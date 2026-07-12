use super::ExecutionContext;
use crate::physical_operator::*;
use kuzu_common::vector::DataChunk;
use kuzu_planner::logical_operator::LogicalOperator;

pub fn map_and_execute_aggregate(
    op: &LogicalOperator,
    current_input: Vec<DataChunk>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, String> {
    match op {
        LogicalOperator::Aggregate(a) => {
            let funcs: Vec<kuzu_function::AggregateFunction> = a
                .aggregates
                .iter()
                .map(|(n, _)| crate::physical::order_aggregate::parse_aggregate_function(n))
                .collect();
            let group_by_cols = if a.group_by.is_empty() {
                Vec::new()
            } else {
                (0..a.group_by.len() as u32).collect()
            };

            let shared_state = std::sync::Arc::new(
                crate::physical::order_aggregate::SharedAggregateState::new(funcs, group_by_cols),
            );

            let agg_scan = crate::physical::order_aggregate::PhysicalAggregateScan {
                shared_state: shared_state.clone(),
            };
            let agg_finalize = crate::physical::order_aggregate::PhysicalAggregateFinalize {
                shared_state,
            };

            // Phase 1: Scan and accumulate (returns empty chunk in sequential push-down)
            let _ = agg_scan.execute(current_input)?;

            // Phase 2: Finalize and yield grouped chunks
            let result = agg_finalize.execute(vec![])?;

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
        _ => Err(format!("Not an aggregate operator: {:?}", op)),
    }
}
