use super::Connection;
use crate::query_result::QueryResult;
use kuzu_binder::bound_statement::{BoundExportDatabase, BoundImportDatabase};

impl Connection {
    pub(crate) fn execute_export_database(
        &self,
        e: &BoundExportDatabase,
    ) -> Result<Option<QueryResult>, String> {
        use std::fs;
        use std::path::Path;

        let dir = Path::new(&e.file_path);
        fs::create_dir_all(dir).map_err(|err| format!("Cannot create export directory '{}': {err}", e.file_path))?;

        let catalog = self.database.catalog.lock().unwrap();

        // Generate schema.cypher
        let mut schema = String::new();
        for entry in catalog.all_entries() {
            match entry {
                kuzu_catalog::CatalogEntry::NodeTable(t) => {
                    let cols: Vec<String> = t
                        .columns
                        .iter()
                        .map(|c| format!("  {} {:?}", c.name, c.logical_type))
                        .collect();
                    let pk = t
                        .columns
                        .get(t.primary_key_column)
                        .map(|c| format!("PRIMARY KEY ({})", c.name))
                        .unwrap_or_default();
                    schema.push_str(&format!("CREATE NODE TABLE {} (\n{}\n);\n\n", t.name, cols.join(",\n")));
                    if !pk.is_empty() {
                        let last_comma = schema.rfind(',').unwrap_or(schema.len() - 2);
                        schema.replace_range(last_comma..last_comma + 1, &format!("\n  {pk},"));
                    }
                }
                kuzu_catalog::CatalogEntry::RelTable(t) => {
                    let cols: Vec<String> = t
                        .columns
                        .iter()
                        .map(|c| format!("  {} {:?}", c.name, c.logical_type))
                        .collect();
                    schema.push_str(&format!("CREATE REL TABLE {} (\n{}\n);\n\n", t.name, cols.join(",\n")));
                }
                _ => {}
            }
        }

        fs::write(dir.join("schema.cypher"), &schema).map_err(|err| format!("Cannot write schema.cypher: {err}"))?;

        // Generate copy.cypher (data export)
        if !e.schema_only {
            let mut copy = String::new();
            for entry in catalog.all_entries() {
                let name = match entry {
                    kuzu_catalog::CatalogEntry::NodeTable(t) => Some(t.name.as_str()),
                    kuzu_catalog::CatalogEntry::RelTable(t) => Some(t.name.as_str()),
                    _ => None,
                };
                if let Some(table_name) = name {
                    let ext = if e.file_type == "parquet" { "parquet" } else { "csv" };
                    let file_name = format!("{}.{}", table_name, ext);
                    copy.push_str(&format!("COPY {} FROM '{}';\n", table_name, file_name));
                }
            }
            fs::write(dir.join("copy.cypher"), &copy).map_err(|err| format!("Cannot write copy.cypher: {err}"))?;
        }

        tracing::info!("Exported database to '{}'", e.file_path);
        Ok(Some(QueryResult::success_message(format!(
            "Database exported to '{}'",
            e.file_path
        ))))
    }

    pub(crate) fn execute_import_database(
        &self,
        i: &BoundImportDatabase,
    ) -> Result<Option<QueryResult>, String> {
        // Execute schema DDL first, then COPY FROM, then indexes
        if !i.query.is_empty() {
            for line in i.query.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with("//") {
                    continue;
                }
                if let Err(e) = self.query(trimmed) {
                    tracing::warn!("Import statement skipped (may be duplicate): {e}");
                }
            }
        }

        if !i.index_query.is_empty() {
            for line in i.index_query.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with("//") {
                    continue;
                }
                if let Err(e) = self.query(trimmed) {
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
