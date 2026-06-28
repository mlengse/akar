//! Function registry — manages lookup and registration of built-in functions.

use hashbrown::HashMap;

/// A simple scalar function identifier.
#[derive(Debug, Clone)]
pub enum ScalarFunction {
    Arithmetic { op: ArithmeticOp },
    Comparison { op: ComparisonOp },
    String { op: StringOp },
    Cast,
}

#[derive(Debug, Clone, Copy)]
pub enum ArithmeticOp { Add, Sub, Mul, Div }

#[derive(Debug, Clone, Copy)]
pub enum ComparisonOp { Eq, Lt, Gt }

#[derive(Debug, Clone, Copy)]
pub enum StringOp { Concat, Contains, StartsWith, EndsWith, ToUpper, ToLower, Trim }

/// Registry of all built-in functions.
#[derive(Default)]
pub struct FunctionRegistry {
    scalar_functions: HashMap<String, ScalarFunction>,
    aggregate_functions: HashMap<String, ScalarFunction>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        let mut reg = Self::default();
        reg.register_builtins();
        reg
    }

    fn register_builtins(&mut self) {
        self.register_scalar("+", ScalarFunction::Arithmetic { op: ArithmeticOp::Add });
        self.register_scalar("-", ScalarFunction::Arithmetic { op: ArithmeticOp::Sub });
        self.register_scalar("*", ScalarFunction::Arithmetic { op: ArithmeticOp::Mul });
        self.register_scalar("/", ScalarFunction::Arithmetic { op: ArithmeticOp::Div });
        self.register_scalar("=", ScalarFunction::Comparison { op: ComparisonOp::Eq });
        self.register_scalar("<", ScalarFunction::Comparison { op: ComparisonOp::Lt });
        self.register_scalar(">", ScalarFunction::Comparison { op: ComparisonOp::Gt });
        self.register_scalar("concat", ScalarFunction::String { op: StringOp::Concat });
        self.register_scalar("contains", ScalarFunction::String { op: StringOp::Contains });
        self.register_scalar("starts_with", ScalarFunction::String { op: StringOp::StartsWith });
        self.register_scalar("ends_with", ScalarFunction::String { op: StringOp::EndsWith });
        self.register_scalar("to_upper", ScalarFunction::String { op: StringOp::ToUpper });
        self.register_scalar("to_lower", ScalarFunction::String { op: StringOp::ToLower });
        self.register_scalar("trim", ScalarFunction::String { op: StringOp::Trim });
        self.register_scalar("cast", ScalarFunction::Cast);
    }

    pub fn register_scalar(&mut self, name: &str, func: ScalarFunction) {
        self.scalar_functions.insert(name.to_lowercase(), func);
    }

    pub fn get_scalar(&self, name: &str) -> Option<&ScalarFunction> {
        self.scalar_functions.get(&name.to_lowercase())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.scalar_functions.contains_key(&name.to_lowercase())
            || self.aggregate_functions.contains_key(&name.to_lowercase())
    }
}
