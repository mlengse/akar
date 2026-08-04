//! Auto-extracted from physical_operator.rs
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::PhysicalTypeID;
use akar_common::vector::DataChunk;
use std::sync::Arc;

// ==================== PhysicalExplain ====================

/// Physical EXPLAIN operator — serializes a logical plan tree to a human-readable
/// string and returns it as a single-row result.
///
/// Corresponds to C++ `PlanPrinter::printPlanToOstream` and `mapExplain`.
pub struct PhysicalExplain {
    /// The inner logical operator tree to serialize.
    pub inner_plan: String,
}

impl PhysicalOperatorExec for PhysicalExplain {
    fn operator_type(&self) -> &str {
        "explain"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let plan_str = self.inner_plan.clone();
        // Build an Arrow string array directly so the plan is never truncated
        // to the 255-byte inline string limit of the legacy value vector.
        let array = Arc::new(arrow::array::StringArray::from(vec![plan_str]));
        let arrow = akar_common::arrow_vector::ArrowVector::new(array, PhysicalTypeID::String);
        let chunk = DataChunk {
            fields: vec![arrow.array],
            field_types: vec![PhysicalTypeID::String],
            size: 1,
            field_names: vec![],
            sel_vector: None,
        };
        Ok(vec![chunk])
    }
}
