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

    /// Export query results to a CSV file (CALL export_csv wrapper).
    pub(crate) fn export_to_csv(&self, path: &str, query_str: &str) -> Result<(), String> {
        // Execute the inner query
        let result = self.query(query_str)?;

        let mut w = csv::WriterBuilder::new()
            .has_headers(true)
            .from_path(path)
            .map_err(|e| format!("Cannot create file '{path}': {e}"))?;

        // Write header from column names
        if let Some(first_chunk) = result.chunks.first() {
            let header: Vec<String> = if first_chunk.field_names.is_empty() {
                (0..first_chunk.num_fields()).map(|i| format!("column_{i}")).collect()
            } else {
                first_chunk.field_names.clone()
            };
            if !header.is_empty() {
                w.write_record(&header)
                    .map_err(|e| format!("CSV write error: {e}"))?;
            }
        }

        // Write rows
        for chunk in &result.chunks {
            for row in 0..chunk.size {
                let row_values: Vec<String> = chunk
                    .fields
                    .iter()
                    .map(|f| {
                        f.get_value(row)
                            .map(|v| super::utils::value_to_csv_string(&v))
                            .unwrap_or_default()
                    })
                    .collect();
                w.write_record(&row_values)
                    .map_err(|e| format!("CSV write error: {e}"))?;
            }
        }
        w.flush().map_err(|e| format!("CSV flush error: {e}"))?;
        Ok(())
    }

    /// Export query results to a Parquet file (CALL export_parquet wrapper).
    #[allow(unused_variables)]
    pub(crate) fn export_to_parquet(&self, path: &str, query_str: &str) -> Result<(), String> {
        #[cfg(feature = "parquet-export")]
        {
            let result = self.query(query_str)?;

            use arrow::array::StringArray;
            use arrow::datatypes::{DataType, Field, Schema};
            use parquet::arrow::ArrowWriter;
            use std::sync::Arc;

            if result.chunks.is_empty() {
                return Err("Query returned no results".into());
            }

            // Build schema from first chunk
            let first = &result.chunks[0];
            let fields: Vec<Field> = (0..first.num_fields())
                .map(|i| {
                    let name = first
                        .field_names
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| format!("column_{i}"));
                    Field::new(&name, DataType::Utf8, true)
                })
                .collect();

            let schema = Arc::new(Schema::new(fields));

            let file = std::fs::File::create(path)
                .map_err(|e| format!("Cannot create file '{path}': {e}"))?;
            let mut writer = ArrowWriter::try_new(file, schema.clone(), None)
                .map_err(|e| format!("Parquet writer error: {e}"))?;

            for chunk in &result.chunks {
                let num_cols = chunk.num_fields();
                let num_rows = chunk.size;
                let mut columns: Vec<StringArray> = Vec::new();

                for col in 0..num_cols {
                    let values: Vec<String> = (0..num_rows)
                        .map(|row| {
                            chunk
                                .fields
                                .get(col)
                                .and_then(|f| f.get_value(row))
                                .map(|v| super::utils::value_to_csv_string(&v))
                                .unwrap_or_default()
                        })
                        .collect();
                    columns.push(StringArray::from(values));
                }

                let batch = arrow::record_batch::RecordBatch::try_new(
                    schema.clone(),
                    columns.iter().map(|c| Arc::new(c.clone()) as Arc<dyn arrow::array::Array>).collect(),
                )
                .map_err(|e| format!("RecordBatch error: {e}"))?;

                writer.write(&batch).map_err(|e| format!("Parquet write error: {e}"))?;
            }

            writer.close().map_err(|e| format!("Parquet close error: {e}"))?;
            Ok(())
        }
        #[cfg(not(feature = "parquet-export"))]
        {
            Err(
                "Parquet export requires 'parquet-export' feature. \
                 Build with: cargo build --features parquet-export"
                    .into(),
            )
        }
    }
}
