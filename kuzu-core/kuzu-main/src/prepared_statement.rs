//! PreparedStatement — parameterized query support.
//!
//! Allows preparing a query once and executing it multiple times with
//! different parameter values. The pipeline is:
//!
//! ```ignore
//! let stmt = conn.prepare("MATCH (p:Person) WHERE p.age > $min_age RETURN p.name")?;
//! let result = conn.execute(&stmt, vec![("min_age", Value::Int64(25))])?;
//! ```

use kuzu_binder::bound_statement::BoundStatement;
use kuzu_common::types::Value;
use kuzu_parser::ast::Expression;
use kuzu_planner::logical_operator::LogicalOperator;
use std::collections::HashMap;

/// A prepared statement with a cached bound statement and logical plan.
#[derive(Debug, Clone)]
pub struct PreparedStatement {
    /// The original query string.
    pub query: String,
    /// The bound statement after semantic analysis.
    pub bound_statement: BoundStatement,
    /// The logical plan (cached after first optimization).
    pub logical_plan: Option<Vec<LogicalOperator>>,
    /// Parameter names and their resolved types (None = unknown).
    pub parameters: Vec<String>,
}

impl PreparedStatement {
    pub fn new(query: String, bound_statement: BoundStatement) -> Self {
        // Extract parameter names from the bound statement
        let parameters = extract_parameters(&bound_statement);

        Self {
            query,
            bound_statement,
            logical_plan: None,
            parameters,
        }
    }

    /// Get the expected parameter names.
    pub fn parameter_names(&self) -> &[String] {
        &self.parameters
    }

    /// Number of expected parameters.
    pub fn num_parameters(&self) -> usize {
        self.parameters.len()
    }
}

/// Extract parameter names from a bound statement by walking the expression tree.
fn extract_parameters(bound: &BoundStatement) -> Vec<String> {
    let mut params = Vec::new();
    collect_params_from_statement(bound, &mut params);
    params.sort();
    params.dedup();
    params
}

fn collect_params_from_statement(bound: &BoundStatement, params: &mut Vec<String>) {
    match bound {
        BoundStatement::BoundQuery(q) => {
            for clause in &q.clauses {
                match clause {
                    kuzu_binder::bound_statement::BoundClause::BoundMatch(m) => {
                        for pattern in &m.patterns {
                            if let Some(_edge) = &pattern.edge {
                                // Edge patterns might have properties with params
                            }
                        }
                    }
                    kuzu_binder::bound_statement::BoundClause::BoundReturn(r) => {
                        for expr in &r.expressions {
                            collect_params_from_expr(&expr.expression, params);
                        }
                    }
                    kuzu_binder::bound_statement::BoundClause::BoundWhere(w) => {
                        collect_params_from_expr(&w.expression.expression, params);
                    }
                    kuzu_binder::bound_statement::BoundClause::BoundDelete(_) => {}
                }
            }
        }
        _ => {}
    }
}

fn collect_params_from_expr(expr: &Expression, params: &mut Vec<String>) {
    match expr {
        Expression::Parameter(name) => {
            params.push(name.clone());
        }
        Expression::PropertyAccess(obj, _) => {
            collect_params_from_expr(obj, params);
        }
        Expression::FunctionCall(_, args) => {
            for arg in args {
                collect_params_from_expr(arg, params);
            }
        }
        Expression::BinaryOp(_, left, right) => {
            collect_params_from_expr(left, params);
            collect_params_from_expr(right, params);
        }
        Expression::UnaryOp(_, inner) => {
            collect_params_from_expr(inner, params);
        }
        Expression::List(items) => {
            for item in items {
                collect_params_from_expr(item, params);
            }
        }
        Expression::Map(entries) => {
            for (_, val) in entries {
                collect_params_from_expr(val, params);
            }
        }
        Expression::Variable(_) | Expression::Constant(_) => {}
    }
}

/// Substitute parameter references with concrete values in an expression tree.
pub fn substitute_params(
    expr: &Expression,
    param_values: &HashMap<String, Value>,
) -> Result<Expression, String> {
    match expr {
        Expression::Parameter(name) => {
            let value = param_values
                .get(name)
                .ok_or_else(|| format!("Missing parameter: ${}", name))?;
            Ok(Expression::Constant(value_to_constant(value)))
        }
        Expression::PropertyAccess(obj, prop) => {
            let new_obj = substitute_params(obj, param_values)?;
            Ok(Expression::PropertyAccess(Box::new(new_obj), prop.clone()))
        }
        Expression::FunctionCall(name, args) => {
            let new_args: Result<Vec<_>, _> = args
                .iter()
                .map(|a| substitute_params(a, param_values))
                .collect();
            Ok(Expression::FunctionCall(name.clone(), new_args?))
        }
        Expression::BinaryOp(op, left, right) => {
            let new_left = substitute_params(left, param_values)?;
            let new_right = substitute_params(right, param_values)?;
            Ok(Expression::BinaryOp(*op, Box::new(new_left), Box::new(new_right)))
        }
        Expression::UnaryOp(op, inner) => {
            let new_inner = substitute_params(inner, param_values)?;
            Ok(Expression::UnaryOp(*op, Box::new(new_inner)))
        }
        Expression::List(items) => {
            let new_items: Result<Vec<_>, _> = items
                .iter()
                .map(|i| substitute_params(i, param_values))
                .collect();
            Ok(Expression::List(new_items?))
        }
        Expression::Map(entries) => {
                        let new_entries: Result<Vec<(String, Expression)>, String> = entries
                .iter()
                .map(|(k, v)| Ok((k.clone(), substitute_params(v, param_values)?)))
                .collect();
            Ok(Expression::Map(new_entries?))
        }
        // Non-parameter expressions pass through
        Expression::Variable(_) | Expression::Constant(_) => Ok(expr.clone()),
    }
}

/// Convert a Value to a Constant for expression substitution.
fn value_to_constant(value: &Value) -> kuzu_parser::ast::Constant {
    use kuzu_parser::ast::Constant;
    match value {
        Value::Null => Constant::Null,
        Value::Bool(b) => Constant::Bool(*b),
        Value::Int64(n) => Constant::Integer(*n),
        Value::Int32(n) => Constant::Integer(*n as i64),
        Value::Int16(n) => Constant::Integer(*n as i64),
        Value::Int8(n) => Constant::Integer(*n as i64),
        Value::UInt64(n) => Constant::Integer(*n as i64),
        Value::UInt32(n) => Constant::Integer(*n as i64),
        Value::UInt16(n) => Constant::Integer(*n as i64),
        Value::UInt8(n) => Constant::Integer(*n as i64),
        Value::Double(f) => Constant::Float(*f),
        Value::Float(f) => Constant::Float(*f as f64),
        Value::String(s) => Constant::String(s.clone()),
        Value::Blob(_) => Constant::String("<blob>".into()),
        _ => Constant::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuzu_common::types::Value;
    use kuzu_parser::ast::*;
    use std::collections::HashMap;

    #[test]
    fn test_extract_parameters_simple() {
        let expr = Expression::BinaryOp(
            BinaryOp::GreaterThan,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("p".into())),
                "age".into(),
            )),
            Box::new(Expression::Parameter("min_age".into())),
        );
        let mut params = Vec::new();
        collect_params_from_expr(&expr, &mut params);
        assert_eq!(params, vec!["min_age"]);
    }

    #[test]
    fn test_extract_multiple_params() {
        let expr = Expression::BinaryOp(
            BinaryOp::And,
            Box::new(Expression::BinaryOp(
                BinaryOp::GreaterThan,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("p".into())),
                    "age".into(),
                )),
                Box::new(Expression::Parameter("min_age".into())),
            )),
            Box::new(Expression::BinaryOp(
                BinaryOp::LessThan,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("p".into())),
                    "age".into(),
                )),
                Box::new(Expression::Parameter("max_age".into())),
            )),
        );
        let mut params = Vec::new();
        collect_params_from_expr(&expr, &mut params);
        params.sort();
        assert_eq!(params, vec!["max_age", "min_age"]);
    }

    #[test]
    fn test_substitute_params() {
        let expr = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::Variable("p".into())),
            Box::new(Expression::Parameter("name".into())),
        );
        let mut params = HashMap::new();
        params.insert("name".into(), Value::String("Alice".into()));

        let substituted = substitute_params(&expr, &params).unwrap();
        match substituted {
            Expression::BinaryOp(_, _, right) => {
                match *right {
                    Expression::Constant(Constant::String(s)) => {
                        assert_eq!(s, "Alice");
                    }
                    _ => panic!("Expected constant string"),
                }
            }
            _ => panic!("Expected binary op"),
        }
    }

    #[test]
    fn test_substitute_missing_param() {
        let expr = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::Variable("p".into())),
            Box::new(Expression::Parameter("missing".into())),
        );
        let params = HashMap::new();
        assert!(substitute_params(&expr, &params).is_err());
    }

    #[test]
    fn test_value_to_constant() {
        assert_eq!(value_to_constant(&Value::Int64(42)), Constant::Integer(42));
        assert_eq!(value_to_constant(&Value::String("hi".into())), Constant::String("hi".into()));
        assert_eq!(value_to_constant(&Value::Bool(true)), Constant::Bool(true));
        assert_eq!(value_to_constant(&Value::Double(3.14)), Constant::Float(3.14));
        assert_eq!(value_to_constant(&Value::Null), Constant::Null);
    }

    #[test]
    fn test_no_params() {
        let expr = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::Variable("a".into())),
            Box::new(Expression::Variable("b".into())),
        );
        let mut params = Vec::new();
        collect_params_from_expr(&expr, &mut params);
        assert!(params.is_empty());
    }
}
