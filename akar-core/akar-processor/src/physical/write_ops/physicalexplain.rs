//! Auto-extracted from physical_operator.rs
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::vector::DataChunk;

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
        use akar_common::types::PhysicalTypeID;
        use akar_common::vector::{DataChunk, ValueVector};

        let plan_str = self.inner_plan.clone();
        let mut vv = ValueVector::new(PhysicalTypeID::String, 1);
        vv.resize(1);
        let bytes = plan_str.as_bytes();
        let copy_len = bytes.len().min(255);
        vv.data_mut()[0] = copy_len as u8;
        if copy_len > 0 {
            vv.data_mut()[1..1 + copy_len].copy_from_slice(&bytes[..copy_len]);
        }
        // For long strings, store the full string in the ValueVector's overflow
        // We store the original Value for the query result
        let chunk = DataChunk {
            fields: vec![akar_common::arrow_vector::ArrowVector::from_legacy(&vv).array],
            field_types: vec![vv.physical_type()],
            size: 1,
            field_names: vec![],
            sel_vector: None,
        };
        Ok(vec![chunk])
    }
}
