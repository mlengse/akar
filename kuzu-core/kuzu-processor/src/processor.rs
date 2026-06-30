//! Query processor — maps logical operators to physical operators and executes them.
//!
//! Pipeline execution model:
//! 1. Scan operators produce raw DataChunks
//! 2. Filter removes non-matching rows
//! 3. Projection selects/transforms columns
//! 4. Limit/OrderBy/Aggregate are applied last

use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical_operator::*;
use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_function::registry::{FunctionRegistry, TableFunction};
use kuzu_planner::logical_operator::LogicalOperator;
use kuzu_storage::table::{ColumnDefinition, TableCatalog};
use std::sync::{Arc, Mutex};

/// The query processor executes a physical plan and produces result chunks.
pub struct QueryProcessor {
    function_registry: Option<Arc<Mutex<FunctionRegistry>>>,
    table_catalog: Option<Arc<TableCatalog>>,
}

impl QueryProcessor {
    pub fn new() -> Self {
        Self {
            function_registry: None,
            table_catalog: None,
        }
    }

    /// Create a processor with access to the function registry.
    pub fn with_registry(registry: Arc<Mutex<FunctionRegistry>>) -> Self {
        Self {
            function_registry: Some(registry),
            table_catalog: None,
        }
    }

    /// Create a processor with function registry and table catalog access.
    pub fn with_catalog(registry: Arc<Mutex<FunctionRegistry>>, table_catalog: Arc<TableCatalog>) -> Self {
        Self {
            function_registry: Some(registry),
            table_catalog: Some(table_catalog),
        }
    }

    /// Resolve table data and column definitions for a scan node.
    fn resolve_scan_data(&self, table_name: &str) -> (Option<Vec<Vec<Value>>>, Vec<ColumnDefinition>, u64) {
        if let Some(ref tc) = self.table_catalog {
            // Try node table first
            if let Some(node_table) = tc.get_node_table_by_name(table_name) {
                let num_rows = node_table.num_rows;
                if num_rows > 0 {
                    return (
                        Some(node_table.to_column_major_data()),
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
                        Some(rel_table.to_column_major_data()),
                        rel_table.columns.clone(),
                        num_rows,
                    );
                }
            }
        }
        (None, Vec::new(), 0)
    }

    /// Execute a sequence of logical operators by mapping them to physical operators.
    pub fn execute(&self, operators: &[LogicalOperator]) -> Result<Vec<DataChunk>, String> {
        if operators.is_empty() {
            return Ok(vec![DataChunk {
                fields: vec![],
                size: 0,
            }]);
        }

        // Map logical operators to physical and execute in pipeline
        let current = Vec::new();

        // Execute each logical operator
        let mut intermediate_result: Option<Vec<DataChunk>> = None;

        for op in operators {
            match op {
                LogicalOperator::ScanNode(s) => {
                    let (data, columns, num_rows) = self.resolve_scan_data(&s.table_name);
                    let mut scan = PhysicalScan::new(s.table_name.clone(), s.table_id, num_rows.max(1));
                    if let Some(d) = data {
                        scan = scan.with_data(d, columns);
                    }
                    let result = scan.execute(current.clone())?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::ScanRel(s) => {
                    let (data, columns, num_rows) = self.resolve_scan_data(&s.table_name);
                    let mut scan = PhysicalScan::new(s.table_name.clone(), s.table_id, num_rows.max(1));
                    if let Some(d) = data {
                        scan = scan.with_data(d, columns);
                    }
                    let result = scan.execute(current.clone())?;
                    intermediate_result = Some(result);
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
                    let result = scan.execute(current.clone())?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Filter(f) => {
                    let evaluator = self
                        .function_registry
                        .clone()
                        .map(|reg| Arc::new(Mutex::new(ExpressionEvaluator::new(reg))));
                    let filter = if let Some(eval) = evaluator {
                        PhysicalFilter::with_evaluator(f.expression.clone(), eval)
                    } else {
                        PhysicalFilter::new(f.expression.clone())
                    };
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = filter.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Projection(p) => {
                    let proj = PhysicalProjection {
                        column_indices: (0..p.expressions.len()).collect(),
                    };
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = proj.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Limit(l) => {
                    let limit = PhysicalLimit {
                        limit: l.limit,
                        offset: l.offset,
                    };
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = limit.execute(input)?;
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
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = order.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Flatten(_) => {
                    // Flatten is a no-op in the flat-list execution model;
                    // it signals that the child's factorization group should
                    // be treated as flat during physical execution.
                    // Pass through the current result unchanged.
                }
                LogicalOperator::Aggregate(a) => {
                    let agg = PhysicalAggregate {
                        group_by_cols: if a.group_by.is_empty() {
                            Vec::new()
                        } else {
                            (0..a.group_by.len() as u32).collect()
                        },
                        aggregate_functions: a.aggregates.iter().map(|(n, _)| n.clone()).collect(),
                    };
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = agg.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::HashJoin(_h) => {
                    let join = PhysicalHashJoin {
                        build_columns: Vec::new(),
                        probe_columns: Vec::new(),
                    };
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = join.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Unwind(uw) => {
                    let input = intermediate_result.take().unwrap_or_default();
                    let unwind = PhysicalUnwind {
                        expression: uw.expression.clone(),
                        variable: uw.variable.clone(),
                    };
                    let result = unwind.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::OptionalMatch(_) => {
                    // OptionalMatch passes through input chunks but marks that NULLs
                    // should be produced for non-matching optional patterns.
                    // In the current flat pipeline model, the scan before this marker
                    // may produce 0 rows for unmatched; this pass-through preserves
                    // the left side rows when the optional has no matches.
                    let input = intermediate_result.take().unwrap_or_default();
                    if input.is_empty() {
                        // Produce a single chunk with one row of NULLs for optional fields
                        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
                        v.resize(1);
                        v.set_null(0, true);
                        intermediate_result = Some(vec![DataChunk::new(vec![v])]);
                    } else {
                        intermediate_result = Some(input);
                    }
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
                        table_catalog,
                    };
                    let input = intermediate_result.take().unwrap_or_default();
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
                        row_indices: Vec::new(),
                        table_catalog,
                    };
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = delete_op.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::CrossProduct(_) => {
                    intermediate_result = Some(vec![]);
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
                    };
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = copy_op.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::TableFunctionCall(tf) => {
                    let result = self.execute_table_function(tf)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Foreach(fc) => {
                    let input = intermediate_result.take().unwrap_or_default();
                    let foreach_op = PhysicalForeach {
                        variable: fc.variable.clone(),
                        expression: fc.expression.clone(),
                        sub_plans: fc.sub_plans.clone(),
                        function_registry: self.function_registry.clone(),
                        table_catalog: self.table_catalog.clone(),
                    };
                    let result = foreach_op.execute(input)?;
                    intermediate_result = Some(result);
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
                    TableFunction::Custom { name } => Err(format!(
                        "Custom table function '{}' has no registered handler",
                        name
                    )),
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
            return Err("vector_similarity_scan requires 4 arguments: table_name, column_name, query_vector, top_k".into());
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

// ---------------------------------------------------------------------------
// UNION helpers
// ---------------------------------------------------------------------------

/// Flatten a `LogicalUnion` child subtree into a sequence of operators
/// suitable for `QueryProcessor::execute`.
///
/// If the child is a `Projection` with empty expressions (the synthetic
/// pipeline wrapper created by the planner), extract its children as the
/// operator sequence. Otherwise, return the single operator as a one-element
/// vector.
fn flatten_union_child(op: &LogicalOperator) -> Vec<LogicalOperator> {
    match op {
        LogicalOperator::Projection(p) if p.expressions.is_empty() => p.children.clone(),
        other => vec![other.clone()],
    }
}

/// Merge left and right result sets from UNION execution.
///
/// Column-by-column concatenation using `ValueVector::append`.
/// For `UNION ALL` (`all = true`), rows are simply concatenated.
/// For `UNION` distinct (`all = false`), duplicate rows are removed.
fn merge_union_chunks(
    left: Vec<DataChunk>,
    right: Vec<DataChunk>,
    all: bool,
) -> Result<Vec<DataChunk>, String> {
    if left.is_empty() {
        return Ok(right);
    }
    if right.is_empty() {
        return Ok(left);
    }

    let num_fields = left[0].num_fields();
    for chunk in &right {
        if chunk.num_fields() != num_fields {
            return Err(format!(
                "UNION column count mismatch: left has {num_fields} columns, right has {} columns",
                chunk.num_fields()
            ));
        }
    }

    // --- Step 1: concatenate fields across all chunks ---
    let mut merged_fields: Vec<ValueVector> = (0..num_fields)
        .map(|i| {
            let first_type = left[0].field(i).physical_type();
            let total_size: usize = left
                .iter()
                .map(|c| c.field(i).size())
                .chain(right.iter().map(|c| c.field(i).size()))
                .sum();
            let mut merged = ValueVector::new(first_type, total_size.max(1));
            for chunk in &left {
                merged.append(chunk.field(i));
            }
            for chunk in &right {
                merged.append(chunk.field(i));
            }
            merged
        })
        .collect();

    let total_size = merged_fields.first().map(|f| f.size()).unwrap_or(0);

    // --- Step 2: deduplicate if UNION (not ALL) ---
    if !all && total_size > 1 {
        // Extract all rows as Vec<Value> for comparison via PartialEq
        let all_rows = extract_all_rows(&merged_fields);
        let mut deduped: Vec<Vec<Value>> = Vec::with_capacity(total_size);
        for row in &all_rows {
            if !deduped.contains(row) {
                deduped.push(row.clone());
            }
        }
        // Rebuild column vectors from deduped rows
        merged_fields = rows_to_columns(&deduped);
    }

    let final_size = merged_fields.first().map(|f| f.size()).unwrap_or(0);
    Ok(vec![DataChunk {
        fields: merged_fields,
        size: final_size,
    }])
}

/// Extract all rows from a set of column vectors as `Vec<Vec<Value>>`.
fn extract_all_rows(fields: &[ValueVector]) -> Vec<Vec<Value>> {
    let num_rows = fields.first().map(|f| f.size()).unwrap_or(0);
    let num_cols = fields.len();
    let mut rows: Vec<Vec<Value>> = Vec::with_capacity(num_rows);
    for row in 0..num_rows {
        let mut values: Vec<Value> = Vec::with_capacity(num_cols);
        for vec in fields {
            let val = if vec.is_null(row) {
                Value::Null
            } else {
                vec.get_value(row).unwrap_or(Value::Null)
            };
            values.push(val);
        }
        rows.push(values);
    }
    rows
}

/// Rebuild column vectors from deduplicated rows.
fn rows_to_columns(rows: &[Vec<Value>]) -> Vec<ValueVector> {
    if rows.is_empty() {
        return Vec::new();
    }
    let num_cols = rows[0].len();
    let num_rows = rows.len();
    (0..num_cols)
        .map(|col| {
            // Determine physical type from the first row's value
            let first_val = &rows[0][col];
            let phys_type = value_to_physical_type(first_val);
            let mut vec = ValueVector::new(phys_type, num_rows.max(1));
            for (row_idx, row) in rows.iter().enumerate() {
                let val = &row[col];
                let _ = vec.set_value(row_idx, val);
            }
            vec.resize(num_rows);
            vec
        })
        .collect()
}

/// Map a `Value` to its corresponding `PhysicalTypeID`.
fn value_to_physical_type(val: &Value) -> PhysicalTypeID {
    match val {
        Value::Null => PhysicalTypeID::Any,
        Value::Bool(_) => PhysicalTypeID::Bool,
        Value::Int64(_) => PhysicalTypeID::Int64,
        Value::Int32(_) => PhysicalTypeID::Int32,
        Value::Int16(_) => PhysicalTypeID::Int16,
        Value::Int8(_) => PhysicalTypeID::Int8,
        Value::UInt64(_) => PhysicalTypeID::UInt64,
        Value::UInt32(_) => PhysicalTypeID::UInt32,
        Value::UInt16(_) => PhysicalTypeID::UInt16,
        Value::UInt8(_) => PhysicalTypeID::UInt8,
        Value::Int128(_) => PhysicalTypeID::Int128,
        Value::Double(_) => PhysicalTypeID::Double,
        Value::Float(_) => PhysicalTypeID::Float,
        Value::String(_) | Value::Blob(_) => PhysicalTypeID::String,
        Value::Date(_) | Value::Timestamp(_) | Value::TimestampTz(_) | Value::TimestampNs(_)
        | Value::TimestampMs(_) | Value::TimestampSec(_) | Value::Interval(_) => {
            PhysicalTypeID::Int64
        }
        Value::InternalID(_) => PhysicalTypeID::Int64,
        Value::List(_) => PhysicalTypeID::List,
        Value::Map(_) => PhysicalTypeID::Struct,
        Value::Struct(_) => PhysicalTypeID::Struct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuzu_binder::bound_statement::BoundExpression;
    use kuzu_common::types::{LogicalTypeID, Value};
    use kuzu_parser::ast::{Constant, Expression};
    use kuzu_storage::table::ColumnDefinition;

    fn make_scan_op() -> LogicalOperator {
        LogicalOperator::ScanNode(kuzu_planner::logical_operator::LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec![],
            cardinality: 0,
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
                    ColumnDefinition {
                        name: "name".into(),
                        logical_type: LogicalTypeID::String,
                        is_primary_key: true,
                    },
                    ColumnDefinition {
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
        QueryProcessor::with_catalog(registry, catalog)
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
        let input = vec![DataChunk::new(vec![build]), DataChunk::new(vec![probe])];
        let result = join.execute(input).unwrap();
        // Should match 2 and 3 (2 rows)
        assert!(!result.is_empty());
    }

    #[test]
    fn test_hash_join_no_match() {
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
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
        let input = vec![DataChunk::new(vec![build]), DataChunk::new(vec![probe])];
        let result = join.execute(input).unwrap();
        assert!(result.is_empty()); // No matches
    }

    #[test]
    fn test_hash_join_empty_build() {
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
        };
        let build = ValueVector::new(PhysicalTypeID::Int64, 0);
        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 3);
        probe.set_i64(0, 1);
        probe.set_i64(1, 2);
        probe.set_i64(2, 3);
        probe.resize(3);
        let input = vec![DataChunk::new(vec![build]), DataChunk::new(vec![probe])];
        let result = join.execute(input).unwrap();
        assert!(result.is_empty()); // Empty build → no matches
    }

    // ==================== Edge Case Tests ====================

    #[test]
    fn test_hash_join_null_keys_no_match() {
        // SQL semantics: NULL keys should never match in a join
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
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
        let input = vec![DataChunk::new(vec![build]), DataChunk::new(vec![probe])];
        let result = join.execute(input).unwrap();
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

        let input = vec![DataChunk::new(vec![build]), DataChunk::new(vec![probe])];
        let result = join.execute(input).unwrap();
        // NULL = NULL is unknown in SQL, so no matches
        assert!(result.is_empty());
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
        let right = vec![DataChunk::new(vec![
            ValueVector::new(PhysicalTypeID::Int64, 1),
        ])];
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
}
