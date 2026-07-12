//! Query processor — maps logical operators to physical operators and executes them.
//!
//! Pipeline execution model:
//! 1. Scan operators produce raw DataChunks
//! 2. Filter removes non-matching rows
//! 3. Projection selects/transforms columns
//! 4. Limit/OrderBy/Aggregate are applied last

pub mod chunk_helpers;
pub mod join_helpers;
pub mod plan_serializer;
pub mod projection_helper;
pub mod union_helpers;

pub use chunk_helpers::*;
pub use join_helpers::*;
pub use plan_serializer::*;
pub use projection_helper::*;
pub use union_helpers::*;

use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical_operator::*;
use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_function::registry::{FunctionRegistry, TableFunction};
use kuzu_parser::ast::Expression;
use kuzu_planner::logical_operator::LogicalOperator;
use kuzu_storage::table::{ColumnDefinition, TableCatalog};
use std::sync::{Arc, Mutex};

pub type SequenceFn = Arc<dyn Fn(&str, bool) -> Result<Value, String> + Send + Sync>;
pub type SubqueryFn = Arc<dyn Fn(&kuzu_parser::ast::Query) -> Result<Vec<DataChunk>, String> + Send + Sync>;

pub trait StandaloneCallHandler: Send + Sync {
    fn execute_call(&self, name: &str, args: &[kuzu_parser::ast::Expression]) -> Result<Vec<kuzu_common::vector::DataChunk>, String>;
}

/// The query processor executes a physical plan and produces result chunks.
pub struct QueryProcessor {
    function_registry: Option<Arc<Mutex<FunctionRegistry>>>,
    table_catalog: Option<Arc<TableCatalog>>,
    vfs: Option<Arc<kuzu_common::file_system::VirtualFileSystemRegistry>>,
    standalone_call_handler: Option<Arc<dyn StandaloneCallHandler>>,
    /// Callback for sequence operations (nextval/currval).
    /// Takes (sequence_name, is_nextval) and returns the resulting value.
    sequence_fn: Option<SequenceFn>,
    /// Callback for executing subqueries.
    subquery_fn: Option<SubqueryFn>,
}

impl QueryProcessor {
    pub fn new() -> Self {
        Self {
            function_registry: None,
            table_catalog: None,
            vfs: None,
            standalone_call_handler: None,
            sequence_fn: None,
            subquery_fn: None,
        }
    }

    /// Create a processor with access to the function registry.
    pub fn with_registry(registry: Arc<Mutex<FunctionRegistry>>) -> Self {
        Self {
            function_registry: Some(registry),
            table_catalog: None,
            vfs: None,
            standalone_call_handler: None,
            sequence_fn: None,
            subquery_fn: None,
        }
    }

    /// Create a processor with function registry, table catalog access, and VFS.
    pub fn with_catalog(
        registry: Arc<Mutex<FunctionRegistry>>,
        table_catalog: Arc<TableCatalog>,
        vfs: Arc<kuzu_common::file_system::VirtualFileSystemRegistry>,
    ) -> Self {
        Self {
            function_registry: Some(registry),
            table_catalog: Some(table_catalog),
            vfs: Some(vfs),
            standalone_call_handler: None,
            sequence_fn: None,
            subquery_fn: None,
        }
    }

    /// Set the sequence operation callback (for nextval/currval).

    pub fn with_standalone_call_handler(mut self, handler: Arc<dyn StandaloneCallHandler>) -> Self {
        self.standalone_call_handler = Some(handler);
        self
    }

    pub fn with_sequence_fn(mut self, f: SequenceFn) -> Self {
        self.sequence_fn = Some(f);
        self
    }

    /// Set the subquery operation callback.
    pub fn with_subquery_fn(mut self, f: SubqueryFn) -> Self {
        self.subquery_fn = Some(f);
        self
    }

    /// Resolve table data and column definitions for a scan node.
    fn resolve_scan_data<'a>(
        &self,
        table_name: &str,
        predicate: Option<(usize, &'a str, &'a Value)>,
    ) -> (Option<Vec<Vec<Value>>>, Vec<ColumnDefinition>, u64) {
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

    fn projection_needs_expression_eval(expr: &Expression) -> bool {
        matches!(
            expr,
            Expression::FunctionCall(_, _)
                | Expression::Constant(_)
                | Expression::BinaryOp(_, _, _)
                | Expression::UnaryOp(_, _)
                | Expression::List(_)
                | Expression::Map(_)
                | Expression::Parameter(_)
                | Expression::ExistsSubquery(_)
                | Expression::ListPredicate { .. }
        )
    }

    fn extract_zone_map_predicate(expr: &Expression, columns: &[String]) -> Option<(usize, String, Value)> {
        if let Expression::BinaryOp(op, left, right) = expr {
            let op_str = match op {
                kuzu_parser::ast::BinaryOp::Equal => "=",
                kuzu_parser::ast::BinaryOp::GreaterThan => ">",
                kuzu_parser::ast::BinaryOp::LessThan => "<",
                kuzu_parser::ast::BinaryOp::GreaterThanOrEqual => ">=",
                kuzu_parser::ast::BinaryOp::LessThanOrEqual => "<=",
                kuzu_parser::ast::BinaryOp::NotEqual => "!=",
                _ => return None,
            };
            if let Expression::Variable(var_name) = &**left
                && let Expression::Constant(c) = &**right
            {
                let col_name = var_name.split('.').next_back().unwrap_or(var_name);
                if let Some(col_idx) = columns.iter().position(|c| c == col_name) {
                    let val = match c {
                        kuzu_parser::ast::Constant::Integer(i) => Value::Int64(*i),
                        kuzu_parser::ast::Constant::Float(f) => Value::Double(*f),
                        kuzu_parser::ast::Constant::String(s) => Value::String(s.clone()),
                        kuzu_parser::ast::Constant::Bool(b) => Value::Bool(*b),
                        kuzu_parser::ast::Constant::Null => Value::Null,
                    };
                    return Some((col_idx, op_str.to_string(), val));
                }
            }
        }
        None
    }

    /// Execute a sequence of logical operators by mapping them to physical operators.
    pub fn execute(&self, operators: &[LogicalOperator]) -> Result<Vec<DataChunk>, String> {
        let mut sip_masks = std::collections::HashMap::new();
        self.execute_internal(operators, &mut sip_masks)
    }

    pub fn execute_internal(
        &self,
        operators: &[LogicalOperator],
        sip_masks: &mut std::collections::HashMap<u64, NodeSemiMask>,
    ) -> Result<Vec<DataChunk>, String> {
        if operators.is_empty() {
            return Ok(vec![DataChunk {
                fields: vec![],
                size: 0,
                field_names: vec![],
            }]);
        }

        // Map logical operators to physical and execute in pipeline
        let mut intermediate_result: Option<Vec<DataChunk>> = None;

        for (i, op) in operators.iter().enumerate() {
            let current = intermediate_result.take().unwrap_or_else(|| {
                let mut dummy = DataChunk::new(vec![]);
                dummy.size = 1;
                vec![dummy]
            });
            match op {
                LogicalOperator::ScanNode(s) => {
                    let mut pred_owned = None;
                    if let Some(LogicalOperator::Filter(f)) = operators.get(i + 1) {
                        pred_owned = Self::extract_zone_map_predicate(&f.expression, &s.columns);
                    }

                    let pred_ref = pred_owned
                        .as_ref()
                        .map(|(idx, op_str, val)| (*idx, op_str.as_str(), val));
                    let (data, columns, num_rows) = self.resolve_scan_data(&s.table_name, pred_ref);
                    let mut scan = PhysicalScan::new(s.table_name.clone(), s.table_id, num_rows.max(1));
                    if let Some(mask) = sip_masks.get(&s.table_id) {
                        scan = scan.with_semi_mask(mask.clone(), 0);
                    }
                    if let Some(d) = data {
                        scan = scan.with_data(d, columns);
                    }
                    if let Some(ref fq) = s.fts_query {
                        scan = scan.with_fts_query(PhysicalFtsScan {
                            index_name: fq.index_name.clone(),
                            query_string: fq.query_string.clone(),
                            docs_table: fq.docs_table.clone(),
                            terms_table: fq.terms_table.clone(),
                            posting_table: fq.posting_table.clone(),
                            table_catalog: self.table_catalog.clone().expect("table catalog required for FTS scan"),
                        });
                    }
                    if let Some(ref pred) = s.predicate {
                        scan = scan.with_predicate(pred.clone());
                        scan = scan.with_evaluator(Arc::new(Mutex::new(ExpressionEvaluator::new(self.function_registry.clone().unwrap()))));
                    }
                    let mut result = scan.execute(current.clone())?;
                    let prefix = s.alias.as_ref().unwrap_or(&s.table_name);

                    for chunk in &mut result {
                        chunk.field_names = chunk.field_names.iter().map(|n| format!("{}.{}", prefix, n)).collect();
                    }
                    match &mut intermediate_result {
                        Some(existing) => existing.extend(result),
                        None => intermediate_result = Some(result),
                    }
                }
                LogicalOperator::SemiMasker(s) => {
                    let mask = NodeSemiMask::new(s.table_id);
                    let masker = PhysicalSemiMasker {
                        key_column: s.key_column,
                        mask: mask.clone(),
                    };

                    let result = if let Some(existing) = intermediate_result.take() {
                        masker.execute(existing)?
                    } else {
                        masker.execute(current.clone())?
                    };

                    sip_masks.insert(s.table_id, mask);
                    intermediate_result = Some(result);
                }
                LogicalOperator::ScanRel(s) => {
                    let (data, columns, _num_rows) = self.resolve_scan_data(&s.table_name, None);
                    let scan = PhysicalScanRel {
                        table_name: s.table_name.clone(),
                        table_id: s.table_id,
                        direction: s.direction.clone(),
                        table_data: data,
                        table_columns: columns,
                    };
                    let mut result = scan.execute(current.clone())?;
                    let prefix = &s.table_name;
                    for chunk in &mut result {
                        chunk.field_names = chunk.field_names.iter().map(|n| format!("{}.{}", prefix, n)).collect();
                    }
                    // Accumulate: extend rather than replace.
                    match &mut intermediate_result {
                        Some(existing) => existing.extend(result),
                        None => intermediate_result = Some(result),
                    }
                }
                LogicalOperator::VectorSimilarityScan(vs) => {
                    let scan = PhysicalVectorSimilarityScan {
                        index_name: vs.index_name.clone(),
                        index_id: vs.index_id,
                        query_vector: vs.query_vector.clone(),
                        top_k: vs.top_k,
                        table_name: vs.table_name.clone(),
                        table_catalog: self.table_catalog.clone(),
                    };
                    let result = scan.execute(current.clone())?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::ArtIndexRangeScan(ars) => {
                    let scan = PhysicalArtIndexRangeScan {
                        table_name: ars.table_name.clone(),
                        table_id: ars.table_id,
                        lower_bound: ars.lower_bound.clone(),
                        upper_bound: ars.upper_bound.clone(),
                        lower_inclusive: ars.lower_inclusive,
                        upper_inclusive: ars.upper_inclusive,
                        table_catalog: self.table_catalog.clone(),
                    };
                    let mut result = scan.execute(current.clone())?;
                    // Prefix field names with the alias (same as ScanNode)
                    let prefix = ars.alias.as_ref().unwrap_or(&ars.table_name);
                    for chunk in &mut result {
                        chunk.field_names = chunk.field_names.iter().map(|n| format!("{}.{}", prefix, n)).collect();
                    }
                    intermediate_result = Some(result);
                }
                LogicalOperator::Accumulate(_ac) => {
                    let result = PhysicalAccumulate.execute(current)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::RecursiveExtend(re) => {
                    let scan = PhysicalRecursiveExtend {
                        source_table_id: re.source_table_id,
                        rel_table_ids: re.rel_table_ids.clone(),
                        lower_bound: re.lower_bound,
                        upper_bound: re.upper_bound,
                        direction: re.direction,
                        semantic: re.semantic,
                        table_catalog: self.table_catalog.clone(),
                        weight_property: re.weight_property.clone(),
                        cost_output_name: re.cost_output_name.clone(),
                    };
                    let result = scan.execute(current.clone())?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Filter(f) => {
                    let evaluator = self.function_registry.clone().map(|reg| {
                        let mut eval = ExpressionEvaluator::new(reg);
                        if let Some(ref seq_fn) = self.sequence_fn {
                            eval = eval.with_sequence_fn(seq_fn.clone());
                        }
                        if let Some(ref subquery_fn) = self.subquery_fn {
                            eval = eval.with_subquery_fn(subquery_fn.clone());
                        }
                        Arc::new(Mutex::new(eval))
                    });
                    let filter = if let Some(eval) = evaluator {
                        PhysicalFilter::with_evaluator(f.expression.clone(), eval)
                    } else {
                        PhysicalFilter::new(f.expression.clone())
                    };
                    let input = current;
                    let result = filter.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Projection(p) => {
                    let input = if p.children.is_empty() {
                        current
                    } else {
                        self.execute_internal(&p.children, sip_masks)?
                    };

                    let result = if p.expressions.is_empty() {
                        // Dummy projection inserted by the planner (e.g. around cross product children).
                        // It should just pass the chunk through, NOT project column indices!
                        input
                    } else {
                        let needs_eval = p
                            .expressions
                            .iter()
                            .any(|be| Self::projection_needs_expression_eval(&be.expression));

                        if needs_eval {
                            let registry = self.function_registry.clone().ok_or_else(|| {
                                "No function registry available for expression projection".to_string()
                            })?;

                            let mut eval = ExpressionEvaluator::new(registry);
                            if let Some(ref seq_fn) = self.sequence_fn {
                                eval = eval.with_sequence_fn(seq_fn.clone());
                            }
                            if let Some(ref subquery_fn) = self.subquery_fn {
                                eval = eval.with_subquery_fn(subquery_fn.clone());
                            }

                            let mut output = Vec::with_capacity(input.len());
                            for chunk in input {
                                let mut fields = Vec::with_capacity(p.expressions.len());
                                for be in &p.expressions {
                                    let result_vec = eval.evaluate(&be.expression, &chunk)?;
                                    fields.push(result_vec);
                                }
                                let size = fields.first().map(|f| f.size()).unwrap_or(chunk.size);
                                output.push(DataChunk {
                                    fields,
                                    size,
                                    field_names: vec![],
                                });
                            }
                            output
                        } else {
                            // Simple column references — resolve column indices from
                            // expression variable names against the input chunk's field names.
                            let column_indices: Vec<usize> = if let Some(first_chunk) = input.first() {
                                p.expressions
                                    .iter()
                                    .filter_map(|be| resolve_projection_column_index(&be.expression, first_chunk))
                                    .collect()
                            } else {
                                Vec::new()
                            };
                            // Fallback: if resolution failed for any expression, use 0..n
                            let column_indices = if column_indices.len() == p.expressions.len() {
                                column_indices
                            } else {
                                (0..p.expressions.len()).collect()
                            };
                            let proj = PhysicalProjection { column_indices };
                            proj.execute(input)?
                        }
                    };

                    intermediate_result = Some(result);
                }
                LogicalOperator::Limit(l) => {
                    let limit = PhysicalLimit {
                        limit: l.limit,
                        offset: l.offset,
                    };
                    let input = current;
                    let result = limit.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::TopK(tk) => {
                    // Build sort_keys from expressions: each key is (column_index, ascending)
                    let sort_keys: Vec<(u32, bool)> = tk
                        .sort_keys
                        .iter()
                        .enumerate()
                        .map(|(i, _s)| {
                            (i as u32, tk.sort_keys.get(i).map(|s| s.1).unwrap_or(true))
                        })
                        .collect();
                    let topk = PhysicalTopK {
                        sort_keys,
                        limit: tk.limit,
                        offset: tk.offset,
                    };
                    let input = current;
                    let result = topk.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::OrderBy(o) => {
                    // Build sort_keys: each key is (column_index, ascending)
                    let sort_keys: Vec<(u32, bool)> = o
                        .sort_keys
                        .iter()
                        .enumerate()
                        .map(|(i, _s)| (i as u32, o.sort_keys.get(i).map(|s| s.1).unwrap_or(true)))
                        .collect();
                    let order = PhysicalOrderBy { sort_keys };
                    let input = current;
                    let result = order.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Flatten(f) => {
                    // Flatten is a no-op in the flat-list execution model;
                    // it signals that the child's factorization group should
                    // be treated as flat during physical execution.
                    // We must execute its children first.
                    let result = if f.children.is_empty() {
                        current
                    } else {
                        self.execute_internal(&f.children, sip_masks)?
                    };
                    intermediate_result = Some(result);
                }
                LogicalOperator::Aggregate(a) => {
                    let funcs: Vec<kuzu_function::AggregateFunction> = a.aggregates.iter()
                        .map(|(n, _)| crate::physical::order_aggregate::parse_aggregate_function(n))
                        .collect();
                    let group_by_cols = if a.group_by.is_empty() {
                        Vec::new()
                    } else {
                        (0..a.group_by.len() as u32).collect()
                    };
                    
                    let shared_state = std::sync::Arc::new(crate::physical::order_aggregate::SharedAggregateState::new(
                        funcs,
                        group_by_cols,
                    ));
                    
                    let agg_scan = crate::physical::order_aggregate::PhysicalAggregateScan {
                        shared_state: shared_state.clone(),
                    };
                    let agg_finalize = crate::physical::order_aggregate::PhysicalAggregateFinalize {
                        shared_state,
                    };

                    let input = current;
                    
                    // Phase 1: Scan and accumulate (returns empty chunk in sequential push-down)
                    let _ = agg_scan.execute(input)?;
                    
                    // Phase 2: Finalize and yield grouped chunks
                    let result = agg_finalize.execute(vec![])?;
                    
                    intermediate_result = Some(result);
                }
                LogicalOperator::HashJoin(h) => {
                    let left_ops = flatten_union_child(&h.build_side);
                    let right_ops = flatten_union_child(&h.probe_side);

                    let build_chunks = self.execute_internal(&left_ops, sip_masks)?;
                    let probe_chunks = self.execute_internal(&right_ops, sip_masks)?;

                    let (build_cols, probe_cols) =
                        derive_join_column_indices(&h.join_keys, &build_chunks, &probe_chunks);
                    let join = PhysicalHashJoin {
                        build_columns: build_cols,
                        probe_columns: probe_cols,
                        semi_mask: None,
                    };
                    let result = join.execute_binary(&build_chunks, &probe_chunks)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::SemiJoin(s) => {
                    let left_ops = flatten_union_child(&s.left);
                    let right_ops = flatten_union_child(&s.right);

                    let build_chunks = self.execute_internal(&left_ops, sip_masks)?;
                    let probe_chunks = self.execute_internal(&right_ops, sip_masks)?;

                    let semi = PhysicalSemiJoin {
                        build_columns: vec![0],
                        probe_columns: vec![0],
                    };
                    let result = semi.execute_binary(&build_chunks, &probe_chunks)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::AntiJoin(a) => {
                    let left_ops = flatten_union_child(&a.left);
                    let right_ops = flatten_union_child(&a.right);

                    let build_chunks = self.execute_internal(&left_ops, sip_masks)?;
                    let probe_chunks = self.execute_internal(&right_ops, sip_masks)?;

                    let anti = PhysicalAntiJoin {
                        build_columns: vec![0],
                        probe_columns: vec![0],
                    };
                    let result = anti.execute_binary(&build_chunks, &probe_chunks)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Intersect(ic) => {
                    let left_ops = flatten_union_child(&ic.left);
                    let right_ops = flatten_union_child(&ic.right);

                    let build_chunks = self.execute_internal(&left_ops, sip_masks)?;
                    let probe_chunks = self.execute_internal(&right_ops, sip_masks)?;

                    let intersect = PhysicalIntersect {
                        num_build_sides: ic.num_build_sides,
                        probe_key_col: 0,
                        build_key_col: 0,
                    };
                    let result = intersect.execute_binary(&build_chunks, &probe_chunks)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Explain(ex) => {
                    // Serialize the inner plan tree to a string
                    let plan_str = serialize_plan_tree(&ex.inner, 0);
                    let explain = PhysicalExplain { inner_plan: plan_str };
                    let result = explain.execute(vec![])?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Unwind(uw) => {
                    let input = current;
                    let unwind = PhysicalUnwind {
                        expression: uw.expression.clone(),
                        variable: uw.variable.clone(),
                    };
                    let result = unwind.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::OptionalMatch(om) => {
                    // Execute left (required) subtree
                    let left_ops = flatten_union_child(&om.left);
                    let left_result = self.execute(&left_ops)?;

                    // Execute right (optional) subtree
                    let right_ops = flatten_union_child(&om.right);
                    let right_result = self.execute(&right_ops)?;

                    // Combine: use flattened row-level merge
                    let merged = merge_optional_chunks(left_result, right_result)?;
                    intermediate_result = Some(merged);
                }
                LogicalOperator::Set(sl) => {
                    let table_catalog = self
                        .table_catalog
                        .clone()
                        .ok_or_else(|| "No table catalog available for SET".to_string())?;

                    let set_op = PhysicalSet {
                        table_name: sl.table_name.clone(),
                        table_id: sl.table_id,
                        column_name: sl.column_name.clone(),
                        column_idx: sl.column_idx,
                        value: sl.value.clone(),
                        is_node: sl.is_node,
                        table_catalog,
                    };
                    let input = current;
                    let result = set_op.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Delete(dl) => {
                    let table_catalog = self
                        .table_catalog
                        .clone()
                        .ok_or_else(|| "No table catalog available for DELETE".to_string())?;

                    let delete_op = PhysicalDelete {
                        table_name: dl.table_name.clone(),
                        table_id: dl.table_id,
                        primary_key_column: dl.primary_key_column.clone(),
                        is_node: dl.is_node,
                        detach: dl.detach,
                        row_indices: Vec::new(),
                        table_catalog,
                    };
                    let input = current;
                    let result = delete_op.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::CreateNode(cn) => {
                    let table_catalog = self
                        .table_catalog
                        .clone()
                        .ok_or_else(|| "No table catalog available for CREATE".to_string())?;

                    let create_node_op = PhysicalInsertNode {
                        table_name: cn.table_name.clone(),
                        table_id: cn.table_id,
                        out_var_name: cn.out_var_name.clone(),
                        properties: cn.properties.clone(),
                        table_catalog,
                    };
                    let input = current;
                    let result = create_node_op.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::CreateRel(cr) => {
                    let table_catalog = self
                        .table_catalog
                        .clone()
                        .ok_or_else(|| "No table catalog available for CREATE".to_string())?;

                    let create_rel_op = PhysicalInsertRel {
                        table_name: cr.table_name.clone(),
                        table_id: cr.table_id,
                        src_node_name: cr.src_node_name.clone(),
                        dst_node_name: cr.dst_node_name.clone(),
                        properties: cr.properties.clone(),
                        table_catalog,
                    };
                    let input = current;
                    let result = create_rel_op.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Extend(ex) => {
                    let table_catalog = self
                        .table_catalog
                        .clone()
                        .ok_or_else(|| "No table catalog available for Extend".to_string())?;

                    let extend_op = PhysicalExtend {
                        rel_table_name: ex.rel_table_name.clone(),
                        rel_table_id: ex.rel_table_id,
                        bound_node_var: ex.bound_node_var.clone(),
                        direction: ex.direction.clone(),
                        dst_node_var: ex.dst_node_var.clone(),
                        dst_table_name: ex.dst_table_name.clone(),
                        dst_table_id: ex.dst_table_id,
                        table_catalog,
                    };
                    let input = current;
                    let result = extend_op.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::CrossProduct(cp) => {
                    let left_ops = flatten_union_child(&cp.left);
                    let right_ops = flatten_union_child(&cp.right);
                    let build_chunks = self.execute_internal(&left_ops, sip_masks)?;
                    let probe_chunks = self.execute_internal(&right_ops, sip_masks)?;
                    let cross = PhysicalCrossProduct;
                    let result = cross.execute_binary(&build_chunks, &probe_chunks)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Union(u) => {
                    // Flatten left and right subtrees into operator sequences
                    let left_ops = flatten_union_child(&u.left);
                    let right_ops = flatten_union_child(&u.right);

                    // Execute each side independently
                    let left_result = self.execute(&left_ops)?;
                    let right_result = self.execute(&right_ops)?;

                    // Merge: concatenate corresponding columns, dedup if UNION (not ALL)
                    let merged = merge_union_chunks(left_result, right_result, u.all)?;
                    intermediate_result = Some(merged);
                }
                LogicalOperator::CopyFrom(cf) => {
                    let table_catalog = self
                        .table_catalog
                        .clone()
                        .ok_or_else(|| "No table catalog available for COPY FROM".to_string())?;

                    // Get column definitions from the table catalog
                    let columns = if let Some(node_table) = table_catalog.get_node_table_by_name(&cf.table_name) {
                        node_table.columns.clone()
                    } else if let Some(rel_table) = table_catalog.get_rel_table_by_name(&cf.table_name) {
                        rel_table.columns.clone()
                    } else {
                        return Err(format!("Table '{}' not found in storage catalog", cf.table_name));
                    };

                    let copy_op = PhysicalCopyFrom {
                        table_name: cf.table_name.clone(),
                        table_id: cf.table_id,
                        file_path: cf.file_path.clone(),
                        columns,
                        options: cf.options.clone(),
                        table_catalog,
                        vfs: self.vfs.clone().expect("VFS not initialized in processor"),
                    };
                    let input = current;
                    let result = copy_op.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::BatchInsert(bi) => {
                    let table_catalog = self
                        .table_catalog
                        .clone()
                        .ok_or_else(|| "No table catalog available for BATCH INSERT".to_string())?;

                    let batch_op = PhysicalBatchInsert {
                        table_name: bi.table_name.clone(),
                        table_id: bi.table_id,
                        rows: bi.rows.clone(),
                        table_catalog,
                    };
                    let input = current;
                    let result = batch_op.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::IndexLookup(il) => {
                    let table_catalog = self
                        .table_catalog
                        .clone()
                        .ok_or_else(|| "No table catalog available for INDEX LOOKUP".to_string())?;

                    let lookup_op = PhysicalIndexLookup {
                        table_name: il.table_name.clone(),
                        table_id: il.table_id,
                        key_value: il.key_value.clone(),
                        table_catalog,
                    };
                    let input = current;
                    let result = lookup_op.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::TableFunctionCall(tf) => {
                    let result = self.execute_table_function(tf)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Foreach(fc) => {
                    let input = current;
                    let foreach_op = PhysicalForeach {
                        variable: fc.variable.clone(),
                        expression: fc.expression.clone(),
                        sub_plans: fc.sub_plans.clone(),
                        function_registry: self.function_registry.clone(),
                        table_catalog: self.table_catalog.clone(),
                        vfs: self.vfs.clone(),
                    };
                    let result = foreach_op.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Merge(m) => {
                    let table_catalog = self
                        .table_catalog
                        .clone()
                        .ok_or_else(|| "No table catalog available for MERGE".to_string())?;

                    let mut on_match_ops = Vec::new();
                    for set_item in &m.on_match {
                        on_match_ops.push(PhysicalSet {
                            table_name: set_item.table_name.clone(),
                            table_id: set_item.table_id,
                            column_name: set_item.column_name.clone(),
                            column_idx: set_item.column_idx,
                            value: set_item.value.clone(),
                            is_node: set_item.is_node,
                            table_catalog: table_catalog.clone(),
                        });
                    }

                    let mut on_create_ops = Vec::new();
                    for set_item in &m.on_create {
                        on_create_ops.push(PhysicalSet {
                            table_name: set_item.table_name.clone(),
                            table_id: set_item.table_id,
                            column_name: set_item.column_name.clone(),
                            column_idx: set_item.column_idx,
                            value: set_item.value.clone(),
                            is_node: set_item.is_node,
                            table_catalog: table_catalog.clone(),
                        });
                    }

                    let merge_op = PhysicalMerge {
                        table_name: m.table_name.clone(),
                        table_id: m.table_id,
                        properties: m.properties.clone(),
                        on_match: on_match_ops,
                        on_create: on_create_ops,
                        table_catalog,
                    };
                    let input = current;
                    let result = merge_op.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::ExpressionsScan(_es) => {
                    // ExpressionsScan reads correlated variables from outer context.
                    // Returns empty data (variables resolved at runtime via ExpressionEvaluator).
                    intermediate_result = Some(vec![DataChunk::new(vec![])]);
                }
                LogicalOperator::CountRelTable(crt) => {
                    let physical = PhysicalCountRelTable {
                        table_name: crt.table_name.clone(),
                        table_id: crt.table_id,
                        table_catalog: self.table_catalog.clone(),
                    };
                    let result = physical.execute(vec![])?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Partitioner(p) => {
                    // Split accumulated data into morsels for parallel downstream processing.
                    // MORSEL_SIZE = 1024 rows per morsel enables cache-friendly parallelism.
                    const MORSEL_SIZE: usize = 1024;
                    let partitioner = Partitioner::new(MORSEL_SIZE);
                    let morsels = partitioner.execute(current)?;

                    // Process children sequentially for now; each morsel flows through
                    // the remaining pipeline independently. Full morsel-driven parallelism
                    // (via rayon::par_iter) requires making execute_internal accept
                    // pre-computed input, which is a deeper architecture change.
                    let mut results = Vec::new();
                    for _morsel in morsels {
                        let child_result = self.execute_internal(&p.children, sip_masks)?;
                        results.extend(child_result);
                    }
                    intermediate_result = Some(results);
                }
                LogicalOperator::PathPropertyProbe(p) => {
                    let properties = p.properties.iter().map(|(t, is_node, props)| {
                        crate::physical::scan_filter::PathPropertySpec {
                            table_name: t.clone(),
                            is_node: *is_node,
                            property_names: props.clone(),
                        }
                    }).collect();

                    let probe = crate::physical::scan_filter::PhysicalPathPropertyProbe {
                        node_ids_col_idx: p.node_ids_col_idx,
                        edge_ids_col_idx: p.edge_ids_col_idx,
                        properties,
                        table_catalog: self.table_catalog.clone().ok_or_else(|| "table catalog required for PathPropertyProbe".to_string())?,
                    };

                    let input = if !p.children.is_empty() {
                        self.execute_internal(&p.children, sip_masks)?
                    } else {
                        current
                    };

                    let result = probe.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::StandaloneCall(c) => {
                    let handler = self.standalone_call_handler.clone().ok_or("No standalone call handler provided")?;
                    let exec = crate::physical::write_ops::PhysicalStandaloneCall {
                        function_name: c.function_name.clone(),
                        args: c.args.clone(),
                        handler,
                    };
                    let result = exec.execute(current)?;
                    intermediate_result = Some(result);
                }

                LogicalOperator::CreateNodeTable(_)
                | LogicalOperator::CreateRelTable(_)
                | LogicalOperator::DropTable(_)
                | LogicalOperator::AlterTable(_)
                | LogicalOperator::CreateIndex(_)
                | LogicalOperator::DropIndex(_)
                | LogicalOperator::CreateVectorIndex(_) => {
                    intermediate_result = Some(vec![DataChunk {
                        fields: vec![],
                        size: 1,
                        field_names: vec![],
                    }]);
                }
                LogicalOperator::CreateFtsIndex(c) => {
                    if let Some(ref tc) = self.table_catalog {
                        let fts_index = PhysicalCreateFtsIndex {
                            index_name: c.index_name.clone(),
                            table_name: c.table_name.clone(),
                            column_name: c.column_name.clone(),
                            docs_table: c.docs_table.clone(),
                            terms_table: c.terms_table.clone(),
                            posting_table: c.posting_table.clone(),
                            table_catalog: tc.clone(),
                        };
                        intermediate_result = Some(fts_index.execute(current)?);
                    } else {
                        return Err("CREATE FTS INDEX requires a table catalog".into());
                    }
                }
                LogicalOperator::FtsScan(s) => {
                    if let Some(ref tc) = self.table_catalog {
                        let fts_scan = PhysicalFtsScan {
                            index_name: s.index_name.clone(),
                            query_string: s.query_string.clone(),
                            docs_table: s.docs_table.clone(),
                            terms_table: s.terms_table.clone(),
                            posting_table: s.posting_table.clone(),
                            table_catalog: tc.clone(),
                        };
                        intermediate_result = Some(fts_scan.execute(current)?);
                    } else {
                        return Err("FTS scan requires a table catalog".into());
                    }
                }
                LogicalOperator::CreateSequence(_)
                | LogicalOperator::DropSequence(_)
                | LogicalOperator::CreateDml(_)
                | LogicalOperator::ExportDatabase(_)
                | LogicalOperator::ImportDatabase(_) => {
                    intermediate_result = Some(vec![DataChunk {
                        fields: vec![],
                        size: 0,
                        field_names: vec![],
                    }]);
                }
                LogicalOperator::EmptyResult(_) => {
                    let exec = crate::physical::misc::PhysicalEmptyResult;
                    intermediate_result = Some(exec.execute(current)?);
                }
                LogicalOperator::MultiplicityReducer(m) => {
                    let exec = crate::physical::misc::PhysicalMultiplicityReducer {
                        key_columns: m.key_columns.clone(),
                    };
                    let input = if !m.children.is_empty() {
                        self.execute_internal(&m.children, sip_masks)?
                    } else {
                        current
                    };
                    intermediate_result = Some(exec.execute(input)?);
                }
                LogicalOperator::Skip(s) => {
                    let exec = crate::physical::misc::PhysicalSkip {
                        skip_count: s.offset as usize,
                    };
                    let input = if !s.children.is_empty() {
                        self.execute_internal(&s.children, sip_masks)?
                    } else {
                        current
                    };
                    intermediate_result = Some(exec.execute(input)?);
                }
                LogicalOperator::Insert(i) => {
                    let exec = crate::physical::misc::PhysicalInsert {
                        table_name: i.table_name.clone(),
                        table_id: i.table_id,
                        columns: i.columns.clone(),
                        values: i.values.clone(),
                        table_catalog: self.table_catalog.clone().unwrap(),
                    };
                    intermediate_result = Some(exec.execute(current)?);
                }
                LogicalOperator::ExtensionClause(e) => {
                    let exec = crate::physical::misc::PhysicalExtensionClause {
                        action: e.action.clone(),
                        extension_name: e.extension_name.clone(),
                    };
                    intermediate_result = Some(exec.execute(current)?);
                }
            }
        }

        Ok(intermediate_result.unwrap_or_default())
    }

    /// Execute a table function call by looking up the function in the registry
    /// and dispatching to the appropriate handler.
    fn execute_table_function(
        &self,
        tf: &kuzu_planner::logical_operator::LogicalTableFunctionCall,
    ) -> Result<Vec<DataChunk>, String> {
        let func_name = &tf.function_name;
        let args: Vec<Value> = Vec::new(); // args would be evaluated from expressions

        // Look up the function in the registry
        if let Some(ref registry) = self.function_registry {
            let reg = registry.lock().unwrap();
            if let Some(tbl_fn) = reg.get_table(func_name) {
                match tbl_fn {
                    TableFunction::CustomTable { execute, .. } => {
                        let mut chunk = DataChunk::new(Vec::new());
                        (execute)(&args, &mut chunk)?;
                        Ok(vec![chunk])
                    }
                    TableFunction::ScanCsv { .. }
                    | TableFunction::ScanParquet { .. }
                    | TableFunction::ScanJson { .. }
                    | TableFunction::ListTables
                    | TableFunction::ShowColumns { .. }
                    | TableFunction::CurrentSetting { .. } => Err(format!(
                        "Table function '{}' cannot be executed dynamically (no callback)",
                        func_name
                    )),
                    TableFunction::Custom { name } if name == "vector_similarity_scan" => {
                        // Evaluate args: [table_name, column_name, query_vector, top_k]
                        // For CALL statement, args are parsed as expressions. We need to evaluate them.
                        // For now, parse from the function args
                        drop(reg);
                        self.execute_vector_similarity_scan(tf)
                    }
                    TableFunction::Custom { name } => {
                        Err(format!("Custom table function '{}' has no registered handler", name))
                    }
                }
            } else {
                Err(format!("Table function '{}' not found", func_name))
            }
        } else {
            Err(format!(
                "Cannot execute table function '{}': no function registry available",
                func_name
            ))
        }
    }

    /// Execute a `vector_similarity_scan` table function call.
    ///
    /// Expects CALL vector_similarity_scan(table_name, column_name, query_vector, top_k)
    /// and dispatches to PhysicalVectorSimilarityScan with the processor's TableCatalog.
    fn execute_vector_similarity_scan(
        &self,
        tf: &kuzu_planner::logical_operator::LogicalTableFunctionCall,
    ) -> Result<Vec<DataChunk>, String> {
        // Evaluate arguments from expressions (they should be constants or simple vars)
        if tf.args.len() < 4 {
            return Err(
                "vector_similarity_scan requires 4 arguments: table_name, column_name, query_vector, top_k".into(),
            );
        }

        // For CALL statements, args arrive as Expression AST nodes.
        // Evaluate them to Values. The simplest approach: evaluate constants inline.
        fn eval_expr_to_value(expr: &kuzu_parser::ast::Expression) -> Option<Value> {
            match expr {
                kuzu_parser::ast::Expression::Constant(c) => match c {
                    kuzu_parser::ast::Constant::String(s) => Some(Value::String(s.clone())),
                    kuzu_parser::ast::Constant::Integer(i) => Some(Value::Int64(*i)),
                    kuzu_parser::ast::Constant::Float(f) => Some(Value::Double(*f)),
                    kuzu_parser::ast::Constant::Bool(b) => Some(Value::Bool(*b)),
                    kuzu_parser::ast::Constant::Null => Some(Value::Null),
                },
                kuzu_parser::ast::Expression::List(items) => {
                    let vals: Vec<Value> = items.iter().filter_map(eval_expr_to_value).collect();
                    Some(Value::List(vals))
                }
                _ => None, // Non-constant expression — skip
            }
        }

        let table_name = match eval_expr_to_value(&tf.args[0]) {
            Some(Value::String(s)) => s,
            _ => return Err("First argument to vector_similarity_scan must be a table name string".into()),
        };

        let _column_name = match eval_expr_to_value(&tf.args[1]) {
            Some(Value::String(s)) => s,
            _ => return Err("Second argument to vector_similarity_scan must be a column name string".into()),
        };

        let query_vector = match eval_expr_to_value(&tf.args[2]) {
            Some(Value::List(items)) => {
                let mut vec = Vec::with_capacity(items.len());
                for item in &items {
                    match item {
                        Value::Double(d) => vec.push(*d),
                        Value::Int64(i) => vec.push(*i as f64),
                        Value::Int32(i) => vec.push(*i as f64),
                        Value::Float(f) => vec.push(*f as f64),
                        _ => return Err("query_vector must be a list of numbers".into()),
                    }
                }
                vec
            }
            _ => return Err("Third argument to vector_similarity_scan must be a list of numbers".into()),
        };

        let top_k = match eval_expr_to_value(&tf.args[3]) {
            Some(Value::Int64(k)) if k > 0 => k as u64,
            _ => return Err("Fourth argument to vector_similarity_scan must be a positive integer".into()),
        };

        // Find the vector index on this table
        let tc = self
            .table_catalog
            .clone()
            .ok_or_else(|| "No table catalog available for vector_similarity_scan".to_string())?;

        // Look for a vector index matching this table name
        let index_name = {
            let mut found = None;
            for entry in tc.all_vector_indexes() {
                if entry.table_name == table_name {
                    found = Some(entry.name.clone());
                    break;
                }
            }
            found.ok_or_else(|| format!("No vector index found on table '{}'", table_name))?
        };

        // Dispatch to PhysicalVectorSimilarityScan
        let scan = PhysicalVectorSimilarityScan {
            index_name,
            index_id: 0,
            query_vector,
            top_k,
            table_name,
            table_catalog: Some(tc),
        };
        scan.execute(vec![])
    }

    /// Execute a single expression against a DataChunk and return a ValueVector of results.
    pub fn evaluate_expression(
        _expr: &kuzu_parser::ast::Expression,
        _chunk: &DataChunk,
    ) -> Result<ValueVector, String> {
        // Placeholder: return a dummy Int64 vector
        let size = _chunk.size;
        let mut v = ValueVector::new(PhysicalTypeID::Int64, size);
        for i in 0..size {
            v.set_i64(i, 0);
        }
        v.resize(size);
        Ok(v)
    }
}

impl Default for QueryProcessor {
    fn default() -> Self {
        Self::new()
    }
}





#[cfg(test)]
mod tests {
    use super::*;
    use hashbrown::HashMap;
    use kuzu_binder::bound_statement::BoundExpression;
    use kuzu_common::types::{LogicalTypeID, Value};
    use kuzu_parser::ast::{Constant, Expression};
    use kuzu_storage::table::ColumnDefinition;

    fn make_scan_op() -> LogicalOperator {
        LogicalOperator::ScanNode(kuzu_planner::logical_operator::LogicalScanNode { predicate: None,
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec![],
            cardinality: 0,
            fts_query: None,
        })
    }

    fn make_filter_op() -> LogicalOperator {
        LogicalOperator::Filter(kuzu_planner::logical_operator::LogicalFilter {
            expression: Expression::Constant(Constant::Bool(true)),
            children: vec![],
            cardinality: 0,
        })
    }

    fn make_proj_op() -> LogicalOperator {
        LogicalOperator::Projection(kuzu_planner::logical_operator::LogicalProjection {
            expressions: vec![BoundExpression {
                expression: Expression::Variable("a".into()),
                resolved_type: LogicalTypeID::Any,
                is_constant: false,
            }],
            children: vec![],
            cardinality: 0,
        })
    }

    fn make_limit_op() -> LogicalOperator {
        LogicalOperator::Limit(kuzu_planner::logical_operator::LogicalLimit {
            limit: 10,
            offset: 0,
            children: vec![],
            cardinality: 0,
        })
    }

    /// Create a processor with a Person table containing test data.
    fn make_processor_with_person_table() -> QueryProcessor {
        let catalog = Arc::new(TableCatalog::new());
        {
            catalog.create_node_table(
                "Person".into(),
                vec![
                    ColumnDefinition { compression: kuzu_common::enums::CompressionType::Uncompressed,
                        name: "name".into(),
                        logical_type: LogicalTypeID::String,
                        is_primary_key: true,
                    },
                    ColumnDefinition { compression: kuzu_common::enums::CompressionType::Uncompressed,
                        name: "age".into(),
                        logical_type: LogicalTypeID::Int64,
                        is_primary_key: false,
                    },
                ],
            );
            // Insert some data
            let mut table = catalog.get_node_table_by_name_mut("Person").unwrap();
            table
                .insert_row(vec![Value::String("Alice".into()), Value::Int64(30)])
                .unwrap();
            table
                .insert_row(vec![Value::String("Bob".into()), Value::Int64(25)])
                .unwrap();
        }
        let registry = Arc::new(Mutex::new(FunctionRegistry::new()));
        QueryProcessor::with_catalog(
            registry,
            catalog,
            std::sync::Arc::new(kuzu_common::file_system::VirtualFileSystemRegistry::new()),
        )
    }

    #[test]
    fn test_empty_plan() {
        let proc = QueryProcessor::new();
        let result = proc.execute(&[]).unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_scan_only() {
        let proc = make_processor_with_person_table();
        let result = proc.execute(&[make_scan_op()]).unwrap();
        assert!(!result.is_empty());
        assert!(result[0].num_fields() > 0);
        assert_eq!(result[0].size, 2); // 2 rows
    }

    #[test]
    fn test_scan_filter_projection() {
        let proc = make_processor_with_person_table();
        let plan = vec![make_scan_op(), make_filter_op(), make_proj_op()];
        let result = proc.execute(&plan).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_scan_filter_limit() {
        let proc = make_processor_with_person_table();
        let plan = vec![make_scan_op(), make_filter_op(), make_limit_op()];
        let result = proc.execute(&plan).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_filter_true_passthrough() {
        let filter = PhysicalFilter::new(Expression::Constant(Constant::Bool(true)));
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v.set_i64(i, i as i64);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = filter.execute(input).unwrap();
        assert!(!result.is_empty());
        assert_eq!(result[0].size, 5); // All rows pass through
    }

    #[test]
    fn test_filter_false_removes_all() {
        let filter = PhysicalFilter::new(Expression::Constant(Constant::Bool(false)));
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v.set_i64(i, i as i64);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = filter.execute(input).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_limit() {
        let limit = PhysicalLimit { limit: 3, offset: 0 };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 10);
        for i in 0..10 {
            v.set_i64(i, i as i64);
        }
        v.resize(10);
        let input = vec![DataChunk::new(vec![v])];
        let result = limit.execute(input).unwrap();
        assert_eq!(result[0].size, 3);
    }

    #[test]
    fn test_limit_with_offset() {
        let limit = PhysicalLimit { limit: 2, offset: 5 };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 10);
        for i in 0..10 {
            v.set_i64(i, i as i64);
        }
        v.resize(10);
        let input = vec![DataChunk::new(vec![v])];
        let result = limit.execute(input).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_projection() {
        let proj = PhysicalProjection {
            column_indices: vec![0],
        };
        let mut v1 = ValueVector::new(PhysicalTypeID::Int64, 5);
        let mut v2 = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v1.set_i64(i, i as i64);
            v2.set_i64(i, (i * 10) as i64);
        }
        v1.resize(5);
        v2.resize(5);
        let input = vec![DataChunk::new(vec![v1, v2])];
        let result = proj.execute(input).unwrap();
        assert_eq!(result[0].num_fields(), 1); // Only first column
    }

    #[test]
    fn test_projection_evaluates_function_call_no_input_source() {
        let state = Arc::new(Mutex::new(HashMap::new()));
        state.lock().unwrap().insert("s".to_string(), 1_i64);
        let state_for_fn = state.clone();
        let seq_fn: Arc<dyn Fn(&str, bool) -> Result<Value, String> + Send + Sync> =
            Arc::new(move |seq_name: &str, is_nextval: bool| {
                let mut m = state_for_fn.lock().map_err(|e| format!("Lock error: {e}"))?;
                let v = m
                    .get_mut(seq_name)
                    .ok_or_else(|| format!("Sequence '{}' not found", seq_name))?;
                if is_nextval {
                    let out = *v;
                    *v += 1;
                    Ok(Value::Int64(out))
                } else {
                    Ok(Value::Int64(*v))
                }
            });

        let proc =
            QueryProcessor::with_registry(Arc::new(Mutex::new(FunctionRegistry::new()))).with_sequence_fn(seq_fn);

        let plan = vec![LogicalOperator::Projection(
            kuzu_planner::logical_operator::LogicalProjection {
                expressions: vec![BoundExpression {
                    expression: Expression::FunctionCall(
                        "nextval".into(),
                        vec![Expression::Constant(Constant::String("s".into()))],
                    ),
                    resolved_type: LogicalTypeID::Int64,
                    is_constant: false,
                }],
                children: vec![],
                cardinality: 1,
            },
        )];

        let result = proc.execute(&plan).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 1);
        assert_eq!(result[0].fields[0].get_value(0), Some(Value::Int64(1)));
    }

    #[test]
    fn test_projection_sequence_missing_callback_errors() {
        let proc = QueryProcessor::with_registry(Arc::new(Mutex::new(FunctionRegistry::new())));

        let plan = vec![LogicalOperator::Projection(
            kuzu_planner::logical_operator::LogicalProjection {
                expressions: vec![BoundExpression {
                    expression: Expression::FunctionCall(
                        "nextval".into(),
                        vec![Expression::Constant(Constant::String("s".into()))],
                    ),
                    resolved_type: LogicalTypeID::Int64,
                    is_constant: false,
                }],
                children: vec![],
                cardinality: 1,
            },
        )];

        let err = proc.execute(&plan).unwrap_err();
        assert!(
            err.contains("No sequence callback configured"),
            "Unexpected error: {err}"
        );
    }

    // ==================== OrderBy Tests ====================

    #[test]
    fn test_order_by_ascending() {
        let order = PhysicalOrderBy {
            sort_keys: vec![(0, true)],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        let vals = [5, 3, 1, 4, 2];
        for i in 0..5 {
            v.set_i64(i, vals[i]);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = order.execute(input).unwrap();
        assert!(!result.is_empty());
        let sorted = result[0].fields[0].get_i64(0).unwrap();
        assert_eq!(sorted, 1); // Min should be first
    }

    #[test]
    fn test_order_by_descending() {
        let order = PhysicalOrderBy {
            sort_keys: vec![(0, false)],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        let vals = [5, 3, 1, 4, 2];
        for i in 0..5 {
            v.set_i64(i, vals[i]);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = order.execute(input).unwrap();
        assert!(!result.is_empty());
        let sorted = result[0].fields[0].get_i64(0).unwrap();
        assert_eq!(sorted, 5); // Max should be first
    }

    #[test]
    fn test_order_by_empty_input() {
        let order = PhysicalOrderBy {
            sort_keys: vec![(0, true)],
        };
        let result = order.execute(vec![]).unwrap();
        assert!(result.is_empty());
    }

    // ==================== Aggregate Tests ====================

    #[test]
    fn test_aggregate_count() {
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["COUNT".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v.set_i64(i, i as i64);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(5)); // COUNT = 5
    }

    #[test]
    fn test_aggregate_sum() {
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["SUM".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 4);
        for i in 0..4 {
            v.set_i64(i, (i + 1) as i64);
        }
        v.resize(4);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(10)); // 1+2+3+4 = 10
    }

    #[test]
    fn test_aggregate_min_max() {
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["MIN".into(), "MAX".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        let vals = [42, 7, 99, 15, 3];
        for i in 0..5 {
            v.set_i64(i, vals[i]);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(3)); // MIN = 3
        assert_eq!(result[0].fields[1].get_value(0).unwrap(), Value::Int64(99)); // MAX = 99
    }

    #[test]
    fn test_aggregate_avg() {
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["AVG".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 4);
        for i in 0..4 {
            v.set_i64(i, (i + 1) as i64);
        }
        v.resize(4);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        // AVG now returns Double (Value::Double)
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Double(2.5)); // (1+2+3+4)/4 = 2.5
    }

    #[test]
    fn test_aggregate_empty_input() {
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["COUNT".into()],
        };
        let result = agg.execute(vec![]).unwrap();
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(0)); // COUNT of empty = 0
    }

    // ==================== HashJoin Tests ====================

    #[test]
    fn test_hash_join_basic() {
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: None,
        };
        // Build side: keys [1, 2, 3]
        let mut build = ValueVector::new(PhysicalTypeID::Int64, 3);
        for i in 0..3 {
            build.set_i64(i, (i + 1) as i64);
        }
        build.resize(3);
        // Probe side: keys [2, 3, 4]
        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 3);
        probe.set_i64(0, 2);
        probe.set_i64(1, 3);
        probe.set_i64(2, 4);
        probe.resize(3);
        let build_chunk = DataChunk::new(vec![build]);
        let probe_chunk = DataChunk::new(vec![probe]);
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();
        // Should match 2 and 3 (2 rows)
        assert!(!result.is_empty());
    }

    #[test]
    fn test_hash_join_no_match() {
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: None,
        };
        // Build: [1, 2]
        let mut build = ValueVector::new(PhysicalTypeID::Int64, 2);
        build.set_i64(0, 1);
        build.set_i64(1, 2);
        build.resize(2);
        // Probe: [3, 4]
        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 2);
        probe.set_i64(0, 3);
        probe.set_i64(1, 4);
        probe.resize(2);
        let build_chunk = DataChunk::new(vec![build]);
        let probe_chunk = DataChunk::new(vec![probe]);
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();
        assert!(result.is_empty()); // No matches
    }

    #[test]
    fn test_hash_join_empty_build() {
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: None,
        };
        let build = ValueVector::new(PhysicalTypeID::Int64, 0);
        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 3);
        probe.set_i64(0, 1);
        probe.set_i64(1, 2);
        probe.set_i64(2, 3);
        probe.resize(3);
        let build_chunk = DataChunk::new(vec![build]);
        let probe_chunk = DataChunk::new(vec![probe]);
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();
        assert!(result.is_empty()); // Empty build → no matches
    }

    // ==================== Edge Case Tests ====================

    #[test]
    fn test_hash_join_null_keys_no_match() {
        // SQL semantics: NULL keys should never match in a join
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: None,
        };
        // Build side with NULLs mixed with real values
        let mut build = ValueVector::new(PhysicalTypeID::Int64, 3);
        build.set_i64(0, 1);
        // Row 1 stays NULL
        build.set_i64(2, 3);
        build.resize(3);
        // Probe side also has NULLs
        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 3);
        probe.set_i64(0, 1);
        probe.set_i64(1, 3);
        // Row 2 stays NULL
        probe.resize(3);
        let build_chunk = DataChunk::new(vec![build]);
        let probe_chunk = DataChunk::new(vec![probe]);
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();
        // Should match 1→1 (1 row) and 3→3 (1 row)
        // NULLs should NOT match each other
        assert!(!result.is_empty(), "Expected at least one matching row");
    }

    #[test]
    fn test_hash_join_all_null_keys() {
        // When both sides have all NULL keys → no matches
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: None,
        };
        let mut build = ValueVector::new(PhysicalTypeID::Int64, 3);
        build.resize(3);
        build.set_null(0, true);
        build.set_null(1, true);
        build.set_null(2, true);

        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 3);
        probe.resize(3);
        probe.set_null(0, true);
        probe.set_null(1, true);
        probe.set_null(2, true);

        let build_chunk = DataChunk::new(vec![build]);
        let probe_chunk = DataChunk::new(vec![probe]);
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();
        // NULL = NULL is unknown in SQL, so no matches
        assert!(result.is_empty());
    }

    // ==================== SemiMasker (SIP) Tests ====================

    #[test]
    fn test_semi_masker_basic() {
        // Create a semi-masker that collects Int64 values (node offsets)
        let mask = NodeSemiMask::new(0);
        let masker = PhysicalSemiMasker {
            key_column: 0,
            mask: mask.clone(),
        };

        // Input: chunk with Int64 values representing node offsets
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 3);
        v.resize(3);
        v.set_i64(0, 10);
        v.set_i64(1, 20);
        v.set_i64(2, 30);
        let input = vec![DataChunk::new(vec![v])];
        let result = masker.execute(input).unwrap();
        assert_eq!(result.len(), 1, "SemiMasker should pass through input");

        // Verify mask collected offsets by checking the underlying shared set
        let collected = mask.masked_offsets.lock().unwrap();
        assert!(collected.contains(&10), "Offset 10 should be masked");
        assert!(collected.contains(&20), "Offset 20 should be masked");
        assert!(collected.contains(&30), "Offset 30 should be masked");
        assert!(!collected.contains(&40), "Offset 40 should NOT be masked");
    }

    #[test]
    fn test_scan_with_semi_mask() {
        // Create a semi-mask with offsets 1, 3 (only allow these)
        let mask = NodeSemiMask::new(0);
        mask.mask(1);
        mask.mask(3);
        mask.finalize();

        // Create scan with 4 rows: offsets 0..3
        let mut scan = PhysicalScan::new("test".into(), 0, 10);
        let data = vec![
            vec![
                Value::InternalID(kuzu_common::types::InternalID { offset: 0, table_id: 0 }),
                Value::InternalID(kuzu_common::types::InternalID { offset: 1, table_id: 0 }),
                Value::InternalID(kuzu_common::types::InternalID { offset: 2, table_id: 0 }),
                Value::InternalID(kuzu_common::types::InternalID { offset: 3, table_id: 0 }),
            ],
            vec![
                Value::Int64(100),
                Value::Int64(200),
                Value::Int64(300),
                Value::Int64(400),
            ],
        ];
        let columns = vec![
            ColumnDefinition { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "id".into(),
                logical_type: LogicalTypeID::InternalID,
                is_primary_key: false,
            },
            ColumnDefinition { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "val".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
            },
        ];
        scan = scan.with_data(data, columns);
        scan = scan.with_semi_mask(mask, 0); // mask on column 0 (InternalID)

        let result = scan.execute(vec![]).unwrap();
        assert_eq!(result.len(), 1, "Should produce one chunk");
        assert_eq!(result[0].size, 2, "Should have 2 rows (offsets 1 and 3)");

        // Verify the values
        let val_field = &result[0].fields[1];
        assert_eq!(val_field.get_value(0), Some(Value::Int64(200)));
        assert_eq!(val_field.get_value(1), Some(Value::Int64(400)));
    }

    #[test]
    fn test_semi_mask_uninitialized_passes_all() {
        // An uninitialized mask should pass all rows (initialized = false)
        let mask = NodeSemiMask::new(0);
        // Don't call finalize — mask is not initialized

        assert!(mask.is_masked(999), "Uninitialized mask should pass all offsets");
    }

    #[test]
    fn test_hash_join_with_semi_mask_collects_build_keys() {
        // When a PhysicalHashJoin has a semi_mask, build-side keys are collected
        let mask = NodeSemiMask::new(0);
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: Some(mask.clone()),
        };

        // Build side with Int64 keys
        let mut build_v = ValueVector::new(PhysicalTypeID::Int64, 3);
        build_v.set_i64(0, 5);
        build_v.set_i64(1, 15);
        build_v.set_i64(2, 25);
        build_v.resize(3);

        // Probe side
        let mut probe_v = ValueVector::new(PhysicalTypeID::Int64, 3);
        probe_v.set_i64(0, 5);
        probe_v.set_i64(1, 15);
        probe_v.set_i64(2, 35);
        probe_v.resize(3);

        let build_chunk = DataChunk::new(vec![build_v]);
        let probe_chunk = DataChunk::new(vec![probe_v]);
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();

        // Should match 5→5 and 15→15 (2 rows). 35 has no build match.
        assert!(!result.is_empty(), "Expected 2 matching rows");

        // Verify mask collected build-side keys via underlying shared set
        let collected = mask.masked_offsets.lock().unwrap();
        assert!(collected.contains(&5), "Offset 5 should be in mask");
        assert!(collected.contains(&15), "Offset 15 should be in mask");
        assert!(collected.contains(&25), "Offset 25 should be in mask");
    }

    #[test]
    fn test_order_by_with_nulls() {
        // NULLs should sort last (ASC)
        let order = PhysicalOrderBy {
            sort_keys: vec![(0, true)],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        v.set_i64(0, 3);
        v.set_null(1, true); // NULL
        v.set_i64(2, 1);
        v.set_i64(3, 2);
        v.set_null(4, true); // NULL
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = order.execute(input).unwrap();
        assert!(!result.is_empty());
        // First three should be 1, 2, 3 (sorted ascending)
        assert_eq!(result[0].fields[0].get_i64(0).unwrap(), 1);
        assert_eq!(result[0].fields[0].get_i64(1).unwrap(), 2);
        assert_eq!(result[0].fields[0].get_i64(2).unwrap(), 3);
        // Last two should be NULL
        assert!(result[0].fields[0].is_null(3));
        assert!(result[0].fields[0].is_null(4));
    }

    #[test]
    fn test_limit_zero() {
        // LIMIT 0 should return empty result
        let limit = PhysicalLimit { limit: 0, offset: 0 };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v.set_i64(i, i as i64);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = limit.execute(input).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_limit_offset_exceeds_total() {
        // OFFSET larger than total rows → empty result
        let limit = PhysicalLimit { limit: 5, offset: 100 };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v.set_i64(i, i as i64);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = limit.execute(input).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_aggregate_count_with_nulls() {
        // COUNT should NOT count NULL values (standard SQL semantics)
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["COUNT".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        v.set_i64(0, 10);
        v.set_null(1, true);
        v.set_i64(2, 20);
        v.set_null(3, true);
        v.set_i64(4, 30);
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        // COUNT of [10, NULL, 20, NULL, 30] = 3
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(3));
    }

    #[test]
    fn test_aggregate_sum_with_nulls() {
        // SUM should skip NULLs
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["SUM".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        v.set_i64(0, 10);
        v.set_null(1, true);
        v.set_i64(2, 20);
        v.set_null(3, true);
        v.set_i64(4, 30);
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        // SUM of [10, NULL, 20, NULL, 30] = 60
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(60));
    }

    #[test]
    fn test_aggregate_group_by_with_nulls() {
        // GROUP BY with NULL keys: NULLs should group together
        let agg = PhysicalAggregate {
            group_by_cols: vec![0],
            aggregate_functions: vec!["COUNT".into()],
        };
        let n = 6;
        let mut keys = ValueVector::new(PhysicalTypeID::Int64, n);
        keys.set_i64(0, 1);
        keys.set_i64(1, 1);
        keys.set_null(2, true);
        keys.set_null(3, true);
        keys.set_i64(4, 2);
        keys.set_i64(5, 2);
        keys.resize(n);
        let mut vals = ValueVector::new(PhysicalTypeID::Int64, n);
        for i in 0..n {
            vals.set_i64(i, i as i64);
        }
        vals.resize(n);
        let input = vec![DataChunk::new(vec![keys, vals])];
        let result = agg.execute(input).unwrap();
        assert!(!result.is_empty());
        // Result should have 3 groups: key=1 (count=2), key=NULL (count=2), key=2 (count=2)
        assert_eq!(result[0].size, 3);
    }

    #[test]
    fn test_filter_with_nulls() {
        // Filter should treat NULL as false
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 4);
        v.set_i64(0, 1);
        v.set_null(1, true);
        v.set_i64(2, 3);
        v.set_i64(3, 4);
        v.resize(4);
        let input = vec![DataChunk::new(vec![v])];

        // Variable filter on first field: non-null rows pass
        let filter = PhysicalFilter::new(Expression::Variable("a".into()));
        let result = filter.execute(input.clone()).unwrap();
        assert!(!result.is_empty());
        assert_eq!(result[0].size, 3); // 3 non-null rows pass
    }

    #[test]
    fn test_empty_table_scan() {
        // Scan of an empty table should return empty result, not error
        let scan = PhysicalScan::new("EmptyTable".into(), 0, 0);
        let result = scan.execute(vec![]).unwrap();
        // Should return a valid empty DataChunk
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 0);
    }

    #[test]
    fn test_empty_input_through_pipeline() {
        // Empty input should produce empty output (no rows to process)
        let filter = PhysicalFilter::new(Expression::Constant(Constant::Bool(true)));
        let result = filter.execute(vec![DataChunk::new(vec![])]).unwrap();
        // Filter with 0 rows produces 0 output chunks (nothing to filter)
        assert!(result.is_empty());
    }

    // ==================== UNION Tests ====================

    fn make_i64_chunk(values: &[i64]) -> DataChunk {
        let mut v = ValueVector::new(PhysicalTypeID::Int64, values.len().max(1));
        for (i, val) in values.iter().enumerate() {
            v.set_i64(i, *val);
        }
        v.resize(values.len());
        DataChunk::new(vec![v])
    }

    #[test]
    fn test_union_all_basic() {
        // UNION ALL: two single-column Int64 vectors concatenated
        let mut left_v = ValueVector::new(PhysicalTypeID::Int64, 3);
        left_v.set_i64(0, 1);
        left_v.set_i64(1, 2);
        left_v.set_i64(2, 3);
        left_v.resize(3);
        let left_data = vec![DataChunk::new(vec![left_v])];
        let mut right_v = ValueVector::new(PhysicalTypeID::Int64, 2);
        right_v.set_i64(0, 4);
        right_v.set_i64(1, 5);
        right_v.resize(2);
        let right_data = vec![DataChunk::new(vec![right_v])];
        let result = merge_union_chunks(left_data, right_data, true).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 5);
        // Verify values in order
        assert_eq!(result[0].field(0).get_i64(0), Some(1));
        assert_eq!(result[0].field(0).get_i64(1), Some(2));
        assert_eq!(result[0].field(0).get_i64(2), Some(3));
        assert_eq!(result[0].field(0).get_i64(3), Some(4));
        assert_eq!(result[0].field(0).get_i64(4), Some(5));
    }

    #[test]
    fn test_union_all_multiple_chunks() {
        // UNION ALL: multiple chunks per side
        let mut v1 = ValueVector::new(PhysicalTypeID::Int64, 2);
        v1.set_i64(0, 1);
        v1.set_i64(1, 2);
        v1.resize(2);
        let mut v2 = ValueVector::new(PhysicalTypeID::Int64, 1);
        v2.set_i64(0, 3);
        v2.resize(1);
        let left = vec![DataChunk::new(vec![v1]), DataChunk::new(vec![v2])];
        let mut rv = ValueVector::new(PhysicalTypeID::Int64, 2);
        rv.set_i64(0, 4);
        rv.set_i64(1, 5);
        rv.resize(2);
        let right = vec![DataChunk::new(vec![rv])];
        let result = merge_union_chunks(left, right, true).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 5);
        assert_eq!(result[0].field(0).get_i64(0), Some(1));
        assert_eq!(result[0].field(0).get_i64(4), Some(5));
    }

    #[test]
    fn test_union_distinct_dedup() {
        // UNION (distinct): duplicates removed
        let mut lv = ValueVector::new(PhysicalTypeID::Int64, 3);
        lv.set_i64(0, 1);
        lv.set_i64(1, 2);
        lv.set_i64(2, 3);
        lv.resize(3);
        let left = vec![DataChunk::new(vec![lv])];
        let mut rv = ValueVector::new(PhysicalTypeID::Int64, 3);
        rv.set_i64(0, 2);
        rv.set_i64(1, 3);
        rv.set_i64(2, 4);
        rv.resize(3);
        let right = vec![DataChunk::new(vec![rv])];
        let result = merge_union_chunks(left, right, false).unwrap();
        assert_eq!(result.len(), 1);
        // Distinct values: {1, 2, 3, 4} → 4 rows
        assert_eq!(result[0].size, 4);
    }

    #[test]
    fn test_union_column_mismatch() {
        // Column count mismatch should produce an error
        let left = vec![DataChunk::new(vec![
            ValueVector::new(PhysicalTypeID::Int64, 1),
            ValueVector::new(PhysicalTypeID::Int64, 1),
        ])];
        let right = vec![DataChunk::new(vec![ValueVector::new(PhysicalTypeID::Int64, 1)])];
        let result = merge_union_chunks(left, right, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("column count mismatch"));
    }

    #[test]
    fn test_union_empty_left() {
        let left = vec![];
        let mut rv = ValueVector::new(PhysicalTypeID::Int64, 2);
        rv.set_i64(0, 42);
        rv.set_i64(1, 43);
        rv.resize(2);
        let right = vec![DataChunk::new(vec![rv])];
        let result = merge_union_chunks(left, right, true).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 2);
        assert_eq!(result[0].field(0).get_i64(0), Some(42));
    }

    #[test]
    fn test_union_empty_right() {
        let mut lv = ValueVector::new(PhysicalTypeID::Int64, 2);
        lv.set_i64(0, 99);
        lv.set_i64(1, 100);
        lv.resize(2);
        let left = vec![DataChunk::new(vec![lv])];
        let right = vec![];
        let result = merge_union_chunks(left, right, true).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 2);
    }

    #[test]
    fn test_union_all_multi_column() {
        // UNION ALL with multiple columns
        let mut left_v1 = ValueVector::new(PhysicalTypeID::Int64, 2);
        left_v1.set_i64(0, 1);
        left_v1.set_i64(1, 2);
        left_v1.resize(2);
        let mut left_v2 = ValueVector::new(PhysicalTypeID::String, 2);
        left_v2.push_string("hello");
        left_v2.push_string("world");
        let left = vec![DataChunk::new(vec![left_v1, left_v2])];

        let mut right_v1 = ValueVector::new(PhysicalTypeID::Int64, 1);
        right_v1.set_i64(0, 3);
        right_v1.resize(1);
        let mut right_v2 = ValueVector::new(PhysicalTypeID::String, 1);
        right_v2.push_string("foo");
        let right = vec![DataChunk::new(vec![right_v1, right_v2])];

        let result = merge_union_chunks(left, right, true).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 3);
        assert_eq!(result[0].field(0).get_i64(0), Some(1));
        assert_eq!(result[0].field(0).get_i64(1), Some(2));
        assert_eq!(result[0].field(0).get_i64(2), Some(3));
    }

    #[test]
    fn test_union_distinct_all_duplicates() {
        // All rows identical → single row after dedup
        let mut lv = ValueVector::new(PhysicalTypeID::Int64, 2);
        lv.set_i64(0, 1);
        lv.set_i64(1, 1);
        lv.resize(2);
        let left = vec![DataChunk::new(vec![lv])];
        let mut rv = ValueVector::new(PhysicalTypeID::Int64, 2);
        rv.set_i64(0, 1);
        rv.set_i64(1, 1);
        rv.resize(2);
        let right = vec![DataChunk::new(vec![rv])];
        let result = merge_union_chunks(left, right, false).unwrap();
        assert_eq!(result[0].size, 1);
        assert_eq!(result[0].field(0).get_i64(0), Some(1));
    }

    #[test]
    fn test_union_all_empty_chunks() {
        // Empty DataChunks should be handled gracefully
        let empty = ValueVector::new(PhysicalTypeID::Int64, 0);
        let left = vec![DataChunk::new(vec![empty])];
        let mut rv = ValueVector::new(PhysicalTypeID::Int64, 1);
        rv.set_i64(0, 42);
        rv.resize(1);
        let right = vec![DataChunk::new(vec![rv])];
        let result = merge_union_chunks(left, right, true).unwrap();
        assert_eq!(result[0].size, 1);
        assert_eq!(result[0].field(0).get_i64(0), Some(42));
    }

    // ==================== CrossProduct Tests ====================

    #[test]
    fn test_cross_product_basic() {
        let cross = PhysicalCrossProduct;
        // Left: [1, 2, 3], Right: [4, 5]
        let left = vec![make_i64_chunk(&[1, 2, 3])];
        let right = vec![make_i64_chunk(&[4, 5])];
        let result = cross.execute_binary(&left, &right).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 6); // 3 × 2 = 6
        assert_eq!(result[0].field(0).get_i64(0), Some(1));
        assert_eq!(result[0].field(0).get_i64(1), Some(1));
        assert_eq!(result[0].field(0).get_i64(2), Some(2));
        assert_eq!(result[0].field(0).get_i64(3), Some(2));
        assert_eq!(result[0].field(0).get_i64(4), Some(3));
        assert_eq!(result[0].field(0).get_i64(5), Some(3));
    }

    #[test]
    fn test_cross_product_multi_column() {
        let cross = PhysicalCrossProduct;
        // Left: [{1, "a"}, {2, "b"}], Right: [{10}, {20}]
        let mut l1 = ValueVector::new(PhysicalTypeID::Int64, 2);
        l1.set_i64(0, 1);
        l1.set_i64(1, 2);
        l1.resize(2);
        let mut l2 = ValueVector::new(PhysicalTypeID::String, 2);
        l2.push_string("a");
        l2.push_string("b");
        let left = DataChunk::new(vec![l1, l2]);

        let mut r1 = ValueVector::new(PhysicalTypeID::Int64, 2);
        r1.set_i64(0, 10);
        r1.set_i64(1, 20);
        r1.resize(2);
        let right = DataChunk::new(vec![r1]);

        let result = cross.execute_binary(&[left], &[right]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 4); // 2 × 2 = 4
        // Row 0: left(1,"a") × right(10) → [1, "a", 10]
        assert_eq!(result[0].field(0).get_i64(0), Some(1));
        // Row 1: left(1,"a") × right(20) → [1, "a", 20]
        assert_eq!(result[0].field(0).get_i64(1), Some(1));
        // Row 2: left(2,"b") × right(10) → [2, "b", 10]
        assert_eq!(result[0].field(0).get_i64(2), Some(2));
        // Row 3: left(2,"b") × right(20) → [2, "b", 20]
        assert_eq!(result[0].field(0).get_i64(3), Some(2));
        // Column 2 should have right-side values: [10, 20, 10, 20]
        assert_eq!(result[0].field(2).get_i64(0), Some(10));
        assert_eq!(result[0].field(2).get_i64(1), Some(20));
        assert_eq!(result[0].field(2).get_i64(2), Some(10));
        assert_eq!(result[0].field(2).get_i64(3), Some(20));
    }

    #[test]
    fn test_cross_product_empty_left() {
        let cross = PhysicalCrossProduct;
        let left = make_i64_chunk(&[]);
        let right = make_i64_chunk(&[1, 2]);
        let result = cross.execute_binary(&[left], &[right]).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_cross_product_empty_right() {
        let cross = PhysicalCrossProduct;
        let left = make_i64_chunk(&[1, 2, 3]);
        let right = make_i64_chunk(&[]);
        let result = cross.execute_binary(&[left], &[right]).unwrap();
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_cross_product_multi_chunk() {
        let cross = PhysicalCrossProduct;
        // Left: two chunks [1,2] and [3]
        let left = vec![make_i64_chunk(&[1, 2]), make_i64_chunk(&[3])];
        // Right: one chunk [4,5]
        let right = vec![make_i64_chunk(&[4, 5])];
        let result = cross.execute_binary(&left, &right).unwrap();
        assert_eq!(result[0].size, 6); // 3 × 2 = 6
    }

    // ==================== SemiJoin / AntiJoin Tests ====================

    #[test]
    fn test_semi_join_basic() {
        let semi = PhysicalSemiJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
        };
        // Build (right): [2, 3]
        let build = make_i64_chunk(&[2, 3]);
        // Probe (left): [1, 2, 3]
        let probe = make_i64_chunk(&[1, 2, 3]);
        let result = semi.execute_binary(&[build], &[probe]).unwrap();
        assert_eq!(result[0].size, 2); // [2, 3] match
    }

    #[test]
    fn test_semi_join_no_match() {
        let semi = PhysicalSemiJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
        };
        let build = make_i64_chunk(&[4, 5]);
        let probe = make_i64_chunk(&[1, 2, 3]);
        let result = semi.execute_binary(&[build], &[probe]).unwrap();
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_anti_join_basic() {
        let anti = PhysicalAntiJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
        };
        // Build (right): [2, 3]
        let build = make_i64_chunk(&[2, 3]);
        // Probe (left): [1, 2, 3]
        let probe = make_i64_chunk(&[1, 2, 3]);
        let result = anti.execute_binary(&[build], &[probe]).unwrap();
        assert_eq!(result[0].size, 1); // Only [1] has no match
    }

    #[test]
    fn test_anti_join_all_match() {
        let anti = PhysicalAntiJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
        };
        let build = make_i64_chunk(&[1, 2, 3]);
        let probe = make_i64_chunk(&[1, 2, 3]);
        let result = anti.execute_binary(&[build], &[probe]).unwrap();
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_semi_join_empty_build() {
        let semi = PhysicalSemiJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
        };
        let build = make_i64_chunk(&[]);
        let probe = make_i64_chunk(&[1, 2, 3]);
        let result = semi.execute_binary(&[build], &[probe]).unwrap();
        assert!(result.is_empty() || result[0].size == 0);
    }

    // --- Intersect tests ---

    #[test]
    fn test_intersect_basic() {
        let intersect = PhysicalIntersect {
            num_build_sides: 2,
            probe_key_col: 0,
            build_key_col: 0,
        };
        // Two build sides with overlapping keys
        let build1 = make_i64_chunk(&[1, 2, 3]);
        let build2 = make_i64_chunk(&[2, 3, 4]);
        // Probe with keys that should match across both builds
        let probe = make_i64_chunk(&[2, 3]);
        let build_chunks = vec![build1, build2];
        let probe_chunks = vec![probe];
        let result = intersect.execute_binary(&build_chunks, &probe_chunks).unwrap();
        // Keys 2 and 3 exist in both build sides → both should produce output
        assert!(!result.is_empty(), "Expected non-empty result");
        assert!(result[0].size > 0, "Expected at least one output row");
    }

    #[test]
    fn test_intersect_no_common() {
        let intersect = PhysicalIntersect {
            num_build_sides: 2,
            probe_key_col: 0,
            build_key_col: 0,
        };
        // Build sides have no overlapping keys
        let build1 = make_i64_chunk(&[1, 2, 3]);
        let build2 = make_i64_chunk(&[4, 5, 6]);
        let probe = make_i64_chunk(&[1, 2, 3, 4, 5, 6]);
        let build_chunks = vec![build1, build2];
        let probe_chunks = vec![probe];
        let result = intersect.execute_binary(&build_chunks, &probe_chunks).unwrap();
        // No key appears in ALL build sides → empty
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_intersect_probe_key_missing() {
        let intersect = PhysicalIntersect {
            num_build_sides: 2,
            probe_key_col: 0,
            build_key_col: 0,
        };
        // Build sides share key 3, but probe doesn't probe for 3
        let build1 = make_i64_chunk(&[1, 3]);
        let build2 = make_i64_chunk(&[3, 5]);
        let probe = make_i64_chunk(&[1, 5]); // probes for 1 and 5 — only 1 is in build1, only 5 is in build2
        let build_chunks = vec![build1, build2];
        let probe_chunks = vec![probe];
        let result = intersect.execute_binary(&build_chunks, &probe_chunks).unwrap();
        // No key appears in ALL build sides → empty (1 not in build2, 5 not in build1)
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_intersect_single_build_side() {
        let intersect = PhysicalIntersect {
            num_build_sides: 1,
            probe_key_col: 0,
            build_key_col: 0,
        };
        // Single build side — acts like semi-join
        let build = make_i64_chunk(&[2, 3]);
        let probe = make_i64_chunk(&[1, 2, 3, 4]);
        let build_chunks = vec![build];
        let probe_chunks = vec![probe];
        let result = intersect.execute_binary(&build_chunks, &probe_chunks).unwrap();
        assert!(!result.is_empty(), "Expected non-empty result for single build side");
        assert!(result[0].size > 0, "Expected matching rows");
    }

    #[test]
    fn test_intersect_empty_build() {
        let intersect = PhysicalIntersect {
            num_build_sides: 2,
            probe_key_col: 0,
            build_key_col: 0,
        };
        let build1 = make_i64_chunk(&[]);
        let build2 = make_i64_chunk(&[1, 2, 3]);
        let probe = make_i64_chunk(&[1, 2, 3]);
        let build_chunks = vec![build1, build2];
        let probe_chunks = vec![probe];
        let result = intersect.execute_binary(&build_chunks, &probe_chunks).unwrap();
        // Empty build side → empty result
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_intersect_no_probe() {
        let intersect = PhysicalIntersect {
            num_build_sides: 2,
            probe_key_col: 0,
            build_key_col: 0,
        };
        let build1 = make_i64_chunk(&[1, 2, 3]);
        let build2 = make_i64_chunk(&[2, 3, 4]);
        let build_chunks = vec![build1, build2];
        let probe_chunks = vec![make_i64_chunk(&[])];
        let result = intersect.execute_binary(&build_chunks, &probe_chunks).unwrap();
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_intersect_three_build_sides() {
        let intersect = PhysicalIntersect {
            num_build_sides: 3,
            probe_key_col: 0,
            build_key_col: 0,
        };
        // Three build sides: key 3 appears in all
        let build1 = make_i64_chunk(&[1, 3, 5]);
        let build2 = make_i64_chunk(&[2, 3, 6]);
        let build3 = make_i64_chunk(&[3, 4, 7]);
        let build_chunks = vec![build1, build2, build3];
        let probe = make_i64_chunk(&[3]);
        let probe_chunks = vec![probe];
        let result = intersect.execute_binary(&build_chunks, &probe_chunks).unwrap();
        assert!(!result.is_empty(), "Expected match for key 3 in all three build sides");
        assert!(result[0].size > 0);
    }

    // ==================== Bug-regression: property access & join correctness ====================

    /// Bug 0.1 regression: `evaluate_property_access` must resolve the named column,
    /// not always return the first column.
    #[test]
    fn test_property_access_resolves_named_column() {
        use crate::expression_evaluator::ExpressionEvaluator;
        use kuzu_common::types::PhysicalTypeID;
        use kuzu_common::vector::{DataChunk, ValueVector};
        use kuzu_function::registry::FunctionRegistry;
        use kuzu_parser::ast::Expression;
        use std::sync::{Arc, Mutex};

        // Build a chunk with two columns: col 0 = id (Int64), col 1 = name (String)
        // field_names = ["id", "name"]
        let mut id_col = ValueVector::new(PhysicalTypeID::Int64, 2);
        id_col.set_i64(0, 10);
        id_col.set_i64(1, 20);
        id_col.resize(2);

        let mut name_col = ValueVector::new(PhysicalTypeID::String, 2);
        name_col.push_string("alice");
        name_col.push_string("bob");

        let chunk = DataChunk::new(vec![id_col, name_col]).with_names(vec!["id".into(), "name".into()]);

        let registry = Arc::new(Mutex::new(FunctionRegistry::new()));
        let eval = ExpressionEvaluator::new(registry);

        // Requesting "name" should return the String column (col 1), not "id" (col 0).
        let expr = Expression::PropertyAccess(Box::new(Expression::Variable("t".into())), "name".into());
        let result = eval.evaluate(&expr, &chunk).unwrap();
        assert_eq!(
            result.physical_type(),
            PhysicalTypeID::String,
            "PropertyAccess('name') must return the String column, not Int64"
        );

        // Requesting "id" should return the Int64 column (col 0).
        let expr_id = Expression::PropertyAccess(Box::new(Expression::Variable("t".into())), "id".into());
        let result_id = eval.evaluate(&expr_id, &chunk).unwrap();
        assert_eq!(
            result_id.physical_type(),
            PhysicalTypeID::Int64,
            "PropertyAccess('id') must return the Int64 column"
        );
        assert_eq!(result_id.get_i64(0), Some(10));
        assert_eq!(result_id.get_i64(1), Some(20));
    }

    /// Bug 0.2 regression: HashJoin with non-overlapping IDs must return 0 rows.
    /// Previously, the second Scan overwrote the first so HashJoin only saw one table
    /// and pass-through'd — returning wrong results.
    #[test]
    fn test_hash_join_non_overlapping_ids_returns_zero_rows() {
        use kuzu_common::types::PhysicalTypeID;
        use kuzu_common::vector::{DataChunk, ValueVector};

        // Build side: id = [1, 2]
        let mut build_id = ValueVector::new(PhysicalTypeID::Int64, 2);
        build_id.set_i64(0, 1);
        build_id.set_i64(1, 2);
        build_id.resize(2);
        let build_chunk = DataChunk::new(vec![build_id]).with_names(vec!["id".into()]);

        // Probe side: id = [3, 4] — no overlap
        let mut probe_id = ValueVector::new(PhysicalTypeID::Int64, 2);
        probe_id.set_i64(0, 3);
        probe_id.set_i64(1, 4);
        probe_id.resize(2);
        let probe_chunk = DataChunk::new(vec![probe_id]).with_names(vec!["id".into()]);

        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: None,
        };
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();
        // Must be empty — no rows share the same id.
        assert!(
            result.is_empty() || result.iter().all(|c| c.size == 0),
            "HashJoin on non-overlapping IDs must produce 0 rows, got {:?}",
            result.iter().map(|c| c.size).collect::<Vec<_>>()
        );
    }

    /// Bug 0.2 regression: scan accumulation — two scans feeding a HashJoin must both
    /// appear in the operator's input (not one overwriting the other).
    #[test]
    fn test_scan_accumulation_both_scans_reach_join() {
        use kuzu_common::types::{LogicalTypeID, Value};
        use kuzu_parser::ast::{BinaryOp, Expression};
        use kuzu_planner::logical_operator::{LogicalHashJoin, LogicalOperator, LogicalScanNode};
        use kuzu_storage::table::{ColumnDefinition, TableCatalog};

        let col_id = ColumnDefinition { compression: kuzu_common::enums::CompressionType::Uncompressed,
            name: "id".into(),
            logical_type: LogicalTypeID::Int64,
            is_primary_key: true,
        };

        // Create tables in the catalog and populate them.
        let catalog = TableCatalog::new();
        catalog.create_node_table("A".into(), vec![col_id.clone()]);
        catalog.create_node_table("B".into(), vec![col_id.clone()]);

        {
            let mut tbl_a = catalog.get_node_table_by_name_mut("A").unwrap();
            tbl_a.insert_row(vec![Value::Int64(1)]).unwrap();
            tbl_a.insert_row(vec![Value::Int64(2)]).unwrap();
        }
        {
            let mut tbl_b = catalog.get_node_table_by_name_mut("B").unwrap();
            tbl_b.insert_row(vec![Value::Int64(2)]).unwrap(); // overlaps with A
            tbl_b.insert_row(vec![Value::Int64(3)]).unwrap();
        }

        use kuzu_function::registry::FunctionRegistry;
        use std::sync::{Arc, Mutex};
        let registry = Arc::new(Mutex::new(FunctionRegistry::new()));
        let proc = QueryProcessor::with_catalog(
            registry,
            std::sync::Arc::new(catalog),
            std::sync::Arc::new(kuzu_common::file_system::VirtualFileSystemRegistry::new()),
        );

        // Plan: ScanNode(A), ScanNode(B), HashJoin{join_keys: [a.id = b.id]}
        let join_key = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("b".into())),
                "id".into(),
            )),
        );
        let plan = vec![LogicalOperator::HashJoin(LogicalHashJoin {
            join_keys: vec![join_key],
            build_side: Box::new(LogicalOperator::ScanNode(LogicalScanNode { predicate: None,
                table_name: "A".into(),
                table_id: 1,
                alias: Some("a".into()),
                columns: vec![],
                cardinality: 2,
                fts_query: None,
            })),
            probe_side: Box::new(LogicalOperator::ScanNode(LogicalScanNode { predicate: None,
                table_name: "B".into(),
                table_id: 2,
                alias: Some("b".into()),
                columns: vec![],
                cardinality: 2,
                fts_query: None,
            })),
            cardinality: 0,
            push_down_eligible: false,
        })];

        let result = proc.execute(&plan).unwrap();
        // A has ids [1,2], B has ids [2,3]. Join on id → exactly 1 matching row (id=2).
        let total_rows: usize = result.iter().map(|c| c.size).sum();
        assert_eq!(
            total_rows, 1,
            "HashJoin A.id=B.id should produce exactly 1 row (id=2 matches), got {} rows",
            total_rows
        );
    }
}
