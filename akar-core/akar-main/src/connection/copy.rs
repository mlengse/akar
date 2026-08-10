use super::Connection;
use crate::query_result::QueryResult;
use akar_binder::bound_statement::{BoundExportDatabase, BoundImportDatabase};
use akar_catalog::Catalog;
use akar_common::types::LogicalTypeID;

impl Connection {
    pub(crate) fn execute_export_database(&self, e: &BoundExportDatabase) -> Result<Option<QueryResult>, String> {
        use std::fs;
        use std::path::Path;

        let dir = Path::new(&e.file_path);
        fs::create_dir_all(dir).map_err(|err| format!("Cannot create export directory '{}': {err}", e.file_path))?;

        let catalog = self
            .database
            .catalog
            .lock()
            .map_err(|e| format!("Lock poisoned: {e}"))?;

        // Generate schema.cypher
        let schema = generate_schema_cypher(&catalog);
        fs::write(dir.join("schema.cypher"), &schema).map_err(|err| format!("Cannot write schema.cypher: {err}"))?;

        // Generate copy.cypher (data export)
        if !e.schema_only {
            let copy = generate_copy_cypher(&catalog, &e.file_type);
            fs::write(dir.join("copy.cypher"), &copy).map_err(|err| format!("Cannot write copy.cypher: {err}"))?;
        }

        tracing::info!("Exported database to '{}'", e.file_path);
        Ok(Some(QueryResult::success_message(format!(
            "Database exported to '{}'",
            e.file_path
        ))))
    }

    pub(crate) fn execute_import_database(&self, i: &BoundImportDatabase) -> Result<Option<QueryResult>, String> {
        // Execute schema DDL first, then COPY FROM, then indexes.
        // The exporter writes multi-line DDL (`CREATE NODE TABLE ... (\n...\n);`),
        // so statements must be split on `;` — never on newlines (P52.12).
        if !i.query.is_empty() {
            for stmt in split_cypher_statements(&i.query) {
                if let Err(e) = self.query(&stmt) {
                    tracing::warn!("Import statement skipped (may be duplicate): {e}");
                }
            }
        }

        if !i.index_query.is_empty() {
            for stmt in split_cypher_statements(&i.index_query) {
                if let Err(e) = self.query(&stmt) {
                    tracing::warn!("Index statement skipped: {e}");
                }
            }
        }

        tracing::info!("Imported database from '{}'", i.file_path);
        Ok(Some(QueryResult::success_message(format!(
            "Database imported from '{}'",
            i.file_path
        ))))
    }
}

/// Split a Cypher script into individual statements on `;` delimiters.
///
/// Semicolons inside single-quoted string literals are not treated as
/// delimiters, so `COPY T FROM 'a;b.csv';` splits correctly. This lets the
/// IMPORT DATABASE round-trip the multi-line `CREATE NODE TABLE ... (\n...\n);`
/// DDL that the exporter writes (P52.12).
pub(crate) fn split_cypher_statements(script: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_single = false;

    for ch in script.chars() {
        match ch {
            '\'' => {
                in_single = !in_single;
                current.push(ch);
            }
            ';' if !in_single => {
                if let Some(stmt) = clean_statement(&current) {
                    statements.push(stmt);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    if let Some(stmt) = clean_statement(&current) {
        statements.push(stmt);
    }

    statements
}

/// Trim a raw statement chunk, dropping leading blank lines and `//` comment
/// lines so a script header comment does not get glued onto the first
/// statement (P52.12 regression).
fn clean_statement(raw: &str) -> Option<String> {
    let mut significant = None;
    for (i, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        significant = Some(i);
        break;
    }

    let sig = significant?;
    let stmt = raw.lines().skip(sig).collect::<Vec<_>>().join("\n");
    let stmt = stmt.trim();
    if stmt.is_empty() {
        None
    } else {
        Some(stmt.to_string())
    }
}

/// Map a catalog logical type to a grammar-valid Cypher DDL type name.
///
/// `{:?}` (Debug) yields variant names like `UInt64` that the DDL grammar
/// rejects — the exporter must emit canonical `UINT64`/`INT32`/... keywords so
/// the schema round-trips through IMPORT DATABASE.
fn logical_type_name(t: LogicalTypeID) -> String {
    let name = match t {
        LogicalTypeID::Bool => "BOOL",
        LogicalTypeID::Int64 => "INT64",
        LogicalTypeID::Int32 => "INT32",
        LogicalTypeID::Int16 => "INT16",
        LogicalTypeID::Int8 => "INT8",
        LogicalTypeID::UInt64 => "UINT64",
        LogicalTypeID::UInt32 => "UINT32",
        LogicalTypeID::UInt16 => "UINT16",
        LogicalTypeID::UInt8 => "UINT8",
        LogicalTypeID::Double => "DOUBLE",
        LogicalTypeID::Float => "FLOAT",
        LogicalTypeID::String => "STRING",
        LogicalTypeID::Blob => "BLOB",
        LogicalTypeID::Date => "DATE",
        LogicalTypeID::Timestamp => "TIMESTAMP",
        LogicalTypeID::TimestampSec => "TIMESTAMP_SEC",
        LogicalTypeID::TimestampMs => "TIMESTAMP_MS",
        LogicalTypeID::TimestampNs => "TIMESTAMP_NS",
        LogicalTypeID::TimestampTz => "TIMESTAMP_TZ",
        LogicalTypeID::Interval => "INTERVAL",
        LogicalTypeID::Serial => "SERIAL",
        LogicalTypeID::UInt128 => "UINT128",
        LogicalTypeID::Json => "JSON",
        LogicalTypeID::Time => "TIME",
        // Compound/nested types carry no child-type metadata in the catalog,
        // so they cannot round-trip; fall back to the Debug name (unchanged
        // pre-fix behavior for these edge cases).
        other => return format!("{other:?}"),
    };
    name.to_string()
}

/// Build the `schema.cypher` file content for a database export.
///
/// Emits grammar-valid DDL for every node and rel table (P52.13/P52.26):
/// - node tables: `CREATE NODE TABLE t (col T, ..., PRIMARY KEY (pk));`
///   (PK is written inside the column list, so single-column tables are valid)
/// - rel tables: `CREATE REL TABLE t (FROM src TO dst, col T, ...);`
///   (endpoint node tables resolved by ID, required by the import grammar)
pub(crate) fn generate_schema_cypher(catalog: &Catalog) -> String {
    let mut schema = String::new();
    for entry in catalog.all_entries() {
        match entry {
            akar_catalog::CatalogEntry::NodeTable(t) => {
                let mut cols: Vec<String> = t
                    .columns
                    .iter()
                    .map(|c| format!("  {} {}", c.name, logical_type_name(c.logical_type)))
                    .collect();
                if let Some(pk) = t.columns.get(t.primary_key_column) {
                    cols.push(format!("  PRIMARY KEY ({})", pk.name));
                }
                schema.push_str(&format!("CREATE NODE TABLE {} (\n{}\n);\n\n", t.name, cols.join(",\n")));
            }
            akar_catalog::CatalogEntry::RelTable(t) => {
                let src = catalog
                    .get_entry(t.src_table_id)
                    .map(|e| e.name().to_string())
                    .unwrap_or_else(|| t.src_table_id.to_string());
                let dst = catalog
                    .get_entry(t.dst_table_id)
                    .map(|e| e.name().to_string())
                    .unwrap_or_else(|| t.dst_table_id.to_string());
                let mut ddl = format!("CREATE REL TABLE {} (FROM {} TO {}", t.name, src, dst);
                if !t.columns.is_empty() {
                    let cols: Vec<String> = t
                        .columns
                        .iter()
                        .map(|c| format!("{} {}", c.name, logical_type_name(c.logical_type)))
                        .collect();
                    ddl.push_str(&format!(", {}", cols.join(", ")));
                }
                ddl.push_str(");\n\n");
                schema.push_str(&ddl);
            }
            _ => {}
        }
    }
    schema
}

/// Build the `copy.cypher` file content for a database export.
pub(crate) fn generate_copy_cypher(catalog: &Catalog, file_type: &str) -> String {
    let mut copy = String::new();
    for entry in catalog.all_entries() {
        let name = match entry {
            akar_catalog::CatalogEntry::NodeTable(t) => Some(t.name.as_str()),
            akar_catalog::CatalogEntry::RelTable(t) => Some(t.name.as_str()),
            _ => None,
        };
        if let Some(table_name) = name {
            let ext = if file_type == "parquet" { "parquet" } else { "csv" };
            let file_name = format!("{}.{}", table_name, ext);
            copy.push_str(&format!("COPY {} FROM '{}';\n", table_name, file_name));
        }
    }
    copy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_cypher_statements_basic() {
        let stmts = split_cypher_statements("CREATE NODE TABLE A (x INT64);\nCOPY A FROM 'a.csv';\n");
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0], "CREATE NODE TABLE A (x INT64)");
        assert_eq!(stmts[1], "COPY A FROM 'a.csv'");
    }

    #[test]
    fn test_split_cypher_statements_multiline_ddl() {
        // The exporter writes multi-line DDL — must round-trip as one statement.
        let script = "CREATE NODE TABLE A (\n  name STRING,\n  age INT64,\n  PRIMARY KEY (name)\n);\n\nCOPY A FROM 'a.csv';\n";
        let stmts = split_cypher_statements(script);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].starts_with("CREATE NODE TABLE A"));
        assert!(stmts[0].contains("PRIMARY KEY (name)"));
        assert_eq!(stmts[1], "COPY A FROM 'a.csv'");
    }

    #[test]
    fn test_split_cypher_statements_semicolon_in_string() {
        // A semicolon inside a quoted string literal must not split a statement.
        let stmts = split_cypher_statements("COPY A FROM 'a;b.csv'; COPY B FROM 'c.csv';");
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0], "COPY A FROM 'a;b.csv'");
        assert_eq!(stmts[1], "COPY B FROM 'c.csv'");
    }

    #[test]
    fn test_split_cypher_statements_skips_comments_and_blank() {
        let stmts = split_cypher_statements("// header comment\n\nCREATE NODE TABLE A (x INT64);\n");
        assert_eq!(stmts.len(), 1);
        assert!(stmts[0].starts_with("CREATE NODE TABLE A"));
    }

    #[test]
    fn test_split_cypher_statements_comments_between_statements() {
        let stmts =
            split_cypher_statements("CREATE NODE TABLE A (x INT64);\n// mid comment\n\nCOPY A FROM 'a.csv';\n");
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0], "CREATE NODE TABLE A (x INT64)");
        assert_eq!(stmts[1], "COPY A FROM 'a.csv'");
    }

    #[test]
    fn test_logical_type_names_are_grammar_valid() {
        assert_eq!(logical_type_name(LogicalTypeID::String), "STRING");
        assert_eq!(logical_type_name(LogicalTypeID::UInt64), "UINT64");
        assert_eq!(logical_type_name(LogicalTypeID::Int32), "INT32");
        assert_eq!(logical_type_name(LogicalTypeID::Double), "DOUBLE");
        assert_eq!(logical_type_name(LogicalTypeID::Serial), "SERIAL");
        assert_eq!(logical_type_name(LogicalTypeID::Timestamp), "TIMESTAMP");
    }
}
