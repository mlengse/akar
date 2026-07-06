//! Parser implementation — converts Cypher query text to AST using pest.rs PEG grammar.
//!
//! This module has been modularized. The actual parsing functions are in:
//! - `parser/ddl.rs` — DDL statements (CREATE/DROP TABLE, INDEX, SEQUENCE, ALTER, COPY, etc.)
//! - `parser/dml.rs` — DML statements (MATCH, RETURN, WHERE, CREATE, DELETE, SET, MERGE, FOREACH, etc.)
//! - `parser/expression.rs` — Expression parsing

use crate::ast::*;
use pest::Parser;

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
            ddl::parse_ddl(ddl_inner)
        }
        Rule::create_dml_statement => {
            let inner_clone = inner.clone();
            let patterns = dml::parse_patterns(inner_clone)?;
            Ok(Statement::CreateDml(CreateClause { patterns }))
        }
        Rule::query_statement => {
            let inner_clone = inner.clone();
            let child_rules: Vec<_> = inner_clone.into_inner().map(|c| c.as_rule()).collect();
            if child_rules.contains(&Rule::merge_clause) {
                let merge = dml::parse_merge(inner)?;
                Ok(merge)
            } else {
                let query = dml::parse_query_pairs(inner)?;
                Ok(Statement::Query(query))
            }
        }
        Rule::union_statement => {
            let inner_clone = inner.clone();
            ddl::parse_ddl(inner_clone)
        }
        Rule::call_statement => {
            let call = dml::parse_call(inner)?;
            Ok(Statement::Call(call))
        }
        Rule::export_database => ddl::parse_export_database(inner),
        Rule::import_database => ddl::parse_import_database(inner),
        Rule::analyze_statement => ddl::parse_analyze(inner),
        _ => Err(format!("Unexpected rule: {:?}", inner.as_rule())),
    }
}

mod ddl;
mod dml;
pub mod expression;
