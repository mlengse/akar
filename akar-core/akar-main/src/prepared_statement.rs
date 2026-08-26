//! PreparedStatement — parameterized query support.
//!
//! Allows preparing a query once and executing it multiple times with
//! different parameter values.

use akar_binder::bound_statement::{BoundMatchClause, BoundStatement};
use akar_common::types::Value;
use akar_parser::ast::{Clause, Expression, Query, ReturnClause, ReturnItem, WhereClause};
use akar_planner::logical_operator::LogicalOperator;
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
                    akar_binder::bound_statement::BoundClause::BoundMatch(m)
                    | akar_binder::bound_statement::BoundClause::BoundOptionalMatch(m)
                    | akar_binder::bound_statement::BoundClause::BoundCreate(m) => {
                        collect_params_from_match_clause(m, params);
                    }
                    akar_binder::bound_statement::BoundClause::BoundReturn(r)
                    | akar_binder::bound_statement::BoundClause::BoundWith(r) => {
                        for expr in &r.expressions {
                            collect_params_from_expr(&expr.expression, params);
                        }
                        if let Some(order_by) = &r.order_by {
                            for item in order_by {
                                collect_params_from_expr(&item.expression.expression, params);
                            }
                        }
                    }
                    akar_binder::bound_statement::BoundClause::BoundWhere(w) => {
                        collect_params_from_expr(&w.expression.expression, params);
                    }
                    akar_binder::bound_statement::BoundClause::BoundDelete(d) => {
                        for item in &d.items {
                            collect_params_from_expr(&item.expression, params);
                        }
                    }
                    akar_binder::bound_statement::BoundClause::BoundSet(s) => {
                        for item in &s.items {
                            collect_params_from_expr(&item.property, params);
                            collect_params_from_expr(&item.value, params);
                        }
                    }
                    akar_binder::bound_statement::BoundClause::BoundUnwind(u) => {
                        collect_params_from_expr(&u.expression, params);
                    }
                    akar_binder::bound_statement::BoundClause::BoundForeach(f) => {
                        collect_params_from_expr(&f.expression, params);
                        for sub in &f.sub_statements {
                            collect_params_from_statement(sub, params);
                        }
                    }
                    akar_binder::bound_statement::BoundClause::BoundMerge(m) => {
                        for (_, v) in &m.properties {
                            collect_params_from_expr(v, params);
                        }
                        for item in m.on_create.iter().chain(m.on_match.iter()) {
                            collect_params_from_expr(&item.property, params);
                            collect_params_from_expr(&item.value, params);
                        }
                    }
                }
            }
        }
        BoundStatement::BoundCreateDml(c) => {
            for p in &c.patterns {
                if let Some(n) = &p.node {
                    for (_, v) in &n.properties {
                        collect_params_from_expr(v, params);
                    }
                }
                if let Some(e) = &p.edge {
                    for (_, v) in &e.properties {
                        collect_params_from_expr(v, params);
                    }
                }
            }
        }
        BoundStatement::BoundMerge(m) => {
            for (_, v) in &m.properties {
                collect_params_from_expr(v, params);
            }
            for p in &m.patterns {
                if let Some(n) = &p.node {
                    for (_, v) in &n.properties {
                        collect_params_from_expr(v, params);
                    }
                }
                if let Some(e) = &p.edge {
                    for (_, v) in &e.properties {
                        collect_params_from_expr(v, params);
                    }
                }
            }
            for item in &m.on_create {
                collect_params_from_expr(&item.property, params);
                collect_params_from_expr(&item.value, params);
            }
            for item in &m.on_match {
                collect_params_from_expr(&item.property, params);
                collect_params_from_expr(&item.value, params);
            }
        }
        BoundStatement::BoundExplain(e) => {
            collect_params_from_statement(&e.inner, params);
        }
        _ => {}
    }
}

fn collect_params_from_match_clause(m: &BoundMatchClause, params: &mut Vec<String>) {
    for p in &m.patterns {
        for (_, v) in &p.properties {
            collect_params_from_expr(v, params);
        }
        if let Some(e) = &p.edge {
            for (_, v) in &e.properties {
                collect_params_from_expr(v, params);
            }
        }
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
        Expression::ExistsSubquery(q) => {
            for clause in &q.clauses {
                match clause {
                    Clause::Where(w) => collect_params_from_expr(&w.expression, params),
                    Clause::Return(r) => {
                        for item in &r.expressions {
                            collect_params_from_expr(&item.expression, params);
                        }
                    }
                    _ => {}
                }
            }
        }
        Expression::Case(case_expr) => {
            if let Some(subj) = &case_expr.subject {
                collect_params_from_expr(subj, params);
            }
            for alt in &case_expr.alternatives {
                collect_params_from_expr(&alt.when, params);
                collect_params_from_expr(&alt.then, params);
            }
            if let Some(else_e) = &case_expr.else_expr {
                collect_params_from_expr(else_e, params);
            }
        }
        Expression::Star => {}
        Expression::ListPredicate { list, predicate, .. } => {
            collect_params_from_expr(list, params);
            collect_params_from_expr(predicate, params);
        }
        Expression::Lambda { body, .. } => {
            collect_params_from_expr(body, params);
        }
    }
}

/// Substitute parameter references with concrete values in an expression tree.
pub fn substitute_params(expr: &Expression, param_values: &HashMap<String, Value>) -> Result<Expression, String> {
    match expr {
        Expression::Parameter(name) => {
            let value = param_values
                .get(name)
                .ok_or_else(|| format!("Missing parameter: ${}", name))?;
            Ok(value_to_expression(value)?)
        }
        Expression::PropertyAccess(obj, prop) => {
            let new_obj = substitute_params(obj, param_values)?;
            Ok(Expression::PropertyAccess(Box::new(new_obj), prop.clone()))
        }
        Expression::FunctionCall(name, args) => {
            let new_args: Result<Vec<_>, _> = args.iter().map(|a| substitute_params(a, param_values)).collect();
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
            let new_items: Result<Vec<_>, _> = items.iter().map(|i| substitute_params(i, param_values)).collect();
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
        Expression::ExistsSubquery(q) => Ok(Expression::ExistsSubquery(Box::new(substitute_params_in_query(
            q,
            param_values,
        )?))),
        Expression::Case(case_expr) => {
            use akar_parser::ast::{CaseAlternative, CaseExpr};
            let subject = if let Some(subj) = &case_expr.subject {
                Some(Box::new(substitute_params(subj, param_values)?))
            } else {
                None
            };
            let alternatives: Result<Vec<CaseAlternative>, String> = case_expr
                .alternatives
                .iter()
                .map(|alt| {
                    Ok(CaseAlternative {
                        when: substitute_params(&alt.when, param_values)?,
                        then: substitute_params(&alt.then, param_values)?,
                    })
                })
                .collect();
            let else_expr = if let Some(e) = &case_expr.else_expr {
                Some(Box::new(substitute_params(e, param_values)?))
            } else {
                None
            };
            Ok(Expression::Case(CaseExpr {
                subject,
                alternatives: alternatives?,
                else_expr,
            }))
        }
        Expression::Star => Ok(expr.clone()),
        Expression::ListPredicate {
            quantifier,
            list,
            var_name,
            predicate,
        } => {
            let new_list = substitute_params(list, param_values)?;
            let new_predicate = substitute_params(predicate, param_values)?;
            Ok(Expression::ListPredicate {
                quantifier: *quantifier,
                list: Box::new(new_list),
                var_name: var_name.clone(),
                predicate: Box::new(new_predicate),
            })
        }
        Expression::Lambda { var_name, body } => {
            let new_body = substitute_params(body, param_values)?;
            Ok(Expression::Lambda {
                var_name: var_name.clone(),
                body: Box::new(new_body),
            })
        }
    }
}

/// Substitute parameters in a Query's clauses.
fn substitute_params_in_query(query: &Query, param_values: &HashMap<String, Value>) -> Result<Query, String> {
    let mut new_clauses = Vec::new();
    for clause in &query.clauses {
        let new_clause = match clause {
            Clause::Where(w) => {
                let new_expr = substitute_params(&w.expression, param_values)?;
                Clause::Where(WhereClause { expression: new_expr })
            }
            Clause::Return(r) => {
                let new_items: Result<Vec<ReturnItem>, String> = r
                    .expressions
                    .iter()
                    .map(|item| {
                        let new_expr = substitute_params(&item.expression, param_values)?;
                        Ok(ReturnItem {
                            expression: new_expr,
                            alias: item.alias.clone(),
                        })
                    })
                    .collect();
                Clause::Return(ReturnClause {
                    expressions: new_items?,
                    distinct: r.distinct,
                    order_by: r.order_by.clone(),
                    limit: r.limit,
                    skip: r.skip,
                })
            }
            other => other.clone(),
        };
        new_clauses.push(new_clause);
    }
    Ok(Query { clauses: new_clauses })
}

/// Convert a Value to a Constant for expression substitution.
///
/// Returns an error instead of silently corrupting the parameter: a
/// `UInt64`/`Int128` that overflows `i64` or a type the parser cannot
/// represent as a constant (Blob, Date, List, …) would previously become a
/// wrong `Integer` or `Null` (P51.32).
fn value_to_constant(value: &Value) -> Result<akar_parser::ast::Constant, String> {
    use akar_parser::ast::Constant;
    match value {
        Value::Null => Ok(Constant::Null),
        Value::Bool(b) => Ok(Constant::Bool(*b)),
        Value::Int64(n) => Ok(Constant::Integer(*n)),
        Value::Int32(n) => Ok(Constant::Integer(*n as i64)),
        Value::Int16(n) => Ok(Constant::Integer(*n as i64)),
        Value::Int8(n) => Ok(Constant::Integer(*n as i64)),
        Value::UInt64(n) => i64::try_from(*n)
            .map(Constant::Integer)
            .map_err(|_| format!("UInt64 parameter {n} exceeds i64 range and cannot be used in a query")),
        Value::UInt32(n) => Ok(Constant::Integer(*n as i64)),
        Value::UInt16(n) => Ok(Constant::Integer(*n as i64)),
        Value::UInt8(n) => Ok(Constant::Integer(*n as i64)),
        Value::Int128(n) => i64::try_from(*n)
            .map(Constant::Integer)
            .map_err(|_| format!("Int128 parameter {n} exceeds i64 range and cannot be used in a query")),
        Value::Double(f) => Ok(Constant::Float(*f)),
        Value::Float(f) => Ok(Constant::Float(*f as f64)),
        Value::String(s) => Ok(Constant::String(s.clone())),
        Value::Blob(_) => Err("BLOB parameters are not supported in queries".into()),
        other => Err(format!(
            "Parameter type {:?} cannot be used in a query",
            std::mem::discriminant(other)
        )),
    }
}

/// Convert an Akar [`Value`] to an AST [`Expression`].
///
/// Like [`value_to_constant`] but supports compound types (List) by
/// producing `Expression::List` nodes.  Used by `substitute_params`
/// when a parameter value is a list (e.g. embedding vector, ID array).
fn value_to_expression(value: &Value) -> Result<akar_parser::ast::Expression, String> {
    use akar_parser::ast::Expression;
    match value {
        Value::List(items) => {
            let mut exprs = Vec::with_capacity(items.len());
            for item in items {
                exprs.push(value_to_expression(item)?);
            }
            Ok(Expression::List(exprs))
        }
        // Scalars: delegate to value_to_constant → Expression::Constant
        other => Ok(Expression::Constant(value_to_constant(other)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_common::types::Value;
    use akar_parser::ast::*;
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
            Expression::BinaryOp(_, _, right) => match *right {
                Expression::Constant(Constant::String(s)) => {
                    assert_eq!(s, "Alice");
                }
                _ => panic!("Expected constant string"),
            },
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
        assert_eq!(value_to_constant(&Value::Int64(42)), Ok(Constant::Integer(42)));
        assert_eq!(
            value_to_constant(&Value::String("hi".into())),
            Ok(Constant::String("hi".into()))
        );
        assert_eq!(value_to_constant(&Value::Bool(true)), Ok(Constant::Bool(true)));
        assert_eq!(value_to_constant(&Value::Double(3.15)), Ok(Constant::Float(3.15)));
        assert_eq!(value_to_constant(&Value::Null), Ok(Constant::Null));
    }

    #[test]
    fn test_value_to_constant_uint64_overflow_errors() {
        // Values beyond i64::MAX must error instead of silently wrapping.
        assert_eq!(
            value_to_constant(&Value::UInt64(i64::MAX as u64 + 1)),
            Err("UInt64 parameter 9223372036854775808 exceeds i64 range and cannot be used in a query".into())
        );
        assert_eq!(
            value_to_constant(&Value::UInt64(u64::MAX)),
            Err("UInt64 parameter 18446744073709551615 exceeds i64 range and cannot be used in a query".into())
        );
        // Fits — ok.
        assert_eq!(
            value_to_constant(&Value::UInt64(i64::MAX as u64)),
            Ok(Constant::Integer(i64::MAX))
        );
    }

    #[test]
    fn test_value_to_constant_blob_errors() {
        assert_eq!(
            value_to_constant(&Value::Blob(vec![1, 2, 3])),
            Err("BLOB parameters are not supported in queries".into())
        );
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

    #[test]
    fn test_value_to_expression_list() {
        let val = Value::List(vec![
            Value::Int64(1),
            Value::Int64(2),
            Value::Int64(3),
        ]);
        let expr = value_to_expression(&val).unwrap();
        match expr {
            Expression::List(items) => {
                assert_eq!(items.len(), 3);
                // Each item should be a Constant(Integer)
                for (i, item) in items.iter().enumerate() {
                    match item {
                        Expression::Constant(Constant::Integer(n)) => {
                            assert_eq!(*n, (i + 1) as i64);
                        }
                        other => panic!("expected Integer constant at {i}, got {other:?}"),
                    }
                }
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn test_value_to_expression_nested_list() {
        let val = Value::List(vec![
            Value::List(vec![Value::Int64(1), Value::Int64(2)]),
            Value::List(vec![Value::Int64(3), Value::Int64(4)]),
        ]);
        let expr = value_to_expression(&val).unwrap();
        match expr {
            Expression::List(outer) => {
                assert_eq!(outer.len(), 2);
                match &outer[0] {
                    Expression::List(inner) => {
                        assert_eq!(inner.len(), 2);
                    }
                    other => panic!("expected inner List, got {other:?}"),
                }
            }
            other => panic!("expected List, got {other:?}"),
        }
    }
}
