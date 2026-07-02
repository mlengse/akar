//! Parser implementation — converts Cypher query text to AST using pest.rs PEG grammar.

use crate::ast::*;
use pest::Parser;
use std::collections::HashMap;

#[derive(pest_derive::Parser)]
#[grammar = "cypher.pest"]
pub struct CypherParser;

pub fn parse(input: &str) -> Result<Statement, String> {
    let trimmed = input.trim();

    // Handle EXPLAIN prefix before PEG parsing to avoid grammar recursion
    if let Some(rest) = trimmed.strip_prefix("EXPLAIN").map(|s| s.trim()) {
        let (explain_type, inner_sql) = if let Some(rest) = rest.strip_prefix("LOGICAL").map(|s| s.trim()) {
            (ExplainType::LogicalPlan, rest)
        } else if let Some(rest) = rest.strip_prefix("PROFILE").map(|s| s.trim()) {
            (ExplainType::Profile, rest)
        } else {
            (ExplainType::PhysicalPlan, rest)
        };
        let inner_stmt = parse(inner_sql)?;
        return Ok(Statement::Explain(ExplainStatement::new(inner_stmt, explain_type)));
    }

    let mut pairs = CypherParser::parse(Rule::kuzu_query, trimmed).map_err(|e| format!("Parse error: {e}"))?;
    let kuzu_query_pair = pairs.next().ok_or("Empty input")?;
    let statement_pair = kuzu_query_pair.into_inner().next().ok_or("No statement in query")?;
    parse_statement(statement_pair)
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
        Rule::union_statement => {
            let inner_clone = inner.clone();
            parse_ddl(inner_clone)
        }
        Rule::call_statement => {
            let call = parse_call(inner)?;
            Ok(Statement::Call(call))
        }
        Rule::export_database => parse_export_database(inner),
        Rule::import_database => parse_import_database(inner),
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
        Rule::create_index => parse_create_index(pair),
        Rule::drop_index => parse_drop_index(pair),
        Rule::create_sequence => parse_create_sequence(pair),
        Rule::drop_sequence => parse_drop_sequence(pair),
        Rule::create_macro => parse_create_macro(pair),
        Rule::export_database => parse_export_database(pair),
        Rule::import_database => parse_import_database(pair),
        _ => Err(format!("Unknown DDL: {:?}", pair.as_rule())),
    }
}

fn parse_export_database(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut file_path = String::new();
    let mut options = std::collections::HashMap::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::string => {
                let raw = inner.as_str().trim();
                file_path = raw.trim_matches('\'').to_string();
            }
            Rule::export_option => {
                let mut key = String::new();
                let mut val = String::new();
                for part in inner.into_inner() {
                    match part.as_rule() {
                        Rule::identifier => key = part.as_str().to_string(),
                        Rule::literal => {
                            let raw = part.as_str();
                            val = raw.trim_matches('\'').to_string();
                        }
                        _ => {}
                    }
                }
                if !key.is_empty() {
                    options.insert(key.to_uppercase(), val);
                }
            }
            _ => {}
        }
    }
    Ok(Statement::ExportDatabase(ExportDatabase { file_path, options }))
}

fn parse_import_database(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut file_path = String::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::string {
            file_path = inner.as_str().trim().trim_matches('\'').to_string();
        }
    }
    Ok(Statement::ImportDatabase(ImportDatabase { file_path }))
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

/// Parse `CREATE [ART|HASH] INDEX [IF NOT EXISTS] name FOR (var:Label) ON (var.prop)`.
fn parse_create_index(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut index_type = String::new();
    let mut index_name = String::new();
    let mut table_name = String::new();
    let mut variable = String::new();
    let mut property = String::new();
    let mut conflict_action: Option<String> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::index_type => {
                index_type = inner.as_str().to_uppercase();
            }
            Rule::identifier if index_name.is_empty() => {
                index_name = inner.as_str().to_string();
            }
            Rule::identifier if table_name.is_empty() => {
                // Table name inside FOR parentheses
                table_name = inner.as_str().to_string();
            }
            Rule::identifier if variable.is_empty() => {
                // Variable inside FOR parentheses
                variable = inner.as_str().to_string();
            }
            Rule::identifier if property.is_empty() => {
                // Property name after the dot
                property = inner.as_str().to_string();
            }
            Rule::if_not_exists => {
                conflict_action = Some("IF_NOT_EXISTS".into());
            }
            _ => {}
        }
    }

    if index_type.is_empty() {
        return Err("Missing index type: use ART or HASH".into());
    }
    if index_name.is_empty() {
        return Err("Missing index name for CREATE INDEX".into());
    }
    if table_name.is_empty() {
        return Err("Missing table name for CREATE INDEX".into());
    }
    if variable.is_empty() {
        return Err("Missing variable for CREATE INDEX".into());
    }
    if property.is_empty() {
        return Err("Missing property for CREATE INDEX".into());
    }

    Ok(Statement::CreateIndex(CreateIndex {
        index_type,
        index_name,
        table_name,
        variable,
        property,
        conflict_action,
    }))
}

/// Parse `DROP INDEX name ON table`.
fn parse_drop_index(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut index_name = String::new();
    let mut table_name = String::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier if index_name.is_empty() => {
                index_name = inner.as_str().to_string();
            }
            Rule::identifier if table_name.is_empty() => {
                table_name = inner.as_str().to_string();
            }
            _ => {}
        }
    }

    if index_name.is_empty() {
        return Err("Missing index name for DROP INDEX".into());
    }
    if table_name.is_empty() {
        return Err("Missing table name for DROP INDEX".into());
    }

    Ok(Statement::DropIndex(DropIndex {
        index_name,
        table_name,
    }))
}

/// Parse `CREATE [OR REPLACE] SEQUENCE [IF NOT EXISTS] name [START WITH n] [INCREMENT [BY] n] [MINVALUE n|NO MINVALUE] [MAXVALUE n|NO MAXVALUE] [CYCLE|NO CYCLE]`.
fn parse_create_sequence(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut name = String::new();
    let mut if_not_exists = false;
    let mut or_replace = false;
    let mut start_with: Option<i64> = None;
    let mut increment: Option<i64> = None;
    let mut min_value: Option<i64> = None;
    let mut max_value: Option<i64> = None;
    let mut cycle: Option<bool> = None;

    /// Parse a single sequence option from its pair.
    fn parse_opt(
        p: pest::iterators::Pair<Rule>,
        start_with: &mut Option<i64>,
        increment: &mut Option<i64>,
        min_value: &mut Option<i64>,
        max_value: &mut Option<i64>,
        cycle: &mut Option<bool>,
    ) -> Result<(), String> {
        /// Extract an i64 from the inner tokens, handling optional minus prefix.
        fn extract_int<'a>(parts: impl Iterator<Item = pest::iterators::Pair<'a, Rule>>) -> Result<i64, String> {
            let mut neg = false;
            let mut val: Option<i64> = None;
            for part in parts {
                match part.as_rule() {
                    Rule::minus => neg = true,
                    Rule::integer => {
                        let raw = part.as_str().trim();
                        let v = raw.parse::<i64>()
                            .map_err(|e| format!("Invalid integer '{raw}': {e}"))?;
                        val = Some(if neg { -v } else { v });
                    }
                    _ => {}
                }
            }
            val.ok_or_else(|| "Missing integer value".into())
        }

        match p.as_rule() {
            Rule::sequence_start_with => {
                let v = extract_int(p.into_inner())?;
                *start_with = Some(v);
            }
            Rule::sequence_increment_by => {
                let v = extract_int(p.into_inner())?;
                *increment = Some(v);
            }
            Rule::sequence_minvalue => {
                let text = p.as_str().to_uppercase();
                if text.starts_with("NO") {
                    *min_value = None;
                } else {
                    let v = extract_int(p.into_inner())?;
                    *min_value = Some(v);
                }
            }
            Rule::sequence_maxvalue => {
                let text = p.as_str().to_uppercase();
                if text.starts_with("NO") {
                    *max_value = None;
                } else {
                    let v = extract_int(p.into_inner())?;
                    *max_value = Some(v);
                }
            }
            Rule::sequence_cycle => {
                let text = p.as_str().to_uppercase();
                *cycle = Some(!text.starts_with("NO"));
            }
            _ => {}
        }
        Ok(())
    }

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier if name.is_empty() => {
                name = inner.as_str().to_string();
            }
            Rule::if_not_exists => {
                if_not_exists = true;
            }
            Rule::or_replace => {
                or_replace = true;
            }
            Rule::sequence_start_with | Rule::sequence_increment_by
            | Rule::sequence_minvalue | Rule::sequence_maxvalue
            | Rule::sequence_cycle => {
                parse_opt(inner, &mut start_with, &mut increment,
                    &mut min_value, &mut max_value, &mut cycle)?;
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err("Missing sequence name".into());
    }

    Ok(Statement::CreateSequence(CreateSequence {
        name,
        if_not_exists,
        or_replace,
        start_with,
        increment,
        min_value,
        max_value,
        cycle,
    }))
}

/// Parse `DROP SEQUENCE [IF EXISTS] name`.
fn parse_drop_sequence(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut name = String::new();
    let mut if_exists = false;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier => {
                name = inner.as_str().to_string();
            }
            Rule::if_exists => {
                if_exists = true;
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err("Missing sequence name for DROP SEQUENCE".into());
    }

    Ok(Statement::DropSequence(DropSequence { name, if_exists }))
}

/// Parse `CREATE MACRO name(params) AS expression`.
fn parse_create_macro(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut name = String::new();
    let mut positional_args: Vec<String> = Vec::new();
    let mut default_args: Vec<(String, Expression)> = Vec::new();
    let mut expression: Option<Expression> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier if name.is_empty() => {
                name = inner.as_str().to_string();
            }
            Rule::macro_params => {
                for param in inner.into_inner() {
                    if param.as_rule() == Rule::macro_param {
                        let mut arg_name = String::new();
                        let mut default_val: Option<Expression> = None;
                        for part in param.into_inner() {
                            match part.as_rule() {
                                Rule::identifier if arg_name.is_empty() => {
                                    arg_name = part.as_str().to_string();
                                }
                                Rule::literal => {
                                    // literal wraps inner token types (integer, string, etc.)
                                    if let Some(inner) = part.into_inner().next() {
                                        default_val = Some(parse_literal(inner)?);
                                    }
                                }
                                _ => {}
                            }
                        }
                        if let Some(expr) = default_val {
                            default_args.push((arg_name, expr));
                        } else if !arg_name.is_empty() {
                            positional_args.push(arg_name);
                        }
                    }
                }
            }
            Rule::expression => {
                expression = Some(parse_expression(inner)?);
            }
            _ => {}
        }
    }

    let expr = expression.ok_or_else(|| "CREATE MACRO requires an AS expression".to_string())?;

    Ok(Statement::CreateMacro(CreateMacro {
        name,
        positional_args,
        default_args,
        expression: Box::new(expr),
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
    let mut patterns = Vec::new();
    for p in pair.into_inner() {
        if p.as_rule() == Rule::pattern {
            patterns.extend(parse_pattern_path(p)?);
        }
    }
    Ok(patterns)
}

fn parse_pattern_path(pair: pest::iterators::Pair<Rule>) -> Result<Vec<Pattern>, String> {
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
    if items.is_empty() {
        // If there are no return_item children, it must be the `*` branch in the grammar.
        items.push(ReturnItem { expression: Expression::Star, alias: None });
    }
    Ok(items)
}

fn parse_expression(pair: pest::iterators::Pair<Rule>) -> Result<Expression, String> {
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
                let args = children[1]
                    .clone()
                    .into_inner()
                    .filter(|c| c.as_rule() == Rule::expression)
                    .map(parse_expression)
                    .collect::<Result<Vec<_>, _>>()?;
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
                match child.as_rule() {
                    Rule::property_access => {
                        let prop = child.into_inner().next().unwrap().as_str().to_string();
                        result = Expression::PropertyAccess(Box::new(result), prop);
                    }
                    _ => {}
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
            Err("property_access should be handled within postfix_expr".into())
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

/// Parse a comparison suffix operator node into an expression given the left-hand side.
fn parse_comparison_suffix(
    pair: pest::iterators::Pair<Rule>,
    left: Expression,
) -> Result<Expression, String> {
    /// Find the additive_expr child inside an operator rule.
    fn get_rhs(pair: pest::iterators::Pair<Rule>) -> Result<Expression, String> {
        let rhs_pair = pair
            .into_inner()
            .find(|p| p.as_rule() == Rule::additive_expr)
            .ok_or_else(|| "Operator missing right-hand expression".to_string())?;
        parse_expression(rhs_pair)
    }

    match pair.as_rule() {
        Rule::is_null_op => Ok(Expression::UnaryOp(UnaryOp::IsNull, Box::new(left))),
        Rule::is_not_null_op => Ok(Expression::UnaryOp(UnaryOp::IsNotNull, Box::new(left))),
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
            Ok(Expression::BinaryOp(BinaryOp::StartsWith, Box::new(left), Box::new(right)))
        }
        Rule::not_starts_with_op => {
            let right = get_rhs(pair)?;
            let inner = Expression::BinaryOp(BinaryOp::StartsWith, Box::new(left), Box::new(right));
            Ok(Expression::UnaryOp(UnaryOp::Not, Box::new(inner)))
        }
        Rule::ends_with_op => {
            let right = get_rhs(pair)?;
            Ok(Expression::BinaryOp(BinaryOp::EndsWith, Box::new(left), Box::new(right)))
        }
        Rule::not_ends_with_op => {
            let right = get_rhs(pair)?;
            let inner = Expression::BinaryOp(BinaryOp::EndsWith, Box::new(left), Box::new(right));
            Ok(Expression::UnaryOp(UnaryOp::Not, Box::new(inner)))
        }
        Rule::contains_op => {
            let right = get_rhs(pair)?;
            Ok(Expression::BinaryOp(BinaryOp::Contains, Box::new(left), Box::new(right)))
        }
        Rule::not_contains_op => {
            let right = get_rhs(pair)?;
            let inner = Expression::BinaryOp(BinaryOp::Contains, Box::new(left), Box::new(right));
            Ok(Expression::UnaryOp(UnaryOp::Not, Box::new(inner)))
        }
        r => Err(format!("Unknown comparison suffix: {:?}", r)),
    }
}

/// Parse a `CASE [subject] WHEN ... THEN ... [ELSE ...] END` expression.
fn parse_case_expr(pair: pest::iterators::Pair<Rule>) -> Result<Expression, String> {
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
                if let Some(expr_pair) = child
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::expression)
                {
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

fn parse_literal(pair: pest::iterators::Pair<Rule>) -> Result<Expression, String> {
    match pair.as_rule() {
        Rule::string => Ok(Expression::Constant(Constant::String(unescape_string(pair.as_str())))),
        Rule::integer => {
            let s = pair.as_str();
            Ok(Expression::Constant(Constant::Integer(
            s.trim().parse().map_err(|e| format!("Int: {e} (for string '{s}')"))?,
        )))
        },
        Rule::float => {
            let s = pair.as_str();
            Ok(Expression::Constant(Constant::Float(
            s.trim().parse().map_err(|e| format!("Float: {e} (for string '{s}')"))?,
        )))
        },
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
    let mut patterns = Vec::new();
    let mut on_create = Vec::new();
    let mut on_match = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::merge_clause => {
                for part in inner.into_inner() {
                    match part.as_rule() {
                        Rule::pattern => {
                            patterns = parse_pattern_path(part)?;
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

    Ok(Statement::Merge(MergeStatement {
        patterns,
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
    fn test_lower_upper_parse() {
        let sql = "RETURN lower('HELLO'), upper('hello')";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_cast_aliases_parse() {
        let sql = "RETURN date('2024-01-01'), float('3.14'), bool('true')";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_function_call() {
        let sql = "MATCH (a:Person) RETURN COUNT(a)";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_function_call_ast_nextval_currval() {
        let sql = "RETURN nextval('my_seq'), currval('my_seq')";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Query(q) => {
                assert_eq!(q.clauses.len(), 1);
                match &q.clauses[0] {
                    Clause::Return(r) => {
                        assert_eq!(r.expressions.len(), 2);
                        assert!(matches!(
                            &r.expressions[0].expression,
                            Expression::FunctionCall(name, args)
                            if name == "nextval" && args.len() == 1
                        ));
                        assert!(matches!(
                            &r.expressions[1].expression,
                            Expression::FunctionCall(name, args)
                            if name == "currval" && args.len() == 1
                        ));
                    }
                    _ => panic!("Expected Return clause"),
                }
            }
            _ => panic!("Expected Query"),
        }
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

    // --- EXPLAIN tests ---
    #[test]
    fn test_explain_query() {
        let sql = "EXPLAIN MATCH (a:Person) RETURN a";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Explain(e) => {
                assert_eq!(e.explain_type, ExplainType::PhysicalPlan);
                assert!(matches!(*e.statement, Statement::Query(_)));
            }
            _ => panic!("Expected Explain, got {:?}", stmt),
        }
    }

    #[test]
    fn test_explain_logical() {
        let sql = "EXPLAIN LOGICAL MATCH (a:Person) RETURN a";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Explain(e) => {
                assert_eq!(e.explain_type, ExplainType::LogicalPlan);
            }
            _ => panic!("Expected Explain(Logical)"),
        }
    }

    #[test]
    fn test_explain_profile() {
        let sql = "EXPLAIN PROFILE MATCH (a:Person) RETURN a";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Explain(e) => {
                assert_eq!(e.explain_type, ExplainType::Profile);
            }
            _ => panic!("Expected Explain(Profile)"),
        }
    }

    #[test]
    fn test_explain_create_table() {
        let sql = "EXPLAIN CREATE NODE TABLE Test(id INT64, PRIMARY KEY (id))";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Explain(e) => {
                assert!(matches!(*e.statement, Statement::CreateNodeTable(_)));
            }
            _ => panic!("Expected Explain(CreateNodeTable)"),
        }
    }

    // --- Sequence tests ---

    #[test]
    fn test_create_sequence_basic() {
        let sql = "CREATE SEQUENCE my_seq";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert!(!s.if_not_exists);
                assert!(!s.or_replace);
                assert_eq!(s.start_with, None);
                assert_eq!(s.increment, None);
            }
            _ => panic!("Expected CreateSequence, got {:?}", stmt),
        }
    }

    #[test]
    fn test_create_sequence_if_not_exists() {
        let sql = "CREATE SEQUENCE IF NOT EXISTS my_seq";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert!(s.if_not_exists);
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_create_sequence_or_replace() {
        let sql = "CREATE OR REPLACE SEQUENCE my_seq";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert!(s.or_replace);
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_create_sequence_start_with() {
        let sql = "CREATE SEQUENCE my_seq START 100";
        let result = parse(sql);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());
        let stmt = result.unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert_eq!(s.start_with, Some(100));
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_create_sequence_start_with_with() {
        let sql = "CREATE SEQUENCE my_seq START WITH 100";
        let result = parse(sql);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());
        let stmt = result.unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert_eq!(s.start_with, Some(100));
            }
            _ => panic!("Expected CreateSequence, got {:?}", stmt),
        }
    }

    #[test]
    fn test_create_sequence_full_options() {
        let sql = "CREATE SEQUENCE my_seq START 100 INCREMENT 5 MINVALUE 1 MAXVALUE 1000 CYCLE";
        let result = parse(sql);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());
        let stmt = result.unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert_eq!(s.start_with, Some(100));
                assert_eq!(s.increment, Some(5));
                assert_eq!(s.min_value, Some(1));
                assert_eq!(s.max_value, Some(1000));
                assert_eq!(s.cycle, Some(true));
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_create_sequence_no_cycle() {
        let sql = "CREATE SEQUENCE my_seq NO CYCLE";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.cycle, Some(false));
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_create_sequence_no_minvalue() {
        let sql = "CREATE SEQUENCE my_seq NO MINVALUE";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.min_value, None);
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_create_sequence_negative_increment() {
        let sql = "CREATE SEQUENCE my_seq INCREMENT -1 START 100";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.increment, Some(-1));
                assert_eq!(s.start_with, Some(100));
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_drop_sequence() {
        let sql = "DROP SEQUENCE my_seq";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::DropSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert!(!s.if_exists);
            }
            _ => panic!("Expected DropSequence"),
        }
    }

    #[test]
    fn test_drop_sequence_if_exists() {
        let sql = "DROP SEQUENCE IF EXISTS my_seq";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::DropSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert!(s.if_exists);
            }
            _ => panic!("Expected DropSequence"),
        }
    }

    #[test]
    fn test_export_database_basic() {
        let sql = "EXPORT DATABASE '/tmp/export'";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::ExportDatabase(e) => {
                assert_eq!(e.file_path, "/tmp/export");
                assert!(e.options.is_empty());
            }
            _ => panic!("Expected ExportDatabase"),
        }
    }

    #[test]
    fn test_import_database_basic() {
        let sql = "IMPORT DATABASE '/tmp/export'";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::ImportDatabase(i) => {
                assert_eq!(i.file_path, "/tmp/export");
            }
            _ => panic!("Expected ImportDatabase"),
        }
    }

    // ==================== CREATE MACRO tests ====================

    #[test]
    fn test_create_macro_no_args() {
        let sql = "CREATE MACRO my_macro() AS 42";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateMacro(m) => {
                assert_eq!(m.name, "my_macro");
                assert!(m.positional_args.is_empty());
                assert!(m.default_args.is_empty());
            }
            _ => panic!("Expected CreateMacro"),
        }
    }

    #[test]
    fn test_create_macro_positional_args() {
        let sql = "CREATE MACRO double(x) AS x * 2";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateMacro(m) => {
                assert_eq!(m.name, "double");
                assert_eq!(m.positional_args, vec!["x"]);
                assert!(m.default_args.is_empty());
            }
            _ => panic!("Expected CreateMacro"),
        }
    }

    #[test]
    fn test_create_macro_multiple_positional_args() {
        let sql = "CREATE MACRO add(x, y) AS x + y";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateMacro(m) => {
                assert_eq!(m.name, "add");
                assert_eq!(m.positional_args, vec!["x", "y"]);
            }
            _ => panic!("Expected CreateMacro"),
        }
    }

    #[test]
    fn test_create_macro_with_default_arg() {
        let sql = "CREATE MACRO inc(x, y = 1) AS x + y";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateMacro(m) => {
                assert_eq!(m.name, "inc");
                assert_eq!(m.positional_args, vec!["x"]);
                assert_eq!(m.default_args.len(), 1);
                assert_eq!(m.default_args[0].0, "y");
            }
            _ => panic!("Expected CreateMacro"),
        }
    }

    #[test]
    fn test_create_macro_expression_body() {
        let sql = "CREATE MACRO square(x) AS x * x";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateMacro(m) => {
                assert_eq!(m.name, "square");
                assert_eq!(m.positional_args, vec!["x"]);
                // Expression should be BinaryOp(Mul, Variable("x"), Variable("x"))
                match &*m.expression {
                    Expression::BinaryOp(op, _, _) => {
                        assert_eq!(*op, BinaryOp::Multiply);
                    }
                    _ => panic!("Expected BinaryOp in macro body"),
                }
            }
            _ => panic!("Expected CreateMacro"),
        }
    }

    // --- List predicate tests ---

    #[test]
    fn test_list_predicate_any() {
        let sql = "RETURN ANY(x IN [1,2,3] WHERE x > 2)";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Query(q) => {
                match &q.clauses[0] {
                    Clause::Return(r) => {
                        assert_eq!(r.expressions.len(), 1);
                        match &r.expressions[0].expression {
                            Expression::ListPredicate { quantifier, var_name, list, predicate } => {
                                assert_eq!(*quantifier, Quantifier::Any);
                                assert_eq!(var_name, "x");
                                assert!(matches!(&**list, Expression::List(_)));
                                assert!(matches!(&**predicate, Expression::BinaryOp(_, _, _)));
                            }
                            _ => panic!("Expected ListPredicate"),
                        }
                    }
                    _ => panic!("Expected Return clause"),
                }
            }
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn test_list_predicate_all() {
        let sql = "RETURN ALL(x IN list WHERE x > 0)";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_list_predicate_none() {
        let sql = "MATCH (n) WHERE NONE(x IN n.list WHERE x = 0) RETURN n";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_list_predicate_single() {
        let sql = "MATCH (n) WHERE SINGLE(x IN n.list WHERE x < 0) RETURN n";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_list_predicate_in_return() {
        let sql = "RETURN ALL(x IN [true, false] WHERE x)";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_list_predicate_nested() {
        let sql = "RETURN ANY(x IN [1,2,3] WHERE x > 0 AND x < 5)";
        assert!(parse(sql).is_ok());
    }
}
