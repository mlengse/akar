//! DDL parsing — CREATE/DROP TABLE, INDEX, SEQUENCE, ALTER, COPY, CALL, EXPORT/IMPORT, ANALYZE, MACRO, FTS.

use super::Rule;
use crate::ast::*;
use crate::parser::dml::parse_query_pairs;
use crate::parser::expression::{parse_expression, parse_literal, unescape_string};
use std::collections::HashMap;

pub(crate) fn parse_ddl(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
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
                            let mut comp = None;
                            for part in col.into_inner() {
                                match part.as_rule() {
                                    Rule::identifier => cn = part.as_str().to_string(),
                                    Rule::type_name => ct = part.as_str().to_string(),
                                    Rule::string => comp = Some(unescape_string(part.as_str())),
                                    _ => {}
                                }
                            }
                            columns.push(ColumnDef {
                                name: cn,
                                type_name: ct,
                                compression: comp,
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
                            let mut comp = None;
                            for part in col.into_inner() {
                                match part.as_rule() {
                                    Rule::identifier => cn = part.as_str().to_string(),
                                    Rule::type_name => ct = part.as_str().to_string(),
                                    Rule::string => comp = Some(unescape_string(part.as_str())),
                                    _ => {}
                                }
                            }
                            columns.push(ColumnDef {
                                name: cn,
                                type_name: ct,
                                compression: comp,
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
            let all = union_keyword.as_str().to_uppercase().contains("UNION ALL");
            let right = parse_query_pairs(inner.next().ok_or("Missing right query")?)?;
            Ok(Statement::Union(UnionStatement { left, right, all }))
        }
        Rule::copy_from => parse_copy_from(pair),
        Rule::copy_to => parse_copy_to(pair),
        Rule::alter_table => parse_alter_table(pair),
        Rule::create_vector_index => parse_create_vector_index(pair),
        Rule::create_index => parse_create_index(pair),
        Rule::drop_index => parse_drop_index(pair),
        Rule::create_sequence => parse_create_sequence(pair),
        Rule::drop_sequence => parse_drop_sequence(pair),
        Rule::create_macro => parse_create_macro(pair),
        Rule::create_fts_index => parse_create_fts_index(pair),
        Rule::export_database => parse_export_database(pair),
        Rule::import_database => parse_import_database(pair),
        Rule::create_type => parse_create_type(pair),
        Rule::comment_on_table => parse_comment_on_table(pair),
        Rule::create_graph => parse_create_graph(pair),
        Rule::use_graph => parse_use_graph(pair),
        Rule::drop_graph => parse_drop_graph(pair),
        _ => Err(format!("Unknown DDL: {:?}", pair.as_rule())),
    }
}

pub(crate) fn parse_export_database(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut file_path = String::new();
    let mut options = HashMap::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::string {
            let raw = inner.as_str().trim();
            file_path = unescape_string(raw);
        }
        parse_options_into(inner, &mut options);
    }
    Ok(Statement::ExportDatabase(ExportDatabase { file_path, options }))
}

/// Collect `(key, value)` pairs from an options clause into `out`.
///
/// The grammar nests each option under a `export_options` parent pair
/// (`import/export_database -> export_options -> export_option`), so callers
/// must descend both levels (the old loop matched only the outer level and
/// silently dropped every option).
fn parse_options_into(pair: pest::iterators::Pair<Rule>, out: &mut HashMap<String, String>) {
    match pair.as_rule() {
        Rule::export_options => {
            for inner in pair.into_inner() {
                parse_options_into(inner, out);
            }
        }
        Rule::export_option => {
            let mut key = String::new();
            let mut val = String::new();
            for part in pair.into_inner() {
                match part.as_rule() {
                    Rule::identifier => key = part.as_str().to_string(),
                    Rule::literal => val = unescape_string(part.as_str()),
                    _ => {}
                }
            }
            if !key.is_empty() {
                out.insert(key.to_uppercase(), val);
            }
        }
        _ => {}
    }
}

pub(crate) fn parse_import_database(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut file_path = String::new();
    let mut options = HashMap::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::string {
            file_path = unescape_string(inner.as_str().trim());
        }
        parse_options_into(inner, &mut options);
    }
    Ok(Statement::ImportDatabase(ImportDatabase { file_path, options }))
}

/// Parse ANALYZE (TABLE) <name> | ANALYZE *
pub(crate) fn parse_analyze(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let inner = pair.as_str().trim();
    // Format: "ANALYZE TABLE Foo" or "ANALYZE Foo" or "ANALYZE *"
    let rest = inner.strip_prefix("ANALYZE").unwrap_or(inner).trim();
    let rest = rest.strip_prefix("TABLE").unwrap_or(rest).trim();
    let table_name = if rest == "*" { None } else { Some(rest.to_string()) };
    Ok(Statement::Analyze(AnalyzeStatement { table_name }))
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
                    // The grammar wraps each option in `vector_index_option`
                    // (metric_option | dimensions_option).
                    if opt.as_rule() == Rule::vector_index_option {
                        for part in opt.into_inner() {
                            match part.as_rule() {
                                Rule::metric_option => {
                                    // pest does not expose string literals as inner
                                    // pairs, so parse the metric value from text.
                                    let s = part.as_str();
                                    metric = s.split_once('=').map(|(_, v)| v.trim().to_string()).unwrap_or_default();
                                }
                                Rule::dimensions_option => {
                                    for p in part.into_inner() {
                                        if p.as_rule() == Rule::integer {
                                            dimensions = Some(
                                                p.as_str()
                                                    .parse::<u64>()
                                                    .map_err(|e| format!("Invalid dimensions value: {e}"))?,
                                            );
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
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
    let mut conflict_action: Option<String> = None;
    let mut identifiers: Vec<String> = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::index_type => {
                index_type = inner.as_str().to_uppercase();
            }
            Rule::identifier => {
                identifiers.push(inner.as_str().to_string());
            }
            Rule::if_not_exists => {
                conflict_action = Some("IF_NOT_EXISTS".into());
            }
            _ => {}
        }
    }

    // Grammar: CREATE TYPE INDEX index_name FOR (variable:table_name) ON (variable.property)
    // Identifiers in order: [index_name, variable, table_name, on_variable, property]
    if identifiers.len() < 5 {
        return Err(format!(
            "Expected 5 identifiers for CREATE INDEX, got {}",
            identifiers.len()
        ));
    }

    let index_name = identifiers[0].clone();
    let variable = identifiers[1].clone();
    let table_name = identifiers[2].clone();
    // identifiers[3] is the ON-clause variable (alias), skip it
    let property = identifiers[4].clone();

    if index_type.is_empty() {
        return Err("Missing index type: use ART or HASH".into());
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

    Ok(Statement::DropIndex(DropIndex { index_name, table_name }))
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
                        let v = raw
                            .parse::<i64>()
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
            Rule::sequence_start_with
            | Rule::sequence_increment_by
            | Rule::sequence_minvalue
            | Rule::sequence_maxvalue
            | Rule::sequence_cycle => {
                parse_opt(
                    inner,
                    &mut start_with,
                    &mut increment,
                    &mut min_value,
                    &mut max_value,
                    &mut cycle,
                )?;
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

/// Parse a COPY TO statement.
fn parse_copy_to(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut query: Option<Query> = None;
    let mut file_path = String::new();
    let mut format = CopyToFormat::Csv;
    let mut header = false;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::query_statement => {
                query = Some(parse_query_pairs(inner)?);
            }
            Rule::string => {
                file_path = unescape_string(inner.as_str());
            }
            Rule::copy_to_options => {
                for item in inner.into_inner() {
                    if item.as_rule() != Rule::copy_to_option {
                        continue;
                    }
                    // Each copy_to_option contains one inner rule
                    let Some(opt_inner) = item.into_inner().next() else {
                        continue;
                    };
                    let rule = opt_inner.as_rule();
                    if rule == Rule::format_option {
                        let text = opt_inner.as_str().trim();
                        let fmt_upper = text.to_uppercase();
                        if fmt_upper.contains("CSV") {
                            format = CopyToFormat::Csv;
                        } else if fmt_upper.contains("PARQUET") {
                            format = CopyToFormat::Parquet;
                        }
                    } else if rule == Rule::header_option {
                        for val in opt_inner.into_inner() {
                            if val.as_rule() == Rule::boolean_literal || val.as_rule() == Rule::integer {
                                header = val.as_str().eq_ignore_ascii_case("true") || val.as_str() == "1";
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let query = query.ok_or("COPY TO requires a query in parentheses")?;
    Ok(Statement::CopyTo(CopyTo {
        query,
        file_path,
        format,
        header,
    }))
}

// ==================== FTS Parsing Helpers ====================

pub(crate) fn parse_create_fts_index(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut if_not_exists = false;
    let mut identifiers: Vec<String> = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::if_not_exists => if_not_exists = true,
            Rule::identifier => identifiers.push(inner.as_str().to_string()),
            _ => {}
        }
    }

    // identifiers: [index_name, table_name, column_name]
    if identifiers.len() < 3 {
        return Err(format!(
            "CREATE FTS INDEX requires index_name, table_name, and column_name, got {:?}",
            identifiers
        ));
    }
    Ok(Statement::CreateFtsIndex(CreateFtsIndex {
        index_name: identifiers[0].clone(),
        table_name: identifiers[1].clone(),
        column_name: identifiers[2].clone(),
        if_not_exists,
    }))
}

pub(crate) fn parse_using_fts_clause(pair: pest::iterators::Pair<Rule>) -> Result<FtsQuery, String> {
    let mut index_name = String::new();
    let mut query_string = String::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier => index_name = inner.as_str().to_string(),
            Rule::string => {
                query_string = inner.as_str().trim().trim_matches('\'').to_string();
            }
            _ => {}
        }
    }

    if index_name.is_empty() {
        return Err("USING FTS INDEX requires an index name".into());
    }
    Ok(FtsQuery {
        index_name,
        query_string,
    })
}

pub(crate) fn parse_transaction(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let text = pair.as_str();
    let upper = text.to_uppercase();
    let action = if upper.starts_with("BEGIN") {
        TransactionAction::Begin
    } else if upper.starts_with("COMMIT") {
        TransactionAction::Commit
    } else if upper.starts_with("ROLLBACK") {
        TransactionAction::Rollback
    } else if upper == "CHECKPOINT" {
        TransactionAction::Checkpoint
    } else {
        return Err(format!("Unknown transaction command: {text}"));
    };
    Ok(Statement::Transaction(TransactionStatement { action }))
}

pub(crate) fn parse_extension(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let text = pair.as_str();
    let mut name = String::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::identifier {
            name = inner.as_str().to_string();
        }
    }
    let upper = text.to_uppercase();
    let action = if upper.starts_with("INSTALL") {
        ExtensionAction::Install
    } else if upper.starts_with("LOAD") {
        ExtensionAction::Load
    } else if upper.starts_with("UNINSTALL") {
        ExtensionAction::Uninstall
    } else {
        return Err(format!("Unknown extension command: {text}"));
    };
    if name.is_empty() {
        return Err("Extension name is required".into());
    }
    Ok(Statement::Extension(ExtensionStatement { action, name }))
}

// ==================== Multi-DB ====================

pub(crate) fn parse_multi_db(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    match pair.as_rule() {
        Rule::attach_database => parse_attach_database(pair),
        Rule::detach_database => parse_detach_database(pair),
        Rule::use_database => parse_use_database(pair),
        Rule::load_from => parse_load_from(pair),
        _ => Err(format!("Unexpected multi-DB rule: {:?}", pair.as_rule())),
    }
}

fn parse_attach_database(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut path = String::new();
    let mut alias = String::new();
    let mut options = HashMap::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::string => path = unescape_string(inner.as_str()),
            Rule::identifier => alias = inner.as_str().to_string(),
            Rule::attach_options => {
                for opt in inner.into_inner() {
                    let mut key = String::new();
                    let mut val = String::new();
                    for part in opt.into_inner() {
                        match part.as_rule() {
                            Rule::identifier => key = part.as_str().to_string(),
                            Rule::literal => val = parse_literal_value(part),
                            _ => {}
                        }
                    }
                    options.insert(key, val);
                }
            }
            _ => {}
        }
    }
    if path.is_empty() || alias.is_empty() {
        return Err("ATTACH requires a path and an alias".into());
    }
    Ok(Statement::AttachDatabase(AttachDatabase { path, alias, options }))
}

fn parse_detach_database(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let alias = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::identifier)
        .map(|p| p.as_str().to_string())
        .ok_or("DETACH requires an alias")?;
    Ok(Statement::DetachDatabase(DetachDatabase { alias }))
}

fn parse_use_database(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let alias = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::identifier)
        .map(|p| p.as_str().to_string())
        .ok_or("USE requires a database alias")?;
    Ok(Statement::UseDatabase(UseDatabase { alias }))
}

fn parse_load_from(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut path = String::new();
    let mut options = HashMap::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::string => path = unescape_string(inner.as_str()),
            Rule::load_options => {
                for opt in inner.into_inner() {
                    let mut key = String::new();
                    let mut val = String::new();
                    for part in opt.into_inner() {
                        match part.as_rule() {
                            Rule::identifier => key = part.as_str().to_string(),
                            Rule::literal => val = parse_literal_value(part),
                            _ => {}
                        }
                    }
                    options.insert(key, val);
                }
            }
            _ => {}
        }
    }
    if path.is_empty() {
        return Err("LOAD FROM requires a file path".into());
    }
    Ok(Statement::LoadFrom(LoadFrom { path, options }))
}

fn parse_literal_value(pair: pest::iterators::Pair<Rule>) -> String {
    let text = pair.as_str().to_string();
    if let Some(inner) = pair.into_inner().next() {
        return match inner.as_rule() {
            Rule::string => unescape_string(inner.as_str()),
            _ => inner.as_str().to_string(),
        }
    }
    text
}

fn parse_create_type(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut name = String::new();
    let mut type_name = String::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier if name.is_empty() => name = inner.as_str().to_string(),
            Rule::type_name if type_name.is_empty() => type_name = inner.as_str().to_string(),
            _ => {}
        }
    }
    if name.is_empty() {
        return Err("CREATE TYPE requires a name".into());
    }
    if type_name.is_empty() {
        return Err("CREATE TYPE requires a type".into());
    }
    Ok(Statement::CreateType(CreateType { name, type_name }))
}

fn parse_comment_on_table(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut table_name = String::new();
    let mut comment = String::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier if table_name.is_empty() => table_name = inner.as_str().to_string(),
            Rule::string => comment = unescape_string(inner.as_str()),
            _ => {}
        }
    }
    if table_name.is_empty() {
        return Err("COMMENT ON requires a table name".into());
    }
    Ok(Statement::CommentOnTable(CommentOnTable { table_name, comment }))
}

fn parse_create_graph(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut name = String::new();
    let mut is_any = false;
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier => name = inner.as_str().to_string(),
            _ => {
                if inner.as_str().to_uppercase() == "ANY" {
                    is_any = true;
                }
            }
        }
    }
    if name.is_empty() {
        return Err("CREATE GRAPH requires a name".into());
    }
    Ok(Statement::CreateGraph(CreateGraph { name, is_any }))
}

fn parse_use_graph(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut name = String::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::identifier {
            name = inner.as_str().to_string();
        }
    }
    if name.is_empty() {
        return Err("USE GRAPH requires a name".into());
    }
    Ok(Statement::UseGraph(UseGraph { name }))
}

fn parse_drop_graph(pair: pest::iterators::Pair<Rule>) -> Result<Statement, String> {
    let mut name = String::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::identifier {
            name = inner.as_str().to_string();
        }
    }
    if name.is_empty() {
        return Err("DROP GRAPH requires a name".into());
    }
    Ok(Statement::DropGraph(DropGraph { name }))
}
