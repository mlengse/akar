use super::Connection;
use crate::query_result::QueryResult;
use akar_binder::bound_statement::{BoundExportDatabase, BoundImportDatabase};
use akar_catalog::Catalog;
use akar_common::types::LogicalTypeID;
use akar_parser::ast::CopyToFormat;

impl Connection {
    pub(crate) fn execute_export_database(&self, e: &BoundExportDatabase) -> Result<Option<QueryResult>, String> {
        use std::fs;
        use std::path::Path;

        let dir = Path::new(&e.file_path);
        fs::create_dir_all(dir).map_err(|err| format!("Cannot create export directory '{}': {err}", e.file_path))?;

        // Get catalog entries needed for schema and data export, then release the lock
        let entries: Vec<_> = {
            let catalog = self
                .database
                .catalog
                .lock()
                .map_err(|e| format!("Lock poisoned: {e}"))?;
            catalog.all_entries().cloned().collect()
        };

        // Generate schema.cypher from entries
        let schema = generate_schema_cypher_from_entries(&entries);
        fs::write(dir.join("schema.cypher"), &schema).map_err(|err| format!("Cannot write schema.cypher: {err}"))?;

        // Generate copy.cypher (data export) AND write actual data files
        if !e.schema_only {
            let copy = generate_copy_cypher_from_entries(&entries, &e.file_type, Some(dir));
            fs::write(dir.join("copy.cypher"), &copy).map_err(|err| format!("Cannot write copy.cypher: {err}"))?;

            // Write actual data files for each table (catalog lock released)
            export_table_data(self, &entries, &e.file_type, dir)?;
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
    if stmt.is_empty() { None } else { Some(stmt.to_string()) }
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
        // so they cannot round-trip; fall back to a parser-accepted default.
        // `List` → `FLOAT[]` (parser accepts `primitive_type ~ "[]"*`).
        LogicalTypeID::List => "FLOAT[]",
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
    let entries: Vec<_> = catalog.all_entries().cloned().collect();
    generate_schema_cypher_from_entries(&entries)
}

/// Build the `schema.cypher` file content from a list of catalog entries.
fn generate_schema_cypher_from_entries(entries: &[akar_catalog::CatalogEntry]) -> String {
    let mut schema = String::new();
    for entry in entries {
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
                let src = entries
                    .iter()
                    .find(|e| e.table_id() == t.src_table_id)
                    .map(|e| e.name().to_string())
                    .unwrap_or_else(|| t.src_table_id.to_string());
                let dst = entries
                    .iter()
                    .find(|e| e.table_id() == t.dst_table_id)
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
    let entries: Vec<_> = catalog.all_entries().cloned().collect();
    generate_copy_cypher_from_entries(&entries, file_type, None)
}

/// Build the `copy.cypher` file content from a list of catalog entries.
///
/// When `dir` is provided, COPY FROM paths are written as absolute paths
/// rooted at that directory so that IMPORT DATABASE can resolve them
/// regardless of the working directory (P53.34b).
fn generate_copy_cypher_from_entries(
    entries: &[akar_catalog::CatalogEntry],
    file_type: &str,
    dir: Option<&std::path::Path>,
) -> String {
    let mut copy = String::new();
    for entry in entries {
        // Rel tables with only endpoints (metadata relations like HAS_TABLE)
        // have no exportable property columns — skip their COPY FROM line so
        // IMPORT doesn't reference a data file that wasn't written (P53.37).
        let skip_data = match entry {
            akar_catalog::CatalogEntry::RelTable(t) => t.columns.is_empty(),
            _ => false,
        };
        if skip_data {
            continue;
        }
        let name = match entry {
            akar_catalog::CatalogEntry::NodeTable(t) => Some(t.name.as_str()),
            akar_catalog::CatalogEntry::RelTable(t) => Some(t.name.as_str()),
            _ => None,
        };
        if let Some(table_name) = name {
            let ext = if file_type == "parquet" { "parquet" } else { "csv" };
            let file_name = format!("{}.{}", table_name, ext);
            let path = match dir {
                Some(d) => d.join(&file_name).to_string_lossy().replace('\\', "/"),
                None => file_name,
            };
            copy.push_str(&format!("COPY {} FROM '{}';\n", table_name, path));
        }
    }
    copy
}

/// Export actual table data to CSV or Parquet files.
fn export_table_data(
    conn: &Connection,
    entries: &[akar_catalog::CatalogEntry],
    file_type: &str,
    dir: &std::path::Path,
) -> Result<(), String> {
    use super::utils::value_to_csv_string;

    let format = if file_type == "parquet" {
        CopyToFormat::Parquet
    } else {
        CopyToFormat::Csv
    };

    for entry in entries {
        let (table_name, query, column_names) = match entry {
            akar_catalog::CatalogEntry::NodeTable(t) => {
                let name = t.name.as_str();
                let cols: Vec<String> = t.columns.iter().map(|c| c.name.clone()).collect();
                // Use explicit column list to exclude internal `_id` (P53.34b).
                let return_cols: Vec<String> = cols.iter().map(|c| format!("n.{}", c)).collect();
                let q = format!("MATCH (n:{}) RETURN {}", name, return_cols.join(", "));
                (name, q, cols)
            }
            akar_catalog::CatalogEntry::RelTable(t) => {
                let name = t.name.as_str();
                let cols: Vec<String> = t.columns.iter().map(|c| c.name.clone()).collect();
                if cols.is_empty() {
                    // Rel table with only endpoints — no exportable property
                    // columns; skip data export (P53.37).
                    continue;
                }
                let return_cols: Vec<String> = cols.iter().map(|c| format!("r.{}", c)).collect();
                let q = format!("MATCH ()-[r:{}]->() RETURN {}", name, return_cols.join(", "));
                (name, q, cols)
            }
            _ => continue,
        };

        let ext = if format == CopyToFormat::Parquet {
            "parquet"
        } else {
            "csv"
        };
        let file_path = dir.join(format!("{}.{}", table_name, ext));
        let file_path_str = file_path.to_string_lossy().to_string();

        // Execute query to get all data using the public query API
        let result = conn.query(&query)?;

        // Write to file
        match format {
            CopyToFormat::Csv => {
                let mut w = csv::WriterBuilder::new()
                    .has_headers(true)
                    .from_path(&file_path)
                    .map_err(|e| format!("Cannot create file '{}': {}", file_path_str, e))?;

                // Write header — strip alias prefix (n.id → id) so COPY FROM
                // column names match the catalog column names (P53.34b).
                if let Some(first_chunk) = result.chunks.first() {
                    let header: Vec<String> = if first_chunk.field_names.is_empty() {
                        (0..first_chunk.num_fields()).map(|i| format!("column_{}", i)).collect()
                    } else {
                        first_chunk
                            .field_names
                            .iter()
                            .map(|n| {
                                n.rsplit_once('.')
                                    .map(|(_, base)| base.to_string())
                                    .unwrap_or_else(|| n.clone())
                            })
                            .collect()
                    };
                    if !header.is_empty() {
                        w.write_record(&header).map_err(|e| format!("CSV write error: {e}"))?;
                    }
                } else if !column_names.is_empty() {
                    // Empty table: use catalog column names
                    w.write_record(&column_names)
                        .map_err(|e| format!("CSV write error: {e}"))?;
                }

                // Write rows
                for chunk in &result.chunks {
                    for row in 0..chunk.size {
                        let row_values: Vec<String> = (0..chunk.fields.len())
                            .map(|col_idx| {
                                chunk
                                    .get_value(col_idx, row)
                                    .map(|v| value_to_csv_string(&v))
                                    .unwrap_or_default()
                            })
                            .collect();
                        w.write_record(&row_values)
                            .map_err(|e| format!("CSV write error: {e}"))?;
                    }
                }

                w.flush().map_err(|e| format!("CSV flush error: {e}"))?;
            }
            CopyToFormat::Parquet => {
                #[cfg(feature = "parquet-export")]
                {
                    // Use query result's field_names (works for empty tables too)
                    let final_column_names = result
                        .chunks
                        .first()
                        .map(|chunk| {
                            if chunk.field_names.is_empty() {
                                (0..chunk.fields.len()).map(|i| format!("column_{}", i)).collect()
                            } else {
                                chunk
                                    .field_names
                                    .iter()
                                    .map(|n| {
                                        n.rsplit_once('.')
                                            .map(|(_, base)| base.to_string())
                                            .unwrap_or_else(|| n.clone())
                                    })
                                    .collect()
                            }
                        })
                        .unwrap_or_else(|| {
                            // No chunks at all - use catalog column names as fallback
                            column_names
                        });

                    if final_column_names.is_empty() {
                        // No schema info available, skip this table
                        continue;
                    }

                    // Stream the result chunks column-major into the parquet
                    // writer with proper column names and declared column
                    // types (all-null columns keep their Arrow type, P53.37) —
                    // no row-major materialization (P51.49).
                    let declared_types = result.chunks.first().map(|c| c.field_types.as_slice());

                    akar_storage::parquet_writer::write_parquet_from_chunks(
                        &file_path_str,
                        &result.chunks,
                        Some(&final_column_names),
                        declared_types,
                    )
                    .map_err(|e| format!("Parquet export error for '{}': {}", table_name, e))?;
                }
                #[cfg(not(feature = "parquet-export"))]
                {
                    return Err(format!(
                        "Parquet export for table '{}' requires 'parquet-export' feature. Build with: cargo build --features parquet-export",
                        table_name
                    ));
                }
            }
        }
    }
    Ok(())
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
        let script =
            "CREATE NODE TABLE A (\n  name STRING,\n  age INT64,\n  PRIMARY KEY (name)\n);\n\nCOPY A FROM 'a.csv';\n";
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
        let stmts = split_cypher_statements("CREATE NODE TABLE A (x INT64);\n// mid comment\n\nCOPY A FROM 'a.csv';\n");
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
