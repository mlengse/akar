//! Expression parsing — arithmetic, boolean, string, list, map, case, function calls.

use super::Rule;
use super::dml::{parse_property_kv, parse_query_pairs};
use crate::ast::*;

/// Parse any expression rule to an AST node.
pub fn parse_expression(pair: pest::iterators::Pair<Rule>) -> Result<Expression, String> {
    // Handle compound binary expressions by collecting children
    let rule = pair.as_rule();
    let children: Vec<_> = pair.clone().into_inner().collect();

    // Handle CASE expression
    if rule == Rule::case_expr {
        return parse_case_expr(pair);
    }

    // Handle comparison_expr specially — it can have 1, 2, or 3 children depending
    // on which operator is used (IS NULL, IN, STARTS WITH, =, etc.)
    if rule == Rule::comparison_expr {
        if children.len() == 1 {
            return parse_expression(children[0].clone());
        }
        let left = parse_expression(children[0].clone())?;
        if children.len() == 2 {
            // Postfix/special operators (IS NULL, IN, STARTS WITH, ...)
            return parse_comparison_suffix(children[1].clone(), left);
        }
        if children.len() == 3 {
            // Standard comparison: left comparison_op right
            let op_str = children[1].as_str();
            let op = match op_str {
                "=" => BinaryOp::Equal,
                "<>" => BinaryOp::NotEqual,
                "<" => BinaryOp::LessThan,
                ">" => BinaryOp::GreaterThan,
                "<=" => BinaryOp::LessThanOrEqual,
                ">=" => BinaryOp::GreaterThanOrEqual,
                _ => return Err(format!("Unknown comparison_op: {}", op_str)),
            };
            let right = parse_expression(children[2].clone())?;
            return Ok(Expression::BinaryOp(op, Box::new(left), Box::new(right)));
        }
        return Err(format!("Unexpected comparison_expr with {} children", children.len()));
    }

    // Unwrap single-child wrappers (priority/precedence levels)
    if matches!(
        rule,
        Rule::expression
            | Rule::or_expr
            | Rule::xor_expr
            | Rule::and_expr
            | Rule::not_expr
            | Rule::additive_expr
            | Rule::multiplicative_expr
            | Rule::unary_expr
    ) {
        if children.len() == 1 {
            return parse_expression(children[0].clone());
        }
        if children.len() >= 3 {
            let mut result = parse_expression(children[0].clone())?;
            let mut i = 1;
            while i + 1 < children.len() {
                let op = match children[i].as_str() {
                    "OR" => BinaryOp::Or,
                    "XOR" => BinaryOp::Xor,
                    "AND" => BinaryOp::And,
                    "+" => BinaryOp::Add,
                    "-" => BinaryOp::Subtract,
                    "*" => BinaryOp::Multiply,
                    "/" => BinaryOp::Divide,
                    "%" => BinaryOp::Modulo,
                    _ => return Err(format!("Unknown op: {}", children[i].as_str())),
                };
                let right = parse_expression(children[i + 1].clone())?;
                result = Expression::BinaryOp(op, Box::new(result), Box::new(right));
                i += 2;
            }
            return Ok(result);
        }
    }

    // Handle unary NOT
    if rule == Rule::not_expr && children.len() == 2 {
        let inner = parse_expression(children[1].clone())?;
        return Ok(Expression::UnaryOp(UnaryOp::Not, Box::new(inner)));
    }

    match rule {
        Rule::primary => {
            // Handle function calls encoded by grammar as: variable ~ function_args?
            // e.g. nextval('seq') or COUNT(a)
            if children.len() == 2
                && children[0].as_rule() == Rule::variable
                && children[1].as_rule() == Rule::function_args
            {
                let name = children[0].as_str().to_string();
                let mut args = Vec::new();
                let args_text = children[1].as_str().replace(" ", "");
                for c in children[1].clone().into_inner() {
                    if c.as_rule() == Rule::expression {
                        args.push(parse_expression(c)?);
                    }
                }
                if args.is_empty() && args_text == "(*)" {
                    args.push(Expression::Star);
                }
                return Ok(Expression::FunctionCall(name, args));
            }
            parse_expression(children.into_iter().next().ok_or("Empty primary")?)
        }
        Rule::literal => parse_literal(children.into_iter().next().ok_or("Empty literal")?),
        Rule::string => Ok(Expression::Constant(Constant::String(unescape_string(pair.as_str())))),
        Rule::integer => {
            let v: i64 = pair.as_str().parse().map_err(|e| format!("Int: {e}"))?;
            Ok(Expression::Constant(Constant::Integer(v)))
        }
        Rule::float => {
            let v: f64 = pair.as_str().parse().map_err(|e| format!("Float: {e}"))?;
            Ok(Expression::Constant(Constant::Float(v)))
        }
        Rule::boolean_literal => Ok(Expression::Constant(Constant::Bool(
            pair.as_str().to_uppercase() == "TRUE",
        ))),
        Rule::null_literal => Ok(Expression::Constant(Constant::Null)),
        Rule::variable => Ok(Expression::Variable(pair.as_str().to_string())),
        Rule::parameter => {
            let name = pair.as_str().strip_prefix('$').unwrap_or(pair.as_str()).to_string();
            Ok(Expression::Parameter(name))
        }
        Rule::list_literal => {
            let items = children
                .into_iter()
                .filter(|c| c.as_rule() == Rule::expression)
                .map(parse_expression)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expression::List(items))
        }
        Rule::map_literal => {
            let mut entries = Vec::new();
            for c in children {
                if c.as_rule() == Rule::property_key_value {
                    entries.push(parse_property_kv(c)?);
                }
            }
            Ok(Expression::Map(entries))
        }
        Rule::postfix_expr => {
            let mut children = pair.clone().into_inner();
            let mut result = parse_expression(children.next().ok_or("Empty postfix")?)?;
            for child in children {
                if child.as_rule() == Rule::property_access {
                    let prop = child.into_inner().next().unwrap().as_str().to_string();
                    result = Expression::PropertyAccess(Box::new(result), prop);
                }
            }
            Ok(result)
        }
        Rule::exists_subquery => {
            // EXISTS { MATCH ... }
            for c in children {
                if c.as_rule() == Rule::query_statement {
                    let query = parse_query_pairs(c)?;
                    return Ok(Expression::ExistsSubquery(Box::new(query)));
                }
            }
            Err("EXISTS subquery requires a query statement".into())
        }
        Rule::list_predicate => {
            // ANY(x IN list WHERE predicate), ALL/NONE/SINGLE
            // Parse tree has: variable, expression(list), expression(predicate)
            // Quantifier is extracted from the matched string prefix.
            let children: Vec<_> = pair.clone().into_inner().collect();
            if children.len() < 3 {
                return Err(format!("Invalid list predicate syntax: {} children", children.len()));
            }
            // Extract quantifier from the raw token: the first word of the pair
            let full_text = pair.as_str();
            let quantifier_str = full_text.split('(').next().unwrap_or("").to_uppercase();
            let quantifier = match quantifier_str.as_str() {
                "ANY" => Quantifier::Any,
                "ALL" => Quantifier::All,
                "NONE" => Quantifier::None,
                "SINGLE" => Quantifier::Single,
                _ => return Err(format!("Unknown quantifier: {}", quantifier_str)),
            };
            let var_name = children[0].as_str().to_string();
            let list = parse_expression(children[1].clone())?;
            let predicate = parse_expression(children[2].clone())?;
            Ok(Expression::ListPredicate {
                quantifier,
                list: Box::new(list),
                var_name,
                predicate: Box::new(predicate),
            })
        }
        Rule::lambda_expr => {
            let mut children = pair.clone().into_inner();
            let var_name = children.next().ok_or("Lambda missing variable")?.as_str().to_string();
            let body = parse_expression(children.next().ok_or("Lambda missing body")?)?;
            Ok(Expression::Lambda {
                var_name,
                body: Box::new(body),
            })
        }
        Rule::function_args => {
            // function_call can appear as child of postfix_expr
            // The parent variable is the function name
            // We detect this in the postfix chain handling
            if children.is_empty() {
                return Ok(Expression::Constant(Constant::Null));
            }
            // Extract function name from siblings in parent
            let name = pair.as_str().split('(').next().unwrap_or("").to_string();
            let args = children
                .into_iter()
                .filter(|c| c.as_rule() == Rule::expression)
                .map(parse_expression)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expression::FunctionCall(name, args))
        }
        Rule::property_access => Err("property_access should be handled within postfix_expr".into()),
        _ => {
            // Try unwrapping single child
            if let Some(child) = children.into_iter().next() {
                parse_expression(child)
            } else {
                Err(format!("Cannot parse: {:?}", rule))
            }
        }
    }
}

/// Parse a comparison suffix operator node into an expression given the left-hand side.
pub fn parse_comparison_suffix(pair: pest::iterators::Pair<Rule>, left: Expression) -> Result<Expression, String> {
    /// Find the additive_expr child inside an operator rule.
    fn get_rhs(pair: pest::iterators::Pair<Rule>) -> Result<Expression, String> {
        let rhs_pair = pair
            .into_inner()
            .find(|p| p.as_rule() == Rule::additive_expr)
            .ok_or_else(|| "Operator missing right-hand expression".to_string())?;
        parse_expression(rhs_pair)
    }

    match pair.as_rule() {
        Rule::is_check_op => {
            let text = pair.as_str();
            if text.contains("NOT") {
                Ok(Expression::UnaryOp(UnaryOp::IsNotNull, Box::new(left)))
            } else {
                Ok(Expression::UnaryOp(UnaryOp::IsNull, Box::new(left)))
            }
        }
        Rule::in_op => {
            let right = get_rhs(pair)?;
            Ok(Expression::BinaryOp(BinaryOp::In, Box::new(left), Box::new(right)))
        }
        Rule::not_in_op => {
            let right = get_rhs(pair)?;
            Ok(Expression::BinaryOp(BinaryOp::NotIn, Box::new(left), Box::new(right)))
        }
        Rule::starts_with_op => {
            let right = get_rhs(pair)?;
            Ok(Expression::BinaryOp(
                BinaryOp::StartsWith,
                Box::new(left),
                Box::new(right),
            ))
        }
        Rule::not_starts_with_op => {
            let right = get_rhs(pair)?;
            let inner = Expression::BinaryOp(BinaryOp::StartsWith, Box::new(left), Box::new(right));
            Ok(Expression::UnaryOp(UnaryOp::Not, Box::new(inner)))
        }
        Rule::ends_with_op => {
            let right = get_rhs(pair)?;
            Ok(Expression::BinaryOp(
                BinaryOp::EndsWith,
                Box::new(left),
                Box::new(right),
            ))
        }
        Rule::not_ends_with_op => {
            let right = get_rhs(pair)?;
            let inner = Expression::BinaryOp(BinaryOp::EndsWith, Box::new(left), Box::new(right));
            Ok(Expression::UnaryOp(UnaryOp::Not, Box::new(inner)))
        }
        Rule::contains_op => {
            let right = get_rhs(pair)?;
            Ok(Expression::BinaryOp(
                BinaryOp::Contains,
                Box::new(left),
                Box::new(right),
            ))
        }
        Rule::not_contains_op => {
            let right = get_rhs(pair)?;
            let inner = Expression::BinaryOp(BinaryOp::Contains, Box::new(left), Box::new(right));
            Ok(Expression::UnaryOp(UnaryOp::Not, Box::new(inner)))
        }
        Rule::like_op => {
            let right = get_rhs(pair)?;
            Ok(Expression::BinaryOp(BinaryOp::Like, Box::new(left), Box::new(right)))
        }
        Rule::between_op => {
            let mut children = pair.into_inner();
            let lower_pair = children
                .find(|p| p.as_rule() == Rule::additive_expr)
                .ok_or_else(|| "BETWEEN missing lower bound expression".to_string())?;
            let upper_pair = children
                .find(|p| p.as_rule() == Rule::additive_expr)
                .ok_or_else(|| "BETWEEN missing upper bound expression".to_string())?;
            let lower_expr = parse_expression(lower_pair)?;
            let upper_expr = parse_expression(upper_pair)?;
            let ge = Expression::BinaryOp(
                BinaryOp::GreaterThanOrEqual,
                Box::new(left.clone()),
                Box::new(lower_expr),
            );
            let le = Expression::BinaryOp(BinaryOp::LessThanOrEqual, Box::new(left), Box::new(upper_expr));
            Ok(Expression::BinaryOp(BinaryOp::And, Box::new(ge), Box::new(le)))
        }
        r => Err(format!("Unknown comparison suffix: {:?}", r)),
    }
}

/// Parse a `CASE [subject] WHEN ... THEN ... [ELSE ...] END` expression.
pub fn parse_case_expr(pair: pest::iterators::Pair<Rule>) -> Result<Expression, String> {
    let mut subject: Option<Expression> = None;
    let mut alternatives: Vec<CaseAlternative> = Vec::new();
    let mut else_expr: Option<Expression> = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::case_subject => {
                // case_subject wraps a single expression (with lookahead to skip WHEN)
                if let Some(expr_pair) = child.into_inner().next() {
                    subject = Some(parse_expression(expr_pair)?);
                }
            }
            Rule::case_when => {
                // case_when contains exactly two expression children: WHEN expr, THEN expr
                let mut exprs = child
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::expression)
                    .map(parse_expression)
                    .collect::<Result<Vec<_>, _>>()?;
                if exprs.len() < 2 {
                    return Err("CASE WHEN clause requires both WHEN and THEN expressions".into());
                }
                let then = exprs.remove(1);
                let when = exprs.remove(0);
                alternatives.push(CaseAlternative { when, then });
            }
            Rule::case_else => {
                if let Some(expr_pair) = child.into_inner().find(|p| p.as_rule() == Rule::expression) {
                    else_expr = Some(parse_expression(expr_pair)?);
                }
            }
            _ => {} // Skip "CASE", "END" keyword tokens
        }
    }

    if alternatives.is_empty() {
        return Err("CASE expression requires at least one WHEN clause".into());
    }

    Ok(Expression::Case(CaseExpr {
        subject: subject.map(Box::new),
        alternatives,
        else_expr: else_expr.map(Box::new),
    }))
}

/// Parse a literal value.
pub fn parse_literal(pair: pest::iterators::Pair<Rule>) -> Result<Expression, String> {
    match pair.as_rule() {
        Rule::string => Ok(Expression::Constant(Constant::String(unescape_string(pair.as_str())))),
        Rule::integer => {
            let s = pair.as_str();
            Ok(Expression::Constant(Constant::Integer(
                s.trim().parse().map_err(|e| format!("Int: {e} (for string '{s}')"))?,
            )))
        }
        Rule::float => {
            let s = pair.as_str();
            Ok(Expression::Constant(Constant::Float(
                s.trim().parse().map_err(|e| format!("Float: {e} (for string '{s}')"))?,
            )))
        }
        Rule::boolean_literal => Ok(Expression::Constant(Constant::Bool(
            pair.as_str().to_uppercase() == "TRUE",
        ))),
        Rule::null_literal => Ok(Expression::Constant(Constant::Null)),
        _ => Err(format!("Unknown literal: {:?}", pair.as_rule())),
    }
}

/// Unescape a string literal — handles \n, \t, \r, \\, \", \'.
pub fn unescape_string(s: &str) -> String {
    let s = s.trim_matches(|c| c == '"' || c == '\'');
    let mut r = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => r.push('\n'),
                Some('t') => r.push('\t'),
                Some('r') => r.push('\r'),
                Some('\\') => r.push('\\'),
                Some('"') => r.push('"'),
                Some('\'') => r.push('\''),
                Some(o) => {
                    r.push('\\');
                    r.push(o);
                }
                None => r.push('\\'),
            }
        } else {
            r.push(c);
        }
    }
    r
}
