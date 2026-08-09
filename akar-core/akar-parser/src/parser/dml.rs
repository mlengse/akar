//! DML parsing — MATCH, RETURN, WHERE, CREATE, DELETE, SET, MERGE, FOREACH, UNWIND, CALL, patterns.

use super::Rule;
use crate::ast::*;
use crate::parser::ddl::parse_using_fts_clause;
use crate::parser::expression::parse_expression;

pub(crate) fn parse_query_pairs(pair: pest::iterators::Pair<Rule>) -> Result<Query, String> {
    let mut clauses = Vec::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::match_clause => {
                // Check for a trailing using_fts_clause child inside the match_clause subtree
                let inner_clone = inner.clone();
                let fts_query = inner_clone
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::using_fts_clause)
                    .map(|fts| parse_using_fts_clause(fts))
                    .transpose()?;
                clauses.push(Clause::Match(MatchClause {
                    patterns: parse_patterns(inner)?,
                    fts_query,
                }));
            }
            Rule::optional_match_clause => {
                clauses.push(Clause::OptionalMatch(OptionalMatchClause {
                    patterns: parse_patterns(inner)?,
                }));
            }
            Rule::return_clause => {
                let distinct = has_distinct_flag(&inner);
                let order_by = parse_order_by(&inner);
                let (limit, skip) = parse_limit_skip(&inner);
                clauses.push(Clause::Return(ReturnClause {
                    expressions: parse_return_items(inner)?,
                    distinct,
                    order_by,
                    limit,
                    skip,
                }));
            }
            Rule::with_clause => {
                let order_by = parse_order_by(&inner);
                let (limit, skip) = parse_limit_skip(&inner);
                clauses.push(Clause::With(ReturnClause {
                    expressions: parse_return_items(inner)?,
                    distinct: false,
                    order_by,
                    limit,
                    skip,
                }));
            }
            Rule::where_clause => {
                let expr = parse_expression(inner.into_inner().next().ok_or("Empty WHERE")?)?;
                clauses.push(Clause::Where(WhereClause { expression: expr }));
            }
            Rule::delete_clause => {
                let detach = inner.as_str().to_uppercase().starts_with("DETACH");
                let expressions: Result<Vec<_>, _> = inner.into_inner().map(parse_expression).collect();
                clauses.push(Clause::Delete(DeleteClause {
                    detach,
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
pub(crate) fn parse_foreach_clause(pair: pest::iterators::Pair<Rule>) -> Result<ForeachClause, String> {
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
                                    let prop =
                                        parse_expression(parts.next().ok_or("Missing SET property".to_string())?)?;
                                    let val = parse_expression(parts.next().ok_or("Missing SET value".to_string())?)?;
                                    Ok(SetItem {
                                        property: prop,
                                        value: val,
                                    })
                                })
                                .collect();
                            sub_clauses.push(Clause::Set(SetClause { items: items? }));
                        }
                        Rule::delete_clause => {
                            let detach = body_inner.as_str().to_uppercase().starts_with("DETACH");
                            let expressions: Result<Vec<_>, _> =
                                body_inner.into_inner().map(parse_expression).collect();
                            sub_clauses.push(Clause::Delete(DeleteClause {
                                detach,
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

pub(crate) fn parse_patterns(pair: pest::iterators::Pair<Rule>) -> Result<Vec<Pattern>, String> {
    let mut patterns = Vec::new();
    for p in pair.into_inner() {
        if p.as_rule() == Rule::pattern {
            patterns.extend(parse_pattern_path(p)?);
        }
    }
    Ok(patterns)
}

pub(crate) fn parse_pattern_path(pair: pest::iterators::Pair<Rule>) -> Result<Vec<Pattern>, String> {
    let mut path = Vec::new();
    let mut current_node = None;
    let mut current_edge = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::node_pattern => {
                let node = parse_node_pattern(inner)?;
                if current_node.is_some() {
                    path.push(Pattern {
                        node: current_node.take(),
                        edge: current_edge.take(),
                    });
                }
                current_node = Some(node);
            }
            Rule::edge_pattern => {
                current_edge = Some(parse_edge_pattern(inner)?);
            }
            _ => {}
        }
    }

    if current_node.is_some() || current_edge.is_some() {
        path.push(Pattern {
            node: current_node.take(),
            edge: current_edge.take(),
        });
    }

    Ok(path)
}

pub(crate) fn parse_node_pattern(pair: pest::iterators::Pair<Rule>) -> Result<NodePattern, String> {
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

pub(crate) fn parse_edge_pattern(pair: pest::iterators::Pair<Rule>) -> Result<EdgePattern, String> {
    let mut variable = None;
    let mut labels = Vec::new();
    let mut properties = Vec::new();
    let mut lower_bound = None;
    let mut upper_bound = None;
    let text = pair.as_str();
    let direction = if text.starts_with("<-") {
        EdgeDirection::RightToLeft
    } else if text.ends_with("->") {
        EdgeDirection::LeftToRight
    } else {
        EdgeDirection::Both
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

pub(crate) fn parse_property_kv(pair: pest::iterators::Pair<Rule>) -> Result<(String, Expression), String> {
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

pub(crate) fn parse_return_items(pair: pest::iterators::Pair<Rule>) -> Result<Vec<ReturnItem>, String> {
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
    if items.is_empty() {
        // If there are no return_item children, it must be the `*` branch in the grammar.
        items.push(ReturnItem {
            expression: Expression::Star,
            alias: None,
        });
    }
    Ok(items)
}

/// Check if the return_clause pair has a DISTINCT flag.
pub(crate) fn has_distinct_flag(pair: &pest::iterators::Pair<Rule>) -> bool {
    pair.clone().into_inner().any(|c| c.as_rule() == Rule::distinct_flag)
}

/// Extract ORDER BY items from a return_clause or with_clause pair.
fn parse_order_by(pair: &pest::iterators::Pair<Rule>) -> Option<Vec<OrderByItem>> {
    let order_by_pair = pair.clone().into_inner().find(|p| p.as_rule() == Rule::order_by)?;
    let mut items = Vec::new();
    for part in order_by_pair.into_inner() {
        if part.as_rule() == Rule::sort_item {
            let sort_text = part.as_str().trim().to_uppercase();
            let ascending = !sort_text.ends_with("DESC");
            let mut expr = None;
            for inner in part.into_inner() {
                if inner.as_rule() == Rule::expression {
                    expr = Some(parse_expression(inner).ok()?);
                }
            }
            items.push(OrderByItem {
                expression: expr?,
                ascending,
            });
        }
    }
    if items.is_empty() { None } else { Some(items) }
}

/// Extract LIMIT and SKIP values from a return_clause or with_clause pair.
fn parse_limit_skip(pair: &pest::iterators::Pair<Rule>) -> (Option<u64>, Option<u64>) {
    let limit_pair = match pair.clone().into_inner().find(|p| p.as_rule() == Rule::limit) {
        Some(p) => p,
        None => return (None, None),
    };
    let mut limit_val = None;
    let mut skip_val = None;
    for inner in limit_pair.into_inner() {
        match inner.as_rule() {
            Rule::integer => {
                if limit_val.is_none() {
                    limit_val = inner.as_str().parse::<u64>().ok();
                }
            }
            Rule::offset => {
                for off_inner in inner.into_inner() {
                    if off_inner.as_rule() == Rule::integer {
                        skip_val = off_inner.as_str().parse::<u64>().ok();
                    }
                }
            }
            _ => {}
        }
    }
    (limit_val, skip_val)
}

pub fn parse_call(pair: pest::iterators::Pair<Rule>) -> Result<StandaloneCall, String> {
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

    Ok(StandaloneCall { function_name, args })
}

/// Parse a MERGE statement.
pub fn parse_merge(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut patterns = Vec::new();
    let mut on_create = Vec::new();
    let mut on_match = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::merge_clause => {
                for part in inner.into_inner() {
                    match part.as_rule() {
                        Rule::pattern => {
                            patterns.extend(parse_pattern_path(part)?);
                        }
                        Rule::on_create_set => {
                            for item in part.into_inner() {
                                if item.as_rule() == Rule::set_item {
                                    let mut p = item.into_inner();
                                    let prop = parse_expression(p.next().ok_or("Missing ON CREATE SET property")?)?;
                                    let val = parse_expression(p.next().ok_or("Missing ON CREATE SET value")?)?;
                                    on_create.push(SetItem {
                                        property: prop,
                                        value: val,
                                    });
                                }
                            }
                        }
                        Rule::on_match_set => {
                            for item in part.into_inner() {
                                if item.as_rule() == Rule::set_item {
                                    let mut p = item.into_inner();
                                    let prop = parse_expression(p.next().ok_or("Missing ON MATCH SET property")?)?;
                                    let val = parse_expression(p.next().ok_or("Missing ON MATCH SET value")?)?;
                                    on_match.push(SetItem {
                                        property: prop,
                                        value: val,
                                    });
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

    Ok(Statement::Merge(MergeStatement {
        patterns,
        on_create,
        on_match,
    }))
}
