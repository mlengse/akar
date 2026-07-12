pub mod map_aggregate;
pub mod map_ddl;
pub mod map_join;
pub mod map_projection;
pub mod map_scan;
pub mod map_update;

use crate::physical_operator::NodeSemiMask;
use crate::physical::types::PhysicalOperatorExec;
use crate::processor::QueryProcessor;
use kuzu_common::vector::DataChunk;
use kuzu_function::registry::FunctionRegistry;
use kuzu_planner::logical_operator::LogicalOperator;
use kuzu_storage::table::TableCatalog;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::{SequenceFn, StandaloneCallHandler, SubqueryFn};

/// Shared state threaded through the mapper functions
pub struct ExecutionContext<'a, 'p> {
    pub processor: &'p QueryProcessor,
    pub sip_masks: &'a mut HashMap<u64, NodeSemiMask>,
    pub function_registry: Option<Arc<Mutex<FunctionRegistry>>>,
    pub table_catalog: Option<Arc<TableCatalog>>,
    pub vfs: Option<Arc<kuzu_common::file_system::VirtualFileSystemRegistry>>,
    pub standalone_call_handler: Option<Arc<dyn StandaloneCallHandler>>,
    pub sequence_fn: Option<SequenceFn>,
    pub subquery_fn: Option<SubqueryFn>,
}

impl<'a, 'p> ExecutionContext<'a, 'p> {
    pub fn execute_children(&mut self, operators: &[LogicalOperator]) -> Result<Vec<DataChunk>, String> {
        self.processor.execute_internal(operators, self.sip_masks)
    }

    /// Resolve table data and column definitions for a scan node.
    pub fn resolve_scan_data<'b>(
        &self,
        table_name: &str,
        predicate: Option<(usize, &'b str, &'b kuzu_common::types::Value)>,
    ) -> (Option<Vec<Vec<kuzu_common::types::Value>>>, Vec<kuzu_storage::table::ColumnDefinition>, u64) {
        if let Some(ref tc) = self.table_catalog {
            // Try node table first
            if let Some(node_table) = tc.get_node_table_by_name(table_name) {
                let num_rows = node_table.num_rows;
                if num_rows > 0 {
                    return (
                        Some(node_table.to_column_major_data_with_predicate(predicate)),
                        node_table.columns.clone(),
                        num_rows,
                    );
                }
            }
            // Try rel table
            if let Some(rel_table) = tc.get_rel_table_by_name(table_name) {
                let num_rows = rel_table.num_rows;
                if num_rows > 0 {
                    return (
                        Some(rel_table.to_column_major_data()), // Rel tables don't have zone map yet
                        rel_table.columns.clone(),
                        num_rows,
                    );
                }
            }
        }
        (None, Vec::new(), 0)
    }
}

pub struct PlanMapper;

impl PlanMapper {
    pub fn map_and_execute(
        op: &LogicalOperator,
        next_op: Option<&LogicalOperator>,
        current_input: Vec<DataChunk>,
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<DataChunk>, String> {
        match op {
            // Scans
            LogicalOperator::ScanNode(s) => map_scan::map_and_execute_scan_node(s, next_op, current_input, ctx),
            LogicalOperator::ScanRel(_)
            | LogicalOperator::VectorSimilarityScan(_)
            | LogicalOperator::ArtIndexRangeScan(_)
            | LogicalOperator::IndexLookup(_)
            | LogicalOperator::ExpressionsScan(_)
            | LogicalOperator::PathPropertyProbe(_) => map_scan::map_and_execute_scan(op, current_input, ctx),

            // Joins
            LogicalOperator::HashJoin(_)
            | LogicalOperator::SemiJoin(_)
            | LogicalOperator::AntiJoin(_)
            | LogicalOperator::Intersect(_)
            | LogicalOperator::CrossProduct(_)
            | LogicalOperator::OptionalMatch(_)
            | LogicalOperator::RecursiveExtend(_) => map_join::map_and_execute_join(op, current_input, ctx),

            // Aggregates
            LogicalOperator::Aggregate(_) | LogicalOperator::CountRelTable(_) => {
                map_aggregate::map_and_execute_aggregate(op, current_input, ctx)
            }

            // Updates
            LogicalOperator::Set(_)
            | LogicalOperator::Delete(_)
            | LogicalOperator::CreateNode(_)
            | LogicalOperator::CreateRel(_)
            | LogicalOperator::Merge(_)
            | LogicalOperator::Extend(_)
            | LogicalOperator::BatchInsert(_)
            | LogicalOperator::Insert(_)
            | LogicalOperator::CopyFrom(_) => map_update::map_and_execute_update(op, current_input, ctx),

            // Union
            LogicalOperator::Union(u) => {
                use crate::processor::union_helpers::{flatten_union_child, merge_union_chunks};
                let left_ops = flatten_union_child(&u.left);
                let right_ops = flatten_union_child(&u.right);
                let left = ctx.execute_children(&left_ops)?;
                let right = ctx.execute_children(&right_ops)?;
                merge_union_chunks(left, right, u.all)
            }

            // Projections & Filters
            LogicalOperator::Projection(_)
            | LogicalOperator::Filter(_)
            | LogicalOperator::TopK(_)
            | LogicalOperator::OrderBy(_)
            | LogicalOperator::Limit(_)
            | LogicalOperator::Flatten(_)
            | LogicalOperator::SemiMasker(_)
            | LogicalOperator::Unwind(_)
            | LogicalOperator::Partitioner(_) => {
                map_projection::map_and_execute_projection(op, current_input, ctx)
            }

            // DDL & Others
            LogicalOperator::CreateNodeTable(_)
            | LogicalOperator::CreateRelTable(_)
            | LogicalOperator::DropTable(_)
            | LogicalOperator::AlterTable(_)
            | LogicalOperator::CreateIndex(_)
            | LogicalOperator::DropIndex(_)
            | LogicalOperator::CreateVectorIndex(_)
            | LogicalOperator::CreateSequence(_)
            | LogicalOperator::DropSequence(_)
            | LogicalOperator::CreateDml(_)
            | LogicalOperator::ExportDatabase(_)
            | LogicalOperator::ImportDatabase(_)
            | LogicalOperator::CreateFtsIndex(_)
            | LogicalOperator::FtsScan(_)
            | LogicalOperator::EmptyResult(_)
            | LogicalOperator::MultiplicityReducer(_)
            | LogicalOperator::Skip(_)
            | LogicalOperator::ExtensionClause(_)
            | LogicalOperator::StandaloneCall(_)
            | LogicalOperator::TableFunctionCall(_)
            | LogicalOperator::Foreach(_)
            | LogicalOperator::Explain(_) => map_ddl::map_and_execute_ddl(op, current_input, ctx),

            LogicalOperator::Accumulate(_) => {
                let result = crate::physical_operator::PhysicalAccumulate.execute(current_input)?;
                Ok(result)
            }
        }
    }
}
