pub mod map_aggregate;
pub mod map_ddl;
pub mod map_join;
pub mod map_projection;
pub mod map_scan;
pub mod map_update;

use crate::physical::types::PhysicalOperatorExec;
use crate::physical_operator::NodeSemiMask;
use crate::processor::QueryProcessor;
use akar_common::types::physical_type_from_logical;
use akar_common::vector::DataChunk;
use akar_function::registry::FunctionRegistry;
use akar_planner::logical_operator::LogicalOperator;
use akar_storage::table::TableCatalog;
use arrow::array::ArrayRef;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::{SchemaDdlFn, SequenceFn, StandaloneCallHandler, SubqueryFn};

/// Shared state threaded through the mapper functions
pub struct ExecutionContext<'a, 'p> {
    pub processor: &'p QueryProcessor,
    pub sip_masks: &'a mut HashMap<u64, NodeSemiMask>,
    pub function_registry: Option<Arc<Mutex<FunctionRegistry>>>,
    pub table_catalog: Option<Arc<TableCatalog>>,
    pub vfs: Option<Arc<akar_common::file_system::VirtualFileSystemRegistry>>,
    pub standalone_call_handler: Option<Arc<dyn StandaloneCallHandler>>,
    pub sequence_fn: Option<SequenceFn>,
    pub subquery_fn: Option<SubqueryFn>,
    pub schema_ddl_fn: Option<SchemaDdlFn>,
}

impl<'a, 'p> ExecutionContext<'a, 'p> {
    pub fn execute_children(&mut self, operators: &[LogicalOperator]) -> Result<Vec<DataChunk>, String> {
        self.processor.execute_internal(operators, self.sip_masks)
    }

    /// Resolve table data and column definitions for a scan node.
    pub fn resolve_scan_data<'b>(
        &self,
        table_name: &str,
        predicate: Option<(usize, &'b str, &'b akar_common::types::Value)>,
    ) -> (
        Option<Vec<Vec<akar_common::types::Value>>>,
        Vec<akar_storage::table::ColumnDefinition>,
        u64,
    ) {
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

    /// Resolve scan data directly into Arrow arrays, bypassing the
    /// `Vec<Vec<Value>>` intermediate materialization.
    ///
    /// Reads from NodeTable's NodeGroup column chunks, converts each
    /// ColumnChunk to an Arrow array, then concatenates per-group arrays
    /// into one array per column.
    pub fn resolve_scan_arrow_data(
        &self,
        table_name: &str,
    ) -> (Option<Vec<ArrayRef>>, Vec<akar_storage::table::ColumnDefinition>, u64) {
        if let Some(ref tc) = self.table_catalog {
            if let Some(node_table) = tc.get_node_table_by_name(table_name) {
                let num_rows = node_table.num_rows;
                if num_rows > 0 {
                    let mut column_arrays: Vec<Vec<ArrayRef>> = vec![Vec::new(); node_table.columns.len()];
                    for ng in &node_table.node_groups {
                        for (col_idx, col_chunk) in ng.columns.iter().enumerate() {
                            let phys_type = physical_type_from_logical(node_table.columns[col_idx].logical_type);
                            let arr = col_chunk.to_arrow_array(phys_type);
                            column_arrays[col_idx].push(arr);
                        }
                    }
                    // Concatenate per-group arrays into one array per column
                    let concat_arrays: Vec<ArrayRef> = column_arrays
                        .into_iter()
                        .map(|group_arrays| {
                            if group_arrays.len() == 1 {
                                group_arrays.into_iter().next().unwrap()
                            } else {
                                let refs: Vec<&dyn arrow::array::Array> =
                                    group_arrays.iter().map(|a| a.as_ref()).collect();
                                arrow::compute::concat(&refs)
                                    .unwrap_or_else(|_| group_arrays.into_iter().next().unwrap())
                            }
                        })
                        .collect();
                    return (Some(concat_arrays), node_table.columns.clone(), num_rows);
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
            | LogicalOperator::Partitioner(_) => map_projection::map_and_execute_projection(op, current_input, ctx),

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
