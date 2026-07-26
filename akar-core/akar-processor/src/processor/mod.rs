//! Query processor — maps logical operators to physical operators and executes them.
//!
//! Pipeline execution model:
//! 1. Scan operators produce raw DataChunks
//! 2. Filter removes non-matching rows
//! 3. Projection selects/transforms columns
//! 4. Limit/OrderBy/Aggregate are applied last

pub mod chunk_helpers;
pub mod join_helpers;
pub mod mapper;
pub mod plan_serializer;
pub mod projection_helper;
pub mod union_helpers;

pub use chunk_helpers::*;
pub use join_helpers::*;
pub use mapper::*;
pub use plan_serializer::*;
pub use projection_helper::*;
pub use union_helpers::*;

use crate::physical_operator::*;
use akar_common::error::ProcessorError;
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};
use akar_function::registry::{FunctionRegistry, TableFunction};
use akar_planner::logical_operator::LogicalOperator;
use akar_storage::table::TableCatalog;
use std::sync::{Arc, Mutex};

pub type SequenceFn = Arc<dyn Fn(&str, bool) -> Result<Value, ProcessorError> + Send + Sync>;
pub type SubqueryFn = Arc<dyn Fn(&akar_parser::ast::Query) -> Result<Vec<DataChunk>, ProcessorError> + Send + Sync>;

/// DDL operations that require the schema-level Catalog (from Akar-catalog).
/// These are dispatched via callback because the processor layer doesn't
/// directly own the schema catalog.
#[derive(Debug, Clone)]
pub enum SchemaDdlOp {
    CreateSequence {
        name: String,
        if_not_exists: bool,
        start_value: i64,
        increment: i64,
        min_value: i64,
        max_value: i64,
        cycle: bool,
    },
    DropSequence {
        name: String,
        if_exists: bool,
    },
    ExportDatabase {
        file_path: String,
        file_type: String,
        schema_only: bool,
    },
    ImportDatabase {
        file_path: String,
        query: String,
        index_query: String,
    },
}
pub type SchemaDdlFn = Arc<dyn Fn(SchemaDdlOp) -> Result<String, ProcessorError> + Send + Sync>;

pub trait StandaloneCallHandler: Send + Sync {
    fn execute_call(
        &self,
        name: &str,
        args: &[akar_parser::ast::Expression],
    ) -> Result<Vec<akar_common::vector::DataChunk>, ProcessorError>;
}

pub trait StandaloneCallFn: Send + Sync {
    fn execute(&self, args: &[akar_parser::ast::Expression]) -> Result<Vec<Vec<akar_common::types::Value>>, ProcessorError>;
    fn aliases(&self) -> Vec<&'static str>;
}

#[derive(Default)]
pub struct StandaloneCallRegistry {
    handlers: std::collections::HashMap<String, std::sync::Arc<dyn StandaloneCallFn>>,
}

impl StandaloneCallRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, handler: std::sync::Arc<dyn StandaloneCallFn>) {
        for alias in handler.aliases() {
            self.handlers.insert(alias.to_lowercase(), handler.clone());
        }
    }

    pub fn get(&self, name: &str) -> Option<std::sync::Arc<dyn StandaloneCallFn>> {
        self.handlers.get(&name.to_lowercase()).cloned()
    }
}

/// The query processor executes a physical plan and produces result chunks.
pub struct QueryProcessor {
    function_registry: Option<Arc<Mutex<FunctionRegistry>>>,
    table_catalog: Option<Arc<TableCatalog>>,
    vfs: Option<Arc<akar_common::file_system::VirtualFileSystemRegistry>>,
    standalone_call_handler: Option<Arc<dyn StandaloneCallHandler>>,
    /// Callback for sequence operations (nextval/currval).
    /// Takes (sequence_name, is_nextval) and returns the resulting value.
    sequence_fn: Option<SequenceFn>,
    /// Callback for executing subqueries.
    subquery_fn: Option<SubqueryFn>,
    /// Callback for schema-level DDL operations (CREATE/DROP SEQUENCE, EXPORT/IMPORT DATABASE).
    schema_ddl_fn: Option<SchemaDdlFn>,
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
            schema_ddl_fn: None,
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
            schema_ddl_fn: None,
        }
    }

    /// Create a processor with function registry, table catalog access, and VFS.
    pub fn with_catalog(
        registry: Arc<Mutex<FunctionRegistry>>,
        table_catalog: Arc<TableCatalog>,
        vfs: Arc<akar_common::file_system::VirtualFileSystemRegistry>,
    ) -> Self {
        Self {
            function_registry: Some(registry),
            table_catalog: Some(table_catalog),
            vfs: Some(vfs),
            standalone_call_handler: None,
            sequence_fn: None,
            subquery_fn: None,
            schema_ddl_fn: None,
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

    /// Set the schema DDL callback (for CREATE/DROP SEQUENCE, EXPORT/IMPORT DATABASE).
    pub fn with_schema_ddl_fn(mut self, f: SchemaDdlFn) -> Self {
        self.schema_ddl_fn = Some(f);
        self
    }

    /// Execute a sequence of logical operators by mapping them to physical operators.
    pub fn execute(&self, operators: &[LogicalOperator]) -> Result<Vec<DataChunk>, ProcessorError> {
        let mut sip_masks = std::collections::HashMap::new();
        self.execute_internal(operators, &mut sip_masks)
    }

    pub fn execute_internal(
        &self,
        operators: &[LogicalOperator],
        sip_masks: &mut std::collections::HashMap<u64, NodeSemiMask>,
    ) -> Result<Vec<DataChunk>, ProcessorError> {
        if operators.is_empty() {
            return Ok(vec![DataChunk {
                fields: vec![],
                field_types: vec![],
                size: 0,
                field_names: vec![],
                sel_vector: None,
            }]);
        }

        let mut intermediate_result: Option<Vec<DataChunk>> = None;

        for (i, op) in operators.iter().enumerate() {
            let current = intermediate_result.take().unwrap_or_else(|| {
                let mut dummy = DataChunk::new(vec![], vec![]);
                dummy.size = 1;
                vec![dummy]
            });
            let next_op = operators.get(i + 1);

            let mut ctx = mapper::ExecutionContext {
                processor: self,
                sip_masks,
                function_registry: self.function_registry.clone(),
                table_catalog: self.table_catalog.clone(),
                vfs: self.vfs.clone(),
                standalone_call_handler: self.standalone_call_handler.clone(),
                sequence_fn: self.sequence_fn.clone(),
                subquery_fn: self.subquery_fn.clone(),
                schema_ddl_fn: self.schema_ddl_fn.clone(),
            };

            let result = mapper::PlanMapper::map_and_execute(op, next_op, current, &mut ctx)?;

            if let LogicalOperator::ScanRel(_) = op {
                // Accumulate: extend rather than replace for ScanRel
                match &mut intermediate_result {
                    Some(existing) => existing.extend(result),
                    None => intermediate_result = Some(result),
                }
            } else {
                intermediate_result = Some(result);
            }
        }

        Ok(intermediate_result.unwrap_or_default())
    }

    /// Execute a table function call by looking up the function in the registry
    /// and dispatching to the appropriate handler.
    fn execute_table_function(
        &self,
        tf: &akar_planner::logical_operator::LogicalTableFunctionCall,
    ) -> Result<Vec<DataChunk>, ProcessorError> {
        let func_name = &tf.function_name;
        let args: Vec<Value> = Vec::new(); // args would be evaluated from expressions

        // Look up the function in the registry
        if let Some(ref registry) = self.function_registry {
            let reg = registry.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
            if let Some(tbl_fn) = reg.get_table(func_name) {
                match tbl_fn {
                    TableFunction::CustomTable { execute, .. } => {
                        let mut chunk = DataChunk::new(Vec::new(), Vec::new());
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
                    ).into()),
                    TableFunction::Custom { name } if name == "vector_similarity_scan" => {
                        // Evaluate args: [table_name, column_name, query_vector, top_k]
                        // For CALL statement, args are parsed as expressions. We need to evaluate them.
                        // For now, parse from the function args
                        drop(reg);
                        self.execute_vector_similarity_scan(tf)
                    }
                    TableFunction::Custom { name } => {
                        Err(format!("Custom table function '{}' has no registered handler", name).into())
                    }
                }
            } else {
                Err(format!("Table function '{}' not found", func_name).into())
            }
        } else {
            Err(format!(
                "Cannot execute table function '{}': no function registry available",
                func_name
            ).into())
        }
    }

    /// Execute a `vector_similarity_scan` table function call.
    ///
    /// Expects CALL vector_similarity_scan(table_name, column_name, query_vector, top_k)
    /// and dispatches to PhysicalVectorSimilarityScan with the processor's TableCatalog.
    fn execute_vector_similarity_scan(
        &self,
        tf: &akar_planner::logical_operator::LogicalTableFunctionCall,
    ) -> Result<Vec<DataChunk>, ProcessorError> {
        // Evaluate arguments from expressions (they should be constants or simple vars)
        if tf.args.len() < 4 {
            return Err(
                "vector_similarity_scan requires 4 arguments: table_name, column_name, query_vector, top_k".into(),
            );
        }

        // For CALL statements, args arrive as Expression AST nodes.
        // Evaluate them to Values. The simplest approach: evaluate constants inline.
        fn eval_expr_to_value(expr: &akar_parser::ast::Expression) -> Option<Value> {
            match expr {
                akar_parser::ast::Expression::Constant(c) => match c {
                    akar_parser::ast::Constant::String(s) => Some(Value::String(s.clone())),
                    akar_parser::ast::Constant::Integer(i) => Some(Value::Int64(*i)),
                    akar_parser::ast::Constant::Float(f) => Some(Value::Double(*f)),
                    akar_parser::ast::Constant::Bool(b) => Some(Value::Bool(*b)),
                    akar_parser::ast::Constant::Null => Some(Value::Null),
                },
                akar_parser::ast::Expression::List(items) => {
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
        _expr: &akar_parser::ast::Expression,
        _chunk: &DataChunk,
    ) -> Result<ValueVector, ProcessorError> {
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
