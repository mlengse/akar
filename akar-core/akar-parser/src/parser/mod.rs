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

    let mut pairs = CypherParser::parse(Rule::akar_query, trimmed).map_err(|e| format!("Parse error: {e}"))?;
    let akar_query_pair = pairs.next().ok_or("Empty input")?;
    let statement_pair = akar_query_pair.into_inner().next().ok_or("No statement in query")?;
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
            let children: Vec<_> = inner_clone.into_inner().collect();
            // A standalone MERGE (possibly with RETURN) keeps the legacy
            // `Statement::Merge` shape. A MERGE embedded in a clause chain becomes
            // `Clause::Merge` inside a Query.
            let standalone_merge = children.len() <= 2
                && children.first().is_some_and(|c| c.as_rule() == Rule::query_clause)
                && (children.len() == 1 || children[1].as_rule() == Rule::return_clause)
                && {
                    let qc_inner: Vec<_> = children[0].clone().into_inner().collect();
                    qc_inner.len() == 1 && qc_inner[0].as_rule() == Rule::merge_clause
                };
            if standalone_merge {
                let qc = inner.into_inner().next().ok_or("Empty MERGE query")?;
                let merge_pair = qc.into_inner().next().ok_or("MERGE missing merge_clause")?;
                Ok(Statement::Merge(dml::parse_merge_clause(merge_pair)?))
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
            Ok(Statement::StandaloneCall(call))
        }
        Rule::export_database => ddl::parse_export_database(inner),
        Rule::import_database => ddl::parse_import_database(inner),
        Rule::analyze_statement => ddl::parse_analyze(inner),
        Rule::transaction_statement => ddl::parse_transaction(inner),
        Rule::extension_statement => ddl::parse_extension(inner),
        Rule::multi_db_statement => {
            let db_inner = inner.into_inner().next().ok_or("Empty multi-DB statement")?;
            ddl::parse_multi_db(db_inner)
        }
        _ => Err(format!("Unexpected rule: {:?}", inner.as_rule())),
    }
}

mod ddl;
mod dml;
pub mod expression;
