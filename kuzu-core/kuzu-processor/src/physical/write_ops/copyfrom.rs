//! Auto-extracted from physical_operator.rs
use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_storage::table::{ColumnDefinition, TableCatalog};
use std::path::Path;
use std::sync::Arc;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

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
    pub vfs: Arc<kuzu_common::file_system::VirtualFileSystemRegistry>,
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

        // 2. Build config and convert column schema
        let catalog_cols: Vec<kuzu_catalog::CatalogColumn> = self
            .columns
            .iter()
            .map(|c| kuzu_catalog::CatalogColumn {
                name: c.name.clone(),
                logical_type: c.logical_type,
                is_primary_key: c.is_primary_key,
                compression: kuzu_common::enums::CompressionType::Uncompressed,
                default_value: None,
            })
            .collect();

        // 3. Read the file
        let rows = match ext.as_str() {
            "csv" | "tsv" => {
                let mut config = kuzu_storage::csv_reader::CsvReaderConfig::from_options(&self.options);
                if ext == "tsv" && !self.options.contains_key("DELIM") && !self.options.contains_key("delim") {
                    config.delimiter = b'\t';
                }

                kuzu_storage::csv_reader::read_csv(&self.file_path, &self.vfs, &catalog_cols, &config)
                    .map_err(|e| format!("CSV read error: {e}"))?
            }
            #[cfg(feature = "parquet")]
            "parquet" => kuzu_storage::parquet_reader::read_parquet(&self.file_path, &self.vfs, &catalog_cols)
                .map_err(|e| format!("Parquet read error: {e}"))?,
            #[cfg(not(feature = "parquet"))]
            "parquet" => return Err("Parquet support not enabled (feature 'parquet' in kuzu-storage)".into()),
            _ => {
                return Err(format!(
                    "Unsupported file type: .{ext} (supported: .csv, .tsv, .parquet)"
                ));
            }
        };

        // 4. Insert rows into the table using batch insert
        let num_rows = rows.len();
        if num_rows == 0 {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            return Ok(vec![DataChunk::new(vec![v])]);
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
            let rels: Vec<(u64, u64, Vec<Value>)> = rows
                .iter()
                .map(|row| {
                    let from = match &row[0] {
                        Value::Int64(v) => *v as u64,
                        _ => 0, // Will fail validation below
                    };
                    let to = match &row[1] {
                        Value::Int64(v) => *v as u64,
                        _ => 0,
                    };
                    let props = row[2..].to_vec();
                    (from, to, props)
                })
                .collect();
            let count = table
                .insert_rels_batch(&rels)
                .map_err(|e| format!("Batch insert rel error: {e}"))?;
            tracing::info!(
                "COPY FROM: batch-inserted {count} rows into rel table '{}'",
                self.table_name
            );
        } else {
            return Err(format!("Table '{}' not found in storage catalog", self.table_name));
        }

        // Return success chunk with row count
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, num_rows as i64);
        Ok(vec![DataChunk::new(vec![v])])
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
            return Err(format!("Table '{}' does not have an ART index", self.table_name));
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
            return Ok(vec![DataChunk::new(vec![])]);
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
        use kuzu_common::types::PhysicalTypeID;
        use kuzu_common::vector::{DataChunk, ValueVector};

        let num_rows = output_columns.first().map(|c| c.len()).unwrap_or(0);
        if num_rows == 0 {
            return Ok(vec![DataChunk::new(vec![])]);
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
                    kuzu_common::types::LogicalTypeID::Bool => PhysicalTypeID::Bool,
                    kuzu_common::types::LogicalTypeID::Int64 | kuzu_common::types::LogicalTypeID::Serial => {
                        PhysicalTypeID::Int64
                    }
                    kuzu_common::types::LogicalTypeID::Int32 => PhysicalTypeID::Int32,
                    kuzu_common::types::LogicalTypeID::Int16 => PhysicalTypeID::Int16,
                    kuzu_common::types::LogicalTypeID::Int8 => PhysicalTypeID::Int8,
                    kuzu_common::types::LogicalTypeID::UInt64 => PhysicalTypeID::UInt64,
                    kuzu_common::types::LogicalTypeID::UInt32 => PhysicalTypeID::UInt32,
                    kuzu_common::types::LogicalTypeID::UInt16 => PhysicalTypeID::UInt16,
                    kuzu_common::types::LogicalTypeID::UInt8 => PhysicalTypeID::UInt8,
                    kuzu_common::types::LogicalTypeID::Double => PhysicalTypeID::Double,
                    kuzu_common::types::LogicalTypeID::Float => PhysicalTypeID::Float,
                    kuzu_common::types::LogicalTypeID::String => PhysicalTypeID::String,
                    kuzu_common::types::LogicalTypeID::Blob => PhysicalTypeID::Blob,
                    kuzu_common::types::LogicalTypeID::Date => PhysicalTypeID::Int32,
                    kuzu_common::types::LogicalTypeID::Timestamp => PhysicalTypeID::Int64,
                    kuzu_common::types::LogicalTypeID::Interval => PhysicalTypeID::Interval,
                    kuzu_common::types::LogicalTypeID::List => PhysicalTypeID::List,
                    kuzu_common::types::LogicalTypeID::Array => PhysicalTypeID::Array,
                    kuzu_common::types::LogicalTypeID::Struct => PhysicalTypeID::Struct,
                    kuzu_common::types::LogicalTypeID::Node => PhysicalTypeID::Struct,
                    kuzu_common::types::LogicalTypeID::Rel => PhysicalTypeID::Struct,
                    kuzu_common::types::LogicalTypeID::InternalID => PhysicalTypeID::Struct, // Internal IDs are Structs
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
                        Value::String(s) => {
                            let bytes = s.as_bytes();
                            let copy_len = bytes.len().min(255);
                            vv.data_mut()[row_offset * 256] = copy_len as u8;
                            vv.data_mut()[row_offset * 256 + 1..row_offset * 256 + 1 + copy_len]
                                .copy_from_slice(&bytes[..copy_len]);
                        }
                        _ => {}
                    }
                }
                fields.push(vv);
            }

            chunks.push(DataChunk {
                fields,
                size: count,
                field_names: col_names.clone(),
            });
        }

        Ok(chunks)
    }
}


