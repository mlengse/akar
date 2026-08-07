//! Auto-extracted from physical_operator.rs
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};
use akar_storage::table::{ColumnDefinition, TableCatalog};
use std::path::Path;
use std::sync::Arc;

// ==================== CopyFrom ====================

/// Physical operator for COPY FROM — loads data from CSV/Parquet files into a table.
///
/// Detects file type from extension, calls the appropriate reader,
/// and inserts rows into the target table via the `TableCatalog`.
pub struct PhysicalCopyFrom {
    pub table_name: String,
    pub table_id: u64,
    pub file_path: String,
    pub columns: Vec<ColumnDefinition>,
    pub options: std::collections::HashMap<String, String>,
    pub table_catalog: Arc<TableCatalog>,
    pub vfs: Arc<akar_common::file_system::VirtualFileSystemRegistry>,
}

impl PhysicalOperatorExec for PhysicalCopyFrom {
    fn operator_type(&self) -> &str {
        "copy_from"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let path = Path::new(&self.file_path);

        // 1. Detect file type from extension
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        // 2. Build config and convert column schema.
        //    For rel tables the COPY file carries two leading [from, to] columns
        //    (node PK values) ahead of the user properties. Synthesize those two
        //    columns (typed as the src/dst node PK types) so readers validate
        //    `columns.len() + 2` and the insert branch can resolve PKs -> offsets.
        let mut catalog_cols: Vec<akar_catalog::CatalogColumn> = self
            .columns
            .iter()
            .map(|c| akar_catalog::CatalogColumn {
                name: c.name.clone(),
                logical_type: c.logical_type,
                is_primary_key: c.is_primary_key,
                compression: akar_common::enums::CompressionType::Uncompressed,
                default_value: None,
            })
            .collect();
        {
            let rel_meta = self.table_catalog.get_rel_table_by_name(&self.table_name);
            if let Some(rel) = &rel_meta {
                let src_pk_type = self
                    .table_catalog
                    .get_node_table(rel.src_table_id)
                    .and_then(|n| n.columns.get(n.primary_key_column).map(|c| c.logical_type))
                    .unwrap_or(akar_common::types::LogicalTypeID::Int64);
                let dst_pk_type = self
                    .table_catalog
                    .get_node_table(rel.dst_table_id)
                    .and_then(|n| n.columns.get(n.primary_key_column).map(|c| c.logical_type))
                    .unwrap_or(akar_common::types::LogicalTypeID::Int64);
                let synthetic = |name: &str, logical_type: akar_common::types::LogicalTypeID| {
                    akar_catalog::CatalogColumn {
                        name: name.to_string(),
                        logical_type,
                        is_primary_key: false,
                        compression: akar_common::enums::CompressionType::Uncompressed,
                        default_value: None,
                    }
                };
                catalog_cols.insert(0, synthetic("from", src_pk_type));
                catalog_cols.insert(1, synthetic("to", dst_pk_type));
            }
        }

        // 3. Read the file
        let rows = match ext.as_str() {
            "csv" | "tsv" => {
                let mut config = akar_storage::csv_reader::CsvReaderConfig::from_options(&self.options);
                if ext == "tsv" && !self.options.contains_key("DELIM") && !self.options.contains_key("delim") {
                    config.delimiter = b'\t';
                }

                akar_storage::csv_reader::read_csv(&self.file_path, &self.vfs, &catalog_cols, &config)
                    .map_err(|e| format!("CSV read error: {e}"))?
            }
            #[cfg(feature = "parquet")]
            "parquet" => akar_storage::parquet_reader::read_parquet(&self.file_path, &self.vfs, &catalog_cols)
                .map_err(|e| format!("Parquet read error: {e}"))?,
            #[cfg(not(feature = "parquet"))]
            "parquet" => return Err("Parquet support not enabled (feature 'parquet' in akar-storage)".into()),
            _ => {
                return Err(format!(
                    "Unsupported file type: .{ext} (supported: .csv, .tsv, .parquet)"
                ).into());
            }
        };

        // 4. Insert rows into the table using batch insert
        let num_rows = rows.len();
        if num_rows == 0 {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            return Ok(vec![DataChunk::new(
                vec![akar_common::arrow_vector::ArrowVector::from_legacy(&v).array],
                vec![akar_common::types::PhysicalTypeID::Int64],
            )]);
        }

        if let Some(mut table) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
            let count = table
                .insert_rows_batch(&rows)
                .map_err(|e| format!("Batch insert error: {e}"))?;
            tracing::info!(
                "COPY FROM: batch-inserted {count} rows into node table '{}'",
                self.table_name
            );
        } else if let Some(mut table) = self.table_catalog.get_rel_table_by_name_mut(&self.table_name) {
            // Rel COPY files carry [from, to, ...props] where from/to are node PK
            // values. Resolve them to internal node offsets via the src/dst node
            // tables' PK index (mirrors C++ IndexLookupInfo).
            let src_node = self.table_catalog.get_node_table(table.src_table_id);
            let dst_node = self.table_catalog.get_node_table(table.dst_table_id);
            let mut rels: Vec<(u64, u64, Vec<Value>)> = Vec::with_capacity(rows.len());
            for row in &rows {
                let from = src_node
                    .as_ref()
                    .and_then(|n| n.lookup_by_pk(&row[0]))
                    .ok_or_else(|| {
                        format!(
                            "COPY rel: source node with PK {:?} not found in table '{}'",
                            row[0], self.table_name
                        )
                    })?;
                let to = dst_node
                    .as_ref()
                    .and_then(|n| n.lookup_by_pk(&row[1]))
                    .ok_or_else(|| {
                        format!(
                            "COPY rel: destination node with PK {:?} not found in table '{}'",
                            row[1], self.table_name
                        )
                    })?;
                rels.push((from, to, row[2..].to_vec()));
            }
            let count = table
                .insert_rels_batch(&rels)
                .map_err(|e| format!("Batch insert rel error: {e}"))?;
            tracing::info!(
                "COPY FROM: batch-inserted {count} rows into rel table '{}'",
                self.table_name
            );
        } else {
            return Err(format!("Table '{}' not found in storage catalog", self.table_name).into());
        }

        // Return success chunk with row count
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, num_rows as i64);
        Ok(vec![DataChunk::new(
            vec![akar_common::arrow_vector::ArrowVector::from_legacy(&v).array],
            vec![akar_common::types::PhysicalTypeID::Int64],
        )])
    }
}

/// Physical operator for ART index range scans.
///
/// Uses the ART index on a node table's PK column to efficiently find rows
/// within a key range, then fetches the full column data for those rows.
///
/// Pattern follows `PhysicalVectorSimilarityScan`.
#[derive(Debug, Clone)]
pub struct PhysicalArtIndexRangeScan {
    pub table_name: String,
    pub table_id: u64,
    pub lower_bound: Option<Value>,
    pub upper_bound: Option<Value>,
    pub lower_inclusive: bool,
    pub upper_inclusive: bool,
    pub table_catalog: Option<Arc<TableCatalog>>,
}

impl PhysicalOperatorExec for PhysicalArtIndexRangeScan {
    fn operator_type(&self) -> &str {
        "art_index_range_scan"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let tc = self
            .table_catalog
            .clone()
            .ok_or_else(|| "No table catalog available for ArtIndexRangeScan".to_string())?;

        let node_table = tc
            .get_node_table_by_name(&self.table_name)
            .ok_or_else(|| format!("Node table '{}' not found", self.table_name))?;

        // Verify ART index exists
        if node_table.art_index.is_none() {
            return Err(format!("Table '{}' does not have an ART index", self.table_name).into());
        }

        // Execute range scan on the ART index
        let row_ids = node_table.lookup_by_pk_range(
            self.lower_bound.as_ref(),
            self.lower_inclusive,
            self.upper_bound.as_ref(),
            self.upper_inclusive,
            u64::MAX,
        );
        drop(node_table); // Release table ref before cloning data

        if row_ids.is_empty() {
            return Ok(vec![DataChunk::new(vec![], vec![])]);
        }

        // Fetch column values for matched row IDs
        let node_table = tc
            .get_node_table_by_name(&self.table_name)
            .ok_or_else(|| format!("Node table '{}' not found", self.table_name))?;

        let num_cols = node_table.columns.len();
        let num_results = row_ids.len();

        let mut output_columns: Vec<Vec<Value>> = vec![Vec::with_capacity(num_results); num_cols];

        for &row_id in &row_ids {
            for (col_idx, out_col) in output_columns.iter_mut().enumerate().take(num_cols) {
                match node_table.get_value(row_id as usize, col_idx) {
                    Some(val) => out_col.push(val.clone()),
                    None => out_col.push(Value::Null),
                }
            }
        }

        let mut col_types = Vec::with_capacity(num_cols);
        let mut col_names: Vec<String> = Vec::with_capacity(num_cols);
        for col in &node_table.columns {
            col_types.push(col.logical_type);
            col_names.push(col.name.clone());
        }

        drop(node_table);

        // Convert column-major Vec<Vec<Value>> to DataChunks
        use akar_common::types::PhysicalTypeID;
        use akar_common::vector::{DataChunk, ValueVector};

        let num_rows = output_columns.first().map(|c| c.len()).unwrap_or(0);
        if num_rows == 0 {
            return Ok(vec![DataChunk::new(vec![], vec![])]);
        }

        let mut chunks = Vec::new();
        let chunk_size = 1024usize;
        for start in (0..num_rows).step_by(chunk_size) {
            let end = (start + chunk_size).min(num_rows);
            let count = end - start;
            let mut fields = Vec::with_capacity(num_cols);

            for col_idx in 0..num_cols {
                let col_data = &output_columns[col_idx];
                let phys_type = match col_types[col_idx] {
                    akar_common::types::LogicalTypeID::Bool => PhysicalTypeID::Bool,
                    akar_common::types::LogicalTypeID::Int64 | akar_common::types::LogicalTypeID::Serial => {
                        PhysicalTypeID::Int64
                    }
                    akar_common::types::LogicalTypeID::Int32 => PhysicalTypeID::Int32,
                    akar_common::types::LogicalTypeID::Int16 => PhysicalTypeID::Int16,
                    akar_common::types::LogicalTypeID::Int8 => PhysicalTypeID::Int8,
                    akar_common::types::LogicalTypeID::UInt64 => PhysicalTypeID::UInt64,
                    akar_common::types::LogicalTypeID::UInt32 => PhysicalTypeID::UInt32,
                    akar_common::types::LogicalTypeID::UInt16 => PhysicalTypeID::UInt16,
                    akar_common::types::LogicalTypeID::UInt8 => PhysicalTypeID::UInt8,
                    akar_common::types::LogicalTypeID::Double => PhysicalTypeID::Double,
                    akar_common::types::LogicalTypeID::Float => PhysicalTypeID::Float,
                    akar_common::types::LogicalTypeID::String => PhysicalTypeID::String,
                    akar_common::types::LogicalTypeID::Blob => PhysicalTypeID::Blob,
                    akar_common::types::LogicalTypeID::Date => PhysicalTypeID::Int32,
                    akar_common::types::LogicalTypeID::Timestamp => PhysicalTypeID::Int64,
                    akar_common::types::LogicalTypeID::Interval => PhysicalTypeID::Interval,
                    akar_common::types::LogicalTypeID::List => PhysicalTypeID::List,
                    akar_common::types::LogicalTypeID::Array => PhysicalTypeID::Array,
                    akar_common::types::LogicalTypeID::Struct => PhysicalTypeID::Struct,
                    akar_common::types::LogicalTypeID::Node => PhysicalTypeID::Struct,
                    akar_common::types::LogicalTypeID::Rel => PhysicalTypeID::Struct,
                    akar_common::types::LogicalTypeID::InternalID => PhysicalTypeID::Struct, // Internal IDs are Structs
                    _ => PhysicalTypeID::Any,
                };
                let mut vv = ValueVector::new(phys_type, count);
                vv.resize(count);
                for row_offset in 0..count {
                    let val = &col_data[start + row_offset];
                    match val {
                        Value::Null => vv.set_null(row_offset, true),
                        Value::Int64(x) => {
                            let buf = &mut vv.data_mut()[row_offset * 8..(row_offset + 1) * 8];
                            buf.copy_from_slice(&x.to_le_bytes());
                        }
                        Value::Int32(x) => {
                            let buf = &mut vv.data_mut()[row_offset * 4..(row_offset + 1) * 4];
                            buf.copy_from_slice(&x.to_le_bytes());
                        }
                        Value::Double(x) => {
                            let buf = &mut vv.data_mut()[row_offset * 8..(row_offset + 1) * 8];
                            buf.copy_from_slice(&x.to_le_bytes());
                        }
                        Value::String(_) => {
                            vv.set_value(row_offset, val)?;
                        }
                        _ => {}
                    }
                }
                fields.push(vv);
            }

            let arrow_fields = fields
                .iter()
                .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
                .collect::<Vec<_>>();
            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
            chunks.push(DataChunk {
                fields: arrow_fields,
                field_types: arrow_field_types,
                size: count,
                field_names: col_names.clone(),
                sel_vector: None,
            });
        }

        Ok(chunks)
    }
}
