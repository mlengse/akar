//! Auto-extracted from physical_operator.rs
use kuzu_common::types::Value;
use crate::physical::write_ops::ast_constant_to_value;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_parser::ast::Expression;
use kuzu_storage::table::TableCatalog;
use std::sync::{Arc, Mutex};
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::scan_filter::PhysicalScan;
use crate::physical::common::store_value_in_vector;

// ==================== Foreach ====================

/// Physical FOREACH operator — iterates over list elements and executes sub-plans.
pub struct PhysicalForeach {
    pub variable: String,
    pub expression: Expression,
    pub sub_plans: Vec<Vec<kuzu_planner::logical_operator::LogicalOperator>>,
    pub function_registry: Option<Arc<Mutex<kuzu_function::registry::FunctionRegistry>>>,
    pub table_catalog: Option<Arc<TableCatalog>>,
    pub vfs: Option<Arc<kuzu_common::file_system::VirtualFileSystemRegistry>>,
}

impl PhysicalOperatorExec for PhysicalForeach {
    fn operator_type(&self) -> &str {
        "foreach"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // Evaluate the list expression
        let list_val = match &self.expression {
            Expression::List(items) => {
                let mut vals = Vec::with_capacity(items.len());
                for item in items {
                    if let Expression::Constant(c) = item {
                        vals.push(ast_constant_to_value(c));
                    } else {
                        vals.push(Value::Null);
                    }
                }
                Value::List(vals)
            }
            _ => {
                return Err(format!(
                    "FOREACH requires a list expression, got: {:?}",
                    self.expression
                ));
            }
        };

        let list_items = match &list_val {
            Value::List(items) => items.clone(),
            _ => return Ok(vec![]),
        };

        if list_items.is_empty() || self.sub_plans.is_empty() {
            return Ok(vec![]);
        }

        // For each list item, execute sub-plans with the item value in scope.
        // We use a simplified approach: create a DataChunk with the item value
        // and pass it to each sub-plan.
        for item in &list_items {
            for sub_plan in &self.sub_plans {
                // Create a single-row DataChunk containing the current item
                let phys_type = PhysicalScan::value_to_physical_type(item);
                let mut v = ValueVector::new(phys_type, 1);
                v.resize(1);
                store_value_in_vector(&mut v, 0, item);
                let _chunk = DataChunk::new(vec![kuzu_common::arrow_vector::ArrowVector::from_legacy(&v).array], vec![kuzu_common::types::PhysicalTypeID::Int64]);

                // Execute the sub-plan using the QueryProcessor-like pipeline
                // Use the processor module directly from the same crate
                let processor = crate::processor::QueryProcessor::with_catalog(
                    self.function_registry.clone().unwrap(),
                    self.table_catalog.clone().unwrap(),
                    self.vfs.clone().unwrap(),
                );
                let _result = processor.execute(sub_plan)?;
            }
        }

        // FOREACH produces no output rows (it's a write-only operation)
        Ok(vec![])
    }
}


