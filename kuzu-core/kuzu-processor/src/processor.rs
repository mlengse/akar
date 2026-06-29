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
use kuzu_storage::table::{TableCatalog, ColumnDefinition};
use std::sync::{Arc, Mutex};

/// The query processor executes a physical plan and produces result chunks.
pub struct QueryProcessor {
    function_registry: Option<Arc<Mutex<FunctionRegistry>>>,
    table_catalog: Option<Arc<Mutex<TableCatalog>>>,
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
    pub fn with_catalog(
        registry: Arc<Mutex<FunctionRegistry>>,
        table_catalog: Arc<Mutex<TableCatalog>>,
    ) -> Self {
        Self {
            function_registry: Some(registry),
            table_catalog: Some(table_catalog),
        }
    }

    /// Resolve table data and column definitions for a scan node.
    fn resolve_scan_data(
        &self,
        table_name: &str,
    ) -> (Option<Vec<Vec<Value>>>, Vec<ColumnDefinition>, u64) {
        if let Some(ref tc) = self.table_catalog {
            let catalog = tc.lock().unwrap();
            // Try node table first
            if let Some(node_table) = catalog.get_node_table_by_name(table_name) {
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
            if let Some(rel_table) = catalog.get_rel_table_by_name(table_name) {
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
    pub fn execute(
        &self,
        operators: &[LogicalOperator],
    ) -> Result<Vec<DataChunk>, String> {
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
                    let mut scan = PhysicalScan::new(
                        s.table_name.clone(),
                        s.table_id,
                        num_rows.max(1),
                    );
                    if let Some(d) = data {
                        scan = scan.with_data(d, columns);
                    }
                    let result = scan.execute(current.clone())?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::ScanRel(s) => {
                    let (data, columns, num_rows) = self.resolve_scan_data(&s.table_name);
                    let mut scan = PhysicalScan::new(
                        s.table_name.clone(),
                        s.table_id,
                        num_rows.max(1),
                    );
                    if let Some(d) = data {
                        scan = scan.with_data(d, columns);
                    }
                    let result = scan.execute(current.clone())?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Filter(f) => {
                    let evaluator = self.function_registry.clone()
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
                    let order = PhysicalOrderBy {
                        sort_column: 0,
                        ascending: o.sort_keys.first().map(|s| s.1).unwrap_or(true),
                    };
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
                    let table_catalog = self.table_catalog.clone()
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
                    let table_catalog = self.table_catalog.clone()
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
                LogicalOperator::CrossProduct(_)
                | LogicalOperator::Union(_) => {
                    intermediate_result = Some(vec![]);
                }
                LogicalOperator::CopyFrom(cf) => {
                    let table_catalog = self.table_catalog.clone()
                        .ok_or_else(|| "No table catalog available for COPY FROM".to_string())?;

                    // Get column definitions from the table catalog
                    let catalog = table_catalog.lock().unwrap();
                    let columns = if let Some(node_table) = catalog.get_node_table_by_name(&cf.table_name) {
                        node_table.columns.clone()
                    } else if let Some(rel_table) = catalog.get_rel_table_by_name(&cf.table_name) {
                        rel_table.columns.clone()
                    } else {
                        return Err(format!("Table '{}' not found in storage catalog", cf.table_name));
                    };
                    drop(catalog);

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
                    | TableFunction::CurrentSetting { .. }
                    | TableFunction::Custom { .. } => {
                        Err(format!("Table function '{}' cannot be executed dynamically (no callback)", func_name))
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
        let catalog = Arc::new(Mutex::new(TableCatalog::new()));
        {
            let mut cat = catalog.lock().unwrap();
            cat.create_node_table("Person".into(), vec![
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
            ]);
            // Insert some data
            let table = cat.get_node_table_by_name_mut("Person").unwrap();
            table.insert_row(vec![Value::String("Alice".into()), Value::Int64(30)]).unwrap();
            table.insert_row(vec![Value::String("Bob".into()), Value::Int64(25)]).unwrap();
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
        let order = PhysicalOrderBy { sort_column: 0, ascending: true };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        let vals = [5, 3, 1, 4, 2];
        for i in 0..5 { v.set_i64(i, vals[i]); }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = order.execute(input).unwrap();
        assert!(!result.is_empty());
        let sorted = result[0].fields[0].get_i64(0).unwrap();
        assert_eq!(sorted, 1); // Min should be first
    }

    #[test]
    fn test_order_by_descending() {
        let order = PhysicalOrderBy { sort_column: 0, ascending: false };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        let vals = [5, 3, 1, 4, 2];
        for i in 0..5 { v.set_i64(i, vals[i]); }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = order.execute(input).unwrap();
        assert!(!result.is_empty());
        let sorted = result[0].fields[0].get_i64(0).unwrap();
        assert_eq!(sorted, 5); // Max should be first
    }

    #[test]
    fn test_order_by_empty_input() {
        let order = PhysicalOrderBy { sort_column: 0, ascending: true };
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
        for i in 0..5 { v.set_i64(i, i as i64); }
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
        for i in 0..4 { v.set_i64(i, (i + 1) as i64); }
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
        for i in 0..5 { v.set_i64(i, vals[i]); }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(3));  // MIN = 3
        assert_eq!(result[0].fields[1].get_value(0).unwrap(), Value::Int64(99)); // MAX = 99
    }

    #[test]
    fn test_aggregate_avg() {
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["AVG".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 4);
        for i in 0..4 { v.set_i64(i, (i + 1) as i64); }
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
        for i in 0..3 { build.set_i64(i, (i + 1) as i64); }
        build.resize(3);
        // Probe side: keys [2, 3, 4]
        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 3);
        probe.set_i64(0, 2); probe.set_i64(1, 3); probe.set_i64(2, 4);
        probe.resize(3);
        let input = vec![
            DataChunk::new(vec![build]),
            DataChunk::new(vec![probe]),
        ];
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
        build.set_i64(0, 1); build.set_i64(1, 2);
        build.resize(2);
        // Probe: [3, 4]
        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 2);
        probe.set_i64(0, 3); probe.set_i64(1, 4);
        probe.resize(2);
        let input = vec![
            DataChunk::new(vec![build]),
            DataChunk::new(vec![probe]),
        ];
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
        probe.set_i64(0, 1); probe.set_i64(1, 2); probe.set_i64(2, 3);
        probe.resize(3);
        let input = vec![
            DataChunk::new(vec![build]),
            DataChunk::new(vec![probe]),
        ];
        let result = join.execute(input).unwrap();
        assert!(result.is_empty()); // Empty build → no matches
    }
}
