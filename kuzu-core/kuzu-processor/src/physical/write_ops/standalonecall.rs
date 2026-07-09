use kuzu_common::vector::DataChunk;
use kuzu_parser::ast::Expression;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

pub struct PhysicalStandaloneCall {
    pub function_name: String,
    pub args: Vec<Expression>,
    pub handler: std::sync::Arc<dyn crate::processor::StandaloneCallHandler>,
}

impl PhysicalOperatorExec for PhysicalStandaloneCall {
    fn operator_type(&self) -> &str {
        "standalone_call"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        self.handler.execute_call(&self.function_name, &self.args)
    }
}
