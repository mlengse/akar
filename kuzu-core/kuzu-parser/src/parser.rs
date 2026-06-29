//! Parser implementation — converts Cypher query text to AST using pest.rs PEG grammar.

use crate::ast::*;
use pest::Parser;
use std::collections::HashMap;

#[derive(pest_derive::Parser)]
#[grammar = "cypher.pest"]
pub struct CypherParser;

pub fn parse(input: &str) -> Result<Statement, String> {
    let trimmed = input.trim();
    let mut pairs = CypherParser::parse(Rule::statement, trimmed).map_err(|e| format!("Parse error: {e}"))?;
    let pair = pairs.next().ok_or("Empty input")?;
    parse_statement(pair)
}

fn parse_statement(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    // Unwrap the outer statement rule to its inner content
    let inner = if pair.as_rule() == Rule::statement {
        pair.into_inner().next().ok_or("Empty statement")?
    } else {
        pair
    };
    match inner.as_rule() {
        Rule::ddl_statement => {
            let ddl_inner = inner.into_inner().next().ok_or("Empty DDL")?;
            parse_ddl(ddl_inner)
        }
        Rule::create_dml_statement => {
            let inner_clone = inner.clone();
            let patterns = parse_patterns(inner_clone)?;
            Ok(Statement::CreateDml(CreateClause { patterns }))
        }
        Rule::query_statement => {
            let inner_clone = inner.clone();
            let child_rules: Vec<_> = inner_clone.into_inner().map(|c| c.as_rule()).collect();
            if child_rules.iter().any(|r| *r == Rule::merge_clause) {
                let merge = parse_merge(inner)?;
                Ok(merge)
            } else {
                let query = parse_query_pairs(inner)?;
                Ok(Statement::Query(query))
            }
        }
        Rule::call_statement => {
            let call = parse_call(inner)?;
            Ok(Statement::Call(call))
        }
        _ => Err(format!("Unexpected rule: {:?}", inner.as_rule())),
    }
}

fn parse_ddl(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    match pair.as_rule() {
        Rule::create_node_table => {
            let mut name = String::new();
            let mut columns = Vec::new();
            let mut pk = String::new();
            for inner in pair.into_inner() {
                match inner.as_rule() {
                    Rule::identifier if name.is_empty() => name = inner.as_str().to_string(),
                    Rule::column_definitions => {
                        for col in inner.into_inner() {
                            let mut cn = String::new();
                            let mut ct = String::new();
                            for part in col.into_inner() {
                                match part.as_rule() {
                                    Rule::identifier => cn = part.as_str().to_string(),
                                    Rule::type_name => ct = part.as_str().to_string(),
                                    _ => {}
                                }
                            }
                            columns.push(ColumnDef {
                                name: cn,
                                type_name: ct,
                            });
                        }
                    }
                    Rule::primary_key => {
                        for part in inner.into_inner() {
                            if part.as_rule() == Rule::identifier {
                                pk = part.as_str().to_string();
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Statement::CreateNodeTable(CreateNodeTable {
                name,
                columns,
                primary_key: pk,
            }))
        }
        Rule::create_rel_table => {
            let mut name = String::new();
            let mut from = String::new();
            let mut to = String::new();
            let mut columns = Vec::new();
            for inner in pair.into_inner() {
                match inner.as_rule() {
                    Rule::identifier => {
                        if name.is_empty() {
                            name = inner.as_str().to_string();
                        } else if from.is_empty() {
                            from = inner.as_str().to_string();
                        } else {
                            to = inner.as_str().to_string();
                        }
                    }
                    Rule::column_definitions => {
                        for col in inner.into_inner() {
                            let mut cn = String::new();
                            let mut ct = String::new();
                            for part in col.into_inner() {
                                match part.as_rule() {
                                    Rule::identifier => cn = part.as_str().to_string(),
                                    Rule::type_name => ct = part.as_str().to_string(),
                                    _ => {}
                                }
                            }
                            columns.push(ColumnDef {
                                name: cn,
                                type_name: ct,
                            });
                        }
                    }
                    _ => {}
                }
            }
            Ok(Statement::CreateRelTable(CreateRelTable {
                name,
                from,
                to,
                columns,
            }))
        }
        Rule::drop_table => {
            let name = pair
                .into_inner()
                .find(|p| p.as_rule() == Rule::identifier)
                .map(|p| p.as_str().to_string())
                .ok_or("Missing table name")?;
            Ok(Statement::DropTable(DropTable { name }))
        }
        Rule::union_statement => {
            let mut inner = pair.into_inner();
            let left = parse_query_pairs(inner.next().ok_or("Missing left query")?)?;
            let union_keyword = inner.next().ok_or("Missing UNION")?;
            let all = union_keyword.as_str().eq_ignore_ascii_case("UNION ALL");
            let right = parse_query_pairs(inner.next().ok_or("Missing right query")?)?;
            Ok(Statement::Union(UnionStatement { left, right, all }))
        }
        Rule::copy_from => parse_copy_from(pair),
        Rule::alter_table => parse_alter_table(pair),
        Rule::create_vector_index => parse_create_vector_index(pair),
        _ => Err(format!("Unknown DDL: {:?}", pair.as_rule())),
    }
}

fn parse_create_vector_index(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut index_name = String::new();
    let mut table_name = String::new();
    let mut column_name = String::new();
    let mut metric = String::new();
    let mut dimensions: Option<u64> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier if index_name.is_empty() => {
                index_name = inner.as_str().to_string();
            }
            Rule::identifier if table_name.is_empty() => {
                // Second identifier is the table name (inside parentheses before dot)
                table_name = inner.as_str().to_string();
            }
            Rule::identifier if column_name.is_empty() => {
                // Third identifier is the column name (after dot)
                column_name = inner.as_str().to_string();
            }
            Rule::vector_index_options => {
                for opt in inner.into_inner() {
                    match opt.as_rule() {
                        Rule::metric_option => {
                            for part in opt.into_inner() {
                                let val = part.as_str().to_lowercase();
                                if val != "metric" && val != "=" {
                                    metric = val;
                                }
                            }
                        }
                        Rule::dimensions_option => {
                            for part in opt.into_inner() {
                                if part.as_rule() == Rule::integer {
                                    dimensions = Some(
                                        part.as_str().parse::<u64>().map_err(|e| {
                                            format!("Invalid dimensions value: {e}")
                                        })?,
                                    );
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    if index_name.is_empty() {
        return Err("Missing index name for CREATE VECTOR INDEX".into());
    }
    if table_name.is_empty() {
        return Err("Missing table name for CREATE VECTOR INDEX".into());
    }
    if column_name.is_empty() {
        return Err("Missing column name for CREATE VECTOR INDEX".into());
    }
    if metric.is_empty() {
        return Err("Missing metric for CREATE VECTOR INDEX (use: cosine, euclidean, l2, or dot)".into());
    }
    let dims = dimensions.ok_or("Missing dimensions for CREATE VECTOR INDEX (use: dims=N)")?;

    Ok(Statement::CreateVectorIndex(CreateVectorIndex {
        index_name,
        table_name,
        column_name,
        metric,
        dimensions: dims,
    }))
}

fn parse_query_pairs(pair: pest::iterators::Pair<Rule>) -> Result<Query, String> {
    let mut clauses = Vec::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::match_clause => {
                clauses.push(Clause::Match(MatchClause {
                    patterns: parse_patterns(inner)?,
                }));
            }
            Rule::optional_match_clause => {
                clauses.push(Clause::OptionalMatch(OptionalMatchClause {
                    patterns: parse_patterns(inner)?,
                }));
            }
            Rule::return_clause => {
                clauses.push(Clause::Return(ReturnClause {
                    expressions: parse_return_items(inner)?,
                }));
            }
            Rule::with_clause => {
                clauses.push(Clause::With(ReturnClause {
                    expressions: parse_return_items(inner)?,
                }));
            }
            Rule::where_clause => {
                let expr = parse_expression(inner.into_inner().next().ok_or("Empty WHERE")?)?;
                clauses.push(Clause::Where(WhereClause { expression: expr }));
            }
            Rule::delete_clause => {
                let expressions: Result<Vec<_>, _> = inner.into_inner().map(parse_expression).collect();
                clauses.push(Clause::Delete(DeleteClause {
                    expressions: expressions?,
                }));
            }
            Rule::unwind_clause => {
                let mut expr = None;
                let mut var = String::new();
                for part in inner.into_inner() {
                    match part.as_rule() {
                        Rule::expression => expr = Some(parse_expression(part)?),
                        Rule::variable => var = part.as_str().to_string(),
                        _ => {}
                    }
                }
                let expression = expr.ok_or("Missing UNWIND expression")?;
                clauses.push(Clause::Unwind(UnwindClause {
                    expression,
                    variable: var,
                }));
            }
            Rule::set_clause => {
                let items: Result<Vec<SetItem>, String> = inner
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::set_item)
                    .map(|item| {
                        let mut parts = item.into_inner();
                        let prop = parse_expression(parts.next().ok_or("Missing SET property".to_string())?)?;
                        let val = parse_expression(parts.next().ok_or("Missing SET value".to_string())?)?;
                        Ok(SetItem {
                            property: prop,
                            value: val,
                        })
                    })
                    .collect();
                clauses.push(Clause::Set(SetClause { items: items? }));
            }
            Rule::merge_clause => {
                // MERGE is handled separately in parse_statement
            }
            Rule::foreach_clause => {
                let clause = parse_foreach_clause(inner)?;
                clauses.push(Clause::Foreach(clause));
            }
            Rule::create_clause_inline => {
                // CREATE inside FOREACH body
                let patterns = parse_patterns(inner)?;
                clauses.push(Clause::Create(CreateClause { patterns }));
            }
            _ => {}
        }
    }
    Ok(Query { clauses })
}

/// Parse a FOREACH clause: `FOREACH (var IN list | body_clauses...)`
fn parse_foreach_clause(pair: pest::iterators::Pair<Rule>) -> Result<ForeachClause, String> {
    let mut variable = String::new();
    let mut expression = None;
    let mut sub_clauses = Vec::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::variable => variable = inner.as_str().to_string(),
            Rule::expression => expression = Some(parse_expression(inner)?),
            Rule::foreach_body => {
                // Parse body clauses (CREATE, SET, DELETE)
                for body_inner in inner.into_inner() {
                    match body_inner.as_rule() {
                        Rule::create_clause_inline => {
                            let patterns = parse_patterns(body_inner)?;
                            sub_clauses.push(Clause::Create(CreateClause { patterns }));
                        }
                        Rule::set_clause => {
                            let items: Result<Vec<SetItem>, String> = body_inner
                                .into_inner()
                                .filter(|p| p.as_rule() == Rule::set_item)
                                .map(|item| {
                                    let mut parts = item.into_inner();
                                    let prop = parse_expression(
                                        parts.next().ok_or("Missing SET property".to_string())?,
                                    )?;
                                    let val = parse_expression(
                                        parts.next().ok_or("Missing SET value".to_string())?,
                                    )?;
                                    Ok(SetItem { property: prop, value: val })
                                })
                                .collect();
                            sub_clauses.push(Clause::Set(SetClause { items: items? }));
                        }
                        Rule::delete_clause => {
                            let expressions: Result<Vec<_>, _> =
                                body_inner.into_inner().map(parse_expression).collect();
                            sub_clauses.push(Clause::Delete(DeleteClause {
                                expressions: expressions?,
                            }));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(ForeachClause {
        variable,
        expression: expression.ok_or("Missing FOREACH expression")?,
        clauses: sub_clauses,
    })
}

fn parse_patterns(pair: pest::iterators::Pair<Rule>) -> Result<Vec<Pattern>, String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::pattern)
        .map(parse_pattern)
        .collect()
}

fn parse_pattern(pair: pest::iterators::Pair<Rule>) -> Result<Pattern, String> {
    let mut node = None;
    let mut edge = None;
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::node_pattern => {
                node = Some(parse_node_pattern(inner)?);
            }
            Rule::edge_pattern => {
                edge = Some(parse_edge_pattern(inner)?);
            }
            _ => {}
        }
    }
    Ok(Pattern { node, edge })
}

fn parse_node_pattern(pair: pest::iterators::Pair<Rule>) -> Result<NodePattern, String> {
    let mut variable = None;
    let mut labels = Vec::new();
    let mut properties = Vec::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::variable => variable = Some(inner.as_str().to_string()),
            Rule::label => {
                for li in inner.into_inner() {
                    labels.push(li.as_str().to_string());
                }
            }
            Rule::property_map => {
                for prop in inner.into_inner() {
                    if prop.as_rule() == Rule::property_key_value {
                        let (k, v) = parse_property_kv(prop)?;
                        properties.push((k, v));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(NodePattern {
        variable,
        labels,
        properties,
    })
}

fn parse_edge_pattern(pair: pest::iterators::Pair<Rule>) -> Result<EdgePattern, String> {
    let mut variable = None;
    let mut labels = Vec::new();
    let mut properties = Vec::new();
    let mut lower_bound = None;
    let mut upper_bound = None;
    let direction = if pair.as_str().contains("<-") {
        EdgeDirection::RightToLeft
    } else {
        EdgeDirection::LeftToRight
    };
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::variable => variable = Some(inner.as_str().to_string()),
            Rule::label => {
                for li in inner.into_inner() {
                    labels.push(li.as_str().to_string());
                }
            }
            Rule::property_map => {
                for prop in inner.into_inner() {
                    if prop.as_rule() == Rule::property_key_value {
                        let (k, v) = parse_property_kv(prop)?;
                        properties.push((k, v));
                    }
                }
            }
            Rule::var_length => {
                let parts: Vec<i64> = inner
                    .into_inner()
                    .filter_map(|p| p.as_str().parse::<i64>().ok())
                    .collect();
                if parts.len() == 2 {
                    lower_bound = Some(parts[0] as u64);
                    upper_bound = Some(parts[1] as u64);
                } else {
                    // Just `*` with no bounds
                    lower_bound = Some(1);
                    upper_bound = None;
                }
            }
            _ => {}
        }
    }
    Ok(EdgePattern {
        variable,
        labels,
        direction,
        properties,
        lower_bound,
        upper_bound,
    })
}

fn parse_property_kv(pair: pest::iterators::Pair<Rule>) -> Result<(String, Expression), String> {
    let mut key = String::new();
    let mut val = None;
    for part in pair.into_inner() {
        match part.as_rule() {
            Rule::identifier => key = part.as_str().to_string(),
            Rule::expression => val = Some(parse_expression(part)?),
            _ => {}
        }
    }
    val.map(|v| (key, v)).ok_or("Missing property value".into())
}

fn parse_return_items(pair: pest::iterators::Pair<Rule>) -> Result<Vec<ReturnItem>, String> {
    let mut items = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::return_item {
            let mut expr = None;
            let mut alias = None;
            for part in inner.into_inner() {
                match part.as_rule() {
                    Rule::expression => expr = Some(parse_expression(part)?),
                    Rule::identifier => alias = Some(part.as_str().to_string()),
                    _ => {}
                }
            }
            if let Some(e) = expr {
                items.push(ReturnItem { expression: e, alias });
            }
        }
    }
    Ok(items)
}

fn parse_expression(pair: pest::iterators::Pair<Rule>) -> Result<Expression, String> {
    // Handle compound binary expressions by collecting children
    let rule = pair.as_rule();
    let children: Vec<_> = pair.clone().into_inner().collect();

    // Unwrap single-child wrappers (priority/precedence levels)
    if matches!(
        rule,
        Rule::expression
            | Rule::or_expr
            | Rule::and_expr
            | Rule::not_expr
            | Rule::comparison_expr
            | Rule::additive_expr
            | Rule::multiplicative_expr
            | Rule::unary_expr
            | Rule::postfix_expr
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
                    "AND" => BinaryOp::And,
                    "=" => BinaryOp::Equal,
                    "<>" => BinaryOp::NotEqual,
                    "<" => BinaryOp::LessThan,
                    ">" => BinaryOp::GreaterThan,
                    "<=" => BinaryOp::LessThanOrEqual,
                    ">=" => BinaryOp::GreaterThanOrEqual,
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
        Rule::primary => parse_expression(children.into_iter().next().ok_or("Empty primary")?),
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
        Rule::property_access => {
            let name = children
                .into_iter()
                .next()
                .map(|p| p.as_str().to_string())
                .ok_or("Empty property")?;
            Ok(Expression::Variable(name))
        }
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

fn parse_literal(pair: pest::iterators::Pair<Rule>) -> Result<Expression, String> {
    match pair.as_rule() {
        Rule::string => Ok(Expression::Constant(Constant::String(unescape_string(pair.as_str())))),
        Rule::integer => Ok(Expression::Constant(Constant::Integer(
            pair.as_str().parse().map_err(|e| format!("Int: {e}"))?,
        ))),
        Rule::float => Ok(Expression::Constant(Constant::Float(
            pair.as_str().parse().map_err(|e| format!("Float: {e}"))?,
        ))),
        Rule::boolean_literal => Ok(Expression::Constant(Constant::Bool(
            pair.as_str().to_uppercase() == "TRUE",
        ))),
        Rule::null_literal => Ok(Expression::Constant(Constant::Null)),
        _ => Err(format!("Unknown literal: {:?}", pair.as_rule())),
    }
}

fn unescape_string(s: &str) -> String {
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

fn parse_alter_table(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut table_name = String::new();
    let mut action = None;
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier => {
                if table_name.is_empty() {
                    table_name = inner.as_str().to_string();
                }
            }
            Rule::alter_action => {
                let action_inner = inner.into_inner().next().ok_or("Empty alter action")?;
                action = Some(match action_inner.as_rule() {
                    Rule::add_column => {
                        let mut name = String::new();
                        let mut type_name = String::new();
                        for part in action_inner.into_inner() {
                            match part.as_rule() {
                                Rule::identifier => name = part.as_str().to_string(),
                                Rule::type_name => type_name = part.as_str().to_string(),
                                _ => {}
                            }
                        }
                        AlterAction::AddColumn { name, type_name }
                    }
                    Rule::drop_column => {
                        let name = action_inner
                            .into_inner()
                            .find(|p| p.as_rule() == Rule::identifier)
                            .map(|p| p.as_str().to_string())
                            .ok_or("Missing column name in DROP")?;
                        AlterAction::DropColumn { name }
                    }
                    Rule::rename_column => {
                        let mut parts = action_inner.into_inner().filter(|p| p.as_rule() == Rule::identifier);
                        let old_name = parts.next().ok_or("Missing old column name")?.as_str().to_string();
                        let new_name = parts.next().ok_or("Missing new column name")?.as_str().to_string();
                        AlterAction::RenameColumn { old_name, new_name }
                    }
                    Rule::rename_table => {
                        let new_name = action_inner
                            .into_inner()
                            .find(|p| p.as_rule() == Rule::identifier)
                            .map(|p| p.as_str().to_string())
                            .ok_or("Missing new table name")?;
                        AlterAction::RenameTable { new_name }
                    }
                    _ => return Err(format!("Unknown alter action: {:?}", action_inner.as_rule())),
                });
            }
            _ => {}
        }
    }
    let action = action.ok_or("Missing alter action")?;
    Ok(Statement::AlterTable(AlterTable { table_name, action }))
}

fn parse_copy_from(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut table_name = String::new();
    let mut file_path = String::new();
    let mut options = HashMap::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier => {
                if table_name.is_empty() {
                    table_name = inner.as_str().to_string();
                }
            }
            Rule::string => {
                if file_path.is_empty() {
                    file_path = unescape_string(inner.as_str());
                }
            }
            Rule::copy_options => {
                // copy_options contains: "(", copy_option, (",", copy_option)*, ")"
                for item in inner.into_inner() {
                    // item is either "(", ")", "," (punctuation) or copy_option
                    if item.as_rule() != Rule::copy_option {
                        continue;
                    }
                    // copy_option wraps one of: header_option | delim_option | escape_option | quote_option
                    if let Some(opt) = item.into_inner().next() {
                        let opt_name = match opt.as_rule() {
                            Rule::header_option => "header",
                            Rule::delim_option => "delim",
                            Rule::escape_option => "escape",
                            Rule::quote_option => "quote",
                            _ => continue,
                        };
                        // Extract the value from the option (skip keyword tokens)
                        for val in opt.into_inner() {
                            let text = val.as_str();
                            let r = val.as_rule();
                            let value = match r {
                                Rule::string => unescape_string(text),
                                Rule::boolean_literal | Rule::integer => text.to_string(),
                                _ => continue, // skip keywords like "HEADER", "DELIM", etc.
                            };
                            options.insert(opt_name.to_string(), value);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(Statement::CopyFrom(CopyFrom {
        table_name,
        file_path,
        options,
    }))
}

/// Parse a CALL statement.
fn parse_call(pair: pest::iterators::Pair<Rule>) -> Result<CallStatement, String> {
    let mut function_name = String::new();
    let mut args = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::call_clause => {
                for part in inner.into_inner() {
                    match part.as_rule() {
                        Rule::function_name => {
                            function_name = part.as_str().to_string();
                        }
                        Rule::call_args => {
                            for expr in part.into_inner() {
                                if expr.as_rule() == Rule::expression {
                                    args.push(parse_expression(expr)?);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Rule::return_clause => {
                // CALL with RETURN — handled at execution level
            }
            _ => {}
        }
    }

    if function_name.is_empty() {
        return Err("CALL requires a function name".into());
    }

    Ok(CallStatement { function_name, args })
}

/// Parse a MERGE statement.
fn parse_merge(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut pattern = None;
    let mut on_create = Vec::new();
    let mut on_match = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::merge_clause => {
                for part in inner.into_inner() {
                    match part.as_rule() {
                        Rule::pattern => {
                            pattern = Some(parse_pattern(part)?);
                        }
                        Rule::on_create_set => {
                            for item in part.into_inner() {
                                if item.as_rule() == Rule::set_item {
                                    let mut p = item.into_inner();
                                    let prop = parse_expression(p.next().ok_or("Missing ON CREATE SET property")?)?;
                                    let val = parse_expression(p.next().ok_or("Missing ON CREATE SET value")?)?;
                                    on_create.push(SetItem { property: prop, value: val });
                                }
                            }
                        }
                        Rule::on_match_set => {
                            for item in part.into_inner() {
                                if item.as_rule() == Rule::set_item {
                                    let mut p = item.into_inner();
                                    let prop = parse_expression(p.next().ok_or("Missing ON MATCH SET property")?)?;
                                    let val = parse_expression(p.next().ok_or("Missing ON MATCH SET value")?)?;
                                    on_match.push(SetItem { property: prop, value: val });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Rule::return_clause => {
                // MERGE with RETURN — ignored for now; handled at binder level
            }
            _ => {}
        }
    }

    let pattern = pattern.ok_or("MERGE requires a pattern")?;
    Ok(Statement::Merge(MergeStatement {
        pattern,
        on_create,
        on_match,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_node_table() {
        let sql = "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateNodeTable(t) => {
                assert_eq!(t.name, "Person");
                assert_eq!(t.columns.len(), 2);
                assert_eq!(t.primary_key, "name");
                assert_eq!(t.columns[0].name, "name");
                assert_eq!(t.columns[0].type_name, "STRING");
            }
            _ => panic!("Expected CreateNodeTable"),
        }
    }

    #[test]
    fn test_drop_table() {
        let sql = "DROP TABLE Person";
        assert!(matches!(parse(sql).unwrap(), Statement::DropTable(t) if t.name == "Person"));
    }

    #[test]
    fn test_match_return() {
        let sql = "MATCH (a:Person) RETURN a.name, a.age";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Query(q) => {
                assert_eq!(q.clauses.len(), 2);
                assert!(matches!(q.clauses[0], Clause::Match(_)));
                assert!(matches!(q.clauses[1], Clause::Return(_)));
            }
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn test_match_where_return() {
        let sql = "MATCH (a:Person) WHERE a.age > 25 RETURN a.name";
        let stmt = parse(sql).unwrap();
        assert!(matches!(stmt, Statement::Query(q) if q.clauses.len() == 3));
    }

    #[test]
    fn test_match_with_string() {
        let sql = "MATCH (a:Person) WHERE a.name = 'Alice' RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_rel_pattern() {
        let sql = "MATCH (a:Person)-[r:Knows]->(b:Person) RETURN a, b";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_function_call() {
        let sql = "MATCH (a:Person) RETURN COUNT(a)";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_count_star() {
        let sql = "MATCH (a:Person) RETURN COUNT(*)";
        let result = parse(sql);
        assert!(result.is_ok(), "COUNT(*) should parse: {:?}", result.err());
    }

    #[test]
    fn test_integer_expr() {
        let sql = "MATCH (a) WHERE a.age = 30 RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_boolean_expr() {
        let sql = "MATCH (a) WHERE a.active = TRUE RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_list_literal() {
        let sql = "MATCH (a) WHERE a.age IN [1, 2, 3] RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_foreach_basic() {
        let sql = "FOREACH (x IN [1,2,3] | CREATE (n:Num {val: x}))";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Query(q) => {
                assert_eq!(q.clauses.len(), 1);
                match &q.clauses[0] {
                    Clause::Foreach(f) => {
                        assert_eq!(f.variable, "x");
                        assert_eq!(f.clauses.len(), 1);
                    }
                    _ => panic!("Expected Foreach clause"),
                }
            }
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn test_foreach_in_match() {
        let sql = "MATCH (a:Person) FOREACH (x IN [1,2] | SET a.val = x)";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_var_length_path_simple() {
        let sql = "MATCH (a:Person)-[*]->(b:Person) RETURN a, b";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_var_length_path_with_bounds() {
        let sql = "MATCH (a:Person)-[*1..5]->(b:Person) RETURN a, b";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_var_length_path_with_rel_variable() {
        let sql = "MATCH (a:Person)-[r:*1..3]->(b:Person) RETURN a, b";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_exists_subquery_basic() {
        let sql = "MATCH (a:Person) WHERE EXISTS { MATCH (b:City) WHERE b.name = a.name } RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_exists_subquery_in_return() {
        let sql = "MATCH (a:Person) RETURN EXISTS { MATCH (b:City) WHERE b.pop > 1000 }";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_complex_and_or() {
        let sql = "MATCH (a:Person) WHERE a.age > 25 AND a.name = 'Bob' OR a.age < 10 RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_create_rel_table() {
        let sql = "CREATE REL TABLE Knows(FROM Person TO Person, since INT64)";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateRelTable(t) => {
                assert_eq!(t.name, "Knows");
                assert_eq!(t.columns.len(), 1);
            }
            _ => panic!("Expected CreateRelTable"),
        }
    }

    #[test]
    fn test_parameter_syntax() {
        let sql = "MATCH (a:Person) WHERE a.age > $min RETURN a";
        let stmt = parse(sql).unwrap();
        assert!(matches!(stmt, Statement::Query(_)));
    }

    #[test]
    fn test_multiple_parameters() {
        let sql = "MATCH (a:Person) WHERE a.age > $min AND a.age < $max RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_parameter_with_string() {
        let sql = "MATCH (a:Person) WHERE a.name = $name RETURN a";
        assert!(parse(sql).is_ok());
    }

    // ==================== COPY FROM tests ====================

    #[test]
    fn test_copy_from_basic() {
        let sql = "COPY Person FROM 'data.csv'";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CopyFrom(c) => {
                assert_eq!(c.table_name, "Person");
                assert!(c.file_path.contains("data.csv"));
                assert!(c.options.is_empty());
            }
            _ => panic!("Expected CopyFrom, got {:?}", stmt),
        }
    }

    #[test]
    fn test_copy_from_with_options() {
        let sql = "COPY Person FROM 'data.csv' (HEADER TRUE, DELIM ',')";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CopyFrom(c) => {
                assert_eq!(c.table_name, "Person");
                assert_eq!(c.options.get("header").map(|s| s.as_str()), Some("TRUE"));
                assert_eq!(c.options.get("delim").map(|s| s.as_str()), Some(","));
            }
            _ => panic!("Expected CopyFrom"),
        }
    }

    #[test]
    fn test_copy_from_full_options() {
        let sql = "COPY Knows FROM 'rels.csv' (HEADER TRUE, DELIM '|', QUOTE '\"', ESCAPE '\\\\')";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CopyFrom(c) => {
                assert_eq!(c.table_name, "Knows");
                assert_eq!(c.options.get("header").map(|s| s.as_str()), Some("TRUE"));
                assert_eq!(c.options.get("delim").map(|s| s.as_str()), Some("|"));
            }
            _ => panic!("Expected CopyFrom"),
        }
    }

    #[test]
    fn test_copy_from_parse_error() {
        // Missing file path
        let sql = "COPY Person FROM";
        assert!(parse(sql).is_err());
    }

    #[test]
    fn test_copy_from_single_quoted_path() {
        let sql = "COPY Person FROM 'path/to/data.csv'";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CopyFrom(c) => {
                assert_eq!(c.table_name, "Person");
                assert!(c.file_path.contains("path/to/data.csv"));
            }
            _ => panic!("Expected CopyFrom"),
        }
    }
}
