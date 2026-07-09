//! Auto-extracted from physical_operator.rs
use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_parser::ast::{Constant, Expression};
use kuzu_storage::table::{ColumnDefinition, TableCatalog};
use std::path::Path;
use std::sync::{Arc, Mutex};
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use super::scan_filter::PhysicalScan;
use crate::physical::common::store_value_in_vector;
// ==================== Unwind ====================

/// Physical operator for UNWIND — expands a list expression into rows.
pub struct PhysicalUnwind {
    pub expression: kuzu_parser::ast::Expression,
    pub variable: String,
}

impl PhysicalOperatorExec for PhysicalUnwind {
    fn operator_type(&self) -> &str {
        "unwind"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Evaluate the expression to get a list value
        let list_val = evaluate_unwind_expr(&self.expression);
        let items = match &list_val {
            kuzu_common::types::Value::List(items) => items.clone(),
            _ => return Err("UNWIND expression must evaluate to a list".into()),
        };

        if items.is_empty() {
            return Ok(Vec::new());
        }

        // Create a new ValueVector for the unwound variable
        let first_type = items
            .first()
            .map(|v| v.physical_type())
            .unwrap_or(PhysicalTypeID::Int64);

        let mut result_chunks = Vec::new();
        // If we have input data, repeat for each input row
        if let Some(chunk) = input.first() {
            for row in 0..chunk.size {
                let mut chunk_fields = Vec::new();
                for field in chunk.fields.iter() {
                    let val = field.get_value(row).unwrap_or(Value::Null);
                    let mut v = ValueVector::new(field.physical_type(), items.len());
                    v.resize(items.len());
                    for i in 0..items.len() {
                        store_value_in_vector(&mut v, i, &val);
                    }
                    chunk_fields.push(v);
                }
                // Add unwound vector
                let mut uw_v = ValueVector::new(first_type, items.len());
                uw_v.resize(items.len());
                for (i, item) in items.iter().enumerate() {
                    store_value_in_vector(&mut uw_v, i, item);
                }
                chunk_fields.push(uw_v);
                result_chunks.push(DataChunk::new(chunk_fields));
            }
        } else {
            // No input — just the unwound vector
            let mut uw_v = ValueVector::new(first_type, items.len());
            uw_v.resize(items.len());
            for (i, item) in items.iter().enumerate() {
                store_value_in_vector(&mut uw_v, i, item);
            }
            result_chunks.push(DataChunk::new(vec![uw_v]));
        }

        Ok(result_chunks)
    }
}

/// Evaluate an UNWIND expression to get the list value.
fn evaluate_unwind_expr(expr: &kuzu_parser::ast::Expression) -> Value {
    match expr {
        kuzu_parser::ast::Expression::List(items) => {
            let values: Vec<Value> = items.iter().map(expr_to_value).collect();
            Value::List(values)
        }
        _ => Value::List(Vec::new()),
    }
}

/// Convert an AST expression to a runtime Value (for simple constants).
fn expr_to_value(expr: &kuzu_parser::ast::Expression) -> Value {
    match expr {
        kuzu_parser::ast::Expression::Constant(c) => match c {
            kuzu_parser::ast::Constant::Null => Value::Null,
            kuzu_parser::ast::Constant::Bool(b) => Value::Bool(*b),
            kuzu_parser::ast::Constant::Integer(i) => Value::Int64(*i),
            kuzu_parser::ast::Constant::Float(f) => Value::Double(*f),
            kuzu_parser::ast::Constant::String(s) => Value::String(s.clone()),
        },
        _ => Value::Null,
    }
}



// ==================== Set ====================

/// Physical operator for SET — updates a property on matched rows.
pub struct PhysicalSet {
    pub table_name: String,
    pub table_id: u64,
    pub column_name: String,
    pub column_idx: usize,
    pub value: kuzu_parser::ast::Expression,
    pub is_node: bool,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalSet {
    fn operator_type(&self) -> &str {
        "set"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Collect row indices from input chunks (first column has row index)
        let mut rows_to_update: Vec<(u64, kuzu_common::types::Value)> = Vec::new();

        for chunk in &input {
            for row in 0..chunk.size {
                if let Some(field) = chunk.fields.first()
                    && let Some(val) = field.get_i64(row)
                {
                    // Evaluate the SET value expression against the current row
                    let set_val = evaluate_expression_for_row(&self.value, chunk, row);
                    rows_to_update.push((val as u64, set_val));
                }
            }
        }

        if rows_to_update.is_empty() {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            return Ok(vec![DataChunk::new(vec![v])]);
        }

        // Apply updates to the table
        let mut updated = 0u64;
        if self.is_node {
            if let Some(mut table) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
                for (row_idx, val) in &rows_to_update {
                    if table.update_cell(*row_idx, self.column_idx, val.clone()).is_ok() {
                        updated += 1;
                    }
                }
            } else {
                return Err(format!("Node table '{}' not found for SET", self.table_name));
            }
        } else {
            if let Some(mut table) = self.table_catalog.get_rel_table_by_name_mut(&self.table_name) {
                for (edge_idx, val) in &rows_to_update {
                    if table
                        .update_cell(*edge_idx as usize, self.column_idx, val.clone())
                        .is_ok()
                    {
                        updated += 1;
                    }
                }
            } else {
                return Err(format!("Rel table '{}' not found for SET", self.table_name));
            }
        }

        tracing::info!("SET: updated {updated} rows in '{}'", self.table_name);

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, updated as i64);
        Ok(vec![DataChunk::new(vec![v])])
    }
}

/// Simple expression evaluator for SET value expressions against a DataChunk row.
fn evaluate_expression_for_row(
    expr: &kuzu_parser::ast::Expression,
    chunk: &DataChunk,
    row: usize,
) -> kuzu_common::types::Value {
    match expr {
        kuzu_parser::ast::Expression::Constant(c) => match c {
            kuzu_parser::ast::Constant::Null => kuzu_common::types::Value::Null,
            kuzu_parser::ast::Constant::Bool(b) => kuzu_common::types::Value::Bool(*b),
            kuzu_parser::ast::Constant::Integer(i) => kuzu_common::types::Value::Int64(*i),
            kuzu_parser::ast::Constant::Float(f) => kuzu_common::types::Value::Double(*f),
            kuzu_parser::ast::Constant::String(s) => kuzu_common::types::Value::String(s.clone()),
        },
        _ => {
            // Fallback: try to get value from chunk fields
            if let Some(field) = chunk.fields.get(1) {
                field.get_value(row).unwrap_or(kuzu_common::types::Value::Null)
            } else {
                kuzu_common::types::Value::Null
            }
        }
    }
}


// ==================== Delete ====================

/// Physical operator for DELETE — removes rows from a node or rel table.
pub struct PhysicalDelete {
    pub table_name: String,
    pub table_id: u64,
    pub primary_key_column: String,
    pub is_node: bool,
    pub detach: bool,
    /// Row indices to delete (found by the scan/filter pipeline).
    pub row_indices: Vec<u64>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalDelete {
    fn operator_type(&self) -> &str {
        "delete"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Collect row indices from input chunks
        let mut rows_to_delete: Vec<u64> = self.row_indices.clone();

        // If input has data, extract row indices from it
        for chunk in &input {
            for row in 0..chunk.size {
                if let Some(field) = chunk.fields.first()
                    && let Some(val) = field.get_i64(row)
                {
                    rows_to_delete.push(val as u64);
                }
            }
        }

        if rows_to_delete.is_empty() {
            // No rows to delete - still return success
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            return Ok(vec![DataChunk::new(vec![v])]);
        }

        // Delete rows from the table
        let mut deleted = 0u64;
        if self.is_node {
            for &row_idx in &rows_to_delete {
                if !self.detach && self.table_catalog.has_incident_edges(self.table_id, row_idx) {
                    return Err(format!(
                        "Cannot delete node {} because it has incident edges (use DETACH DELETE)",
                        row_idx
                    ));
                }
            }
            if self.detach {
                for &row_idx in &rows_to_delete {
                    self.table_catalog.detach_node(self.table_id, row_idx);
                }
            }
            if let Some(mut table) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
                for &row_idx in &rows_to_delete {
                    if table.delete_row(row_idx).is_ok() {
                        deleted += 1;
                    }
                }
            } else {
                return Err(format!("Node table '{}' not found for DELETE", self.table_name));
            }
        } else {
            if let Some(mut table) = self.table_catalog.get_rel_table_by_name_mut(&self.table_name) {
                for &edge_idx in &rows_to_delete {
                    if table.delete_edge(edge_idx as usize).is_ok() {
                        deleted += 1;
                    }
                }
            } else {
                return Err(format!("Rel table '{}' not found for DELETE", self.table_name));
            }
        }

        tracing::info!("DELETE: removed {deleted} rows from '{}'", self.table_name);

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, deleted as i64);
        Ok(vec![DataChunk::new(vec![v])])
    }
}

/// Convert an AST Constant to a Value.
fn ast_constant_to_value(c: &Constant) -> Value {
    match c {
        Constant::Null => Value::Null,
        Constant::Bool(b) => Value::Bool(*b),
        Constant::Integer(i) => Value::Int64(*i),
        Constant::Float(f) => Value::Double(*f),
        Constant::String(s) => Value::String(s.clone()),
    }
}

/// Compute a hash of a Value for use in hash-based joins.
/// Hashes the discriminant (variant type) and the payload data.


// ==================== Foreach ====================

/// Physical FOREACH operator — iterates over list elements and executes sub-plans.
pub struct PhysicalForeach {
    pub variable: String,
    pub expression: Expression,
    pub sub_plans: Vec<Vec<kuzu_planner::logical_operator::LogicalOperator>>,
    pub function_registry: Option<Arc<Mutex<kuzu_function::registry::FunctionRegistry>>>,
    pub table_catalog: Option<Arc<TableCatalog>>,
    pub vfs: Option<Arc<kuzu_common::file_system::VirtualFileSystemRegistry>>,
}

impl PhysicalOperatorExec for PhysicalForeach {
    fn operator_type(&self) -> &str {
        "foreach"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // Evaluate the list expression
        let list_val = match &self.expression {
            Expression::List(items) => {
                let mut vals = Vec::with_capacity(items.len());
                for item in items {
                    if let Expression::Constant(c) = item {
                        vals.push(ast_constant_to_value(c));
                    } else {
                        vals.push(Value::Null);
                    }
                }
                Value::List(vals)
            }
            _ => {
                return Err(format!(
                    "FOREACH requires a list expression, got: {:?}",
                    self.expression
                ));
            }
        };

        let list_items = match &list_val {
            Value::List(items) => items.clone(),
            _ => return Ok(vec![]),
        };

        if list_items.is_empty() || self.sub_plans.is_empty() {
            return Ok(vec![]);
        }

        // For each list item, execute sub-plans with the item value in scope.
        // We use a simplified approach: create a DataChunk with the item value
        // and pass it to each sub-plan.
        for item in &list_items {
            for sub_plan in &self.sub_plans {
                // Create a single-row DataChunk containing the current item
                let phys_type = PhysicalScan::value_to_physical_type(item);
                let mut v = ValueVector::new(phys_type, 1);
                v.resize(1);
                store_value_in_vector(&mut v, 0, item);
                let _chunk = DataChunk::new(vec![v]);

                // Execute the sub-plan using the QueryProcessor-like pipeline
                // Use the processor module directly from the same crate
                let processor = crate::processor::QueryProcessor::with_catalog(
                    self.function_registry.clone().unwrap(),
                    self.table_catalog.clone().unwrap(),
                    self.vfs.clone().unwrap(),
                );
                let _result = processor.execute(sub_plan)?;
            }
        }

        // FOREACH produces no output rows (it's a write-only operation)
        Ok(vec![])
    }
}


// ==================== VectorSimilarityScan ====================

/// Physical operator for vector similarity search using an HNSW index.
///
/// Searches the `VectorIndexTable` for the top-K nearest neighbours and
/// looks up the corresponding rows from the `NodeTable` to produce
/// output columns including a `distance` column.
pub struct PhysicalVectorSimilarityScan {
    pub index_name: String,
    pub index_id: u64,
    pub query_vector: Vec<f64>,
    pub top_k: u64,
    pub table_name: String,
    pub table_catalog: Option<Arc<TableCatalog>>,
}

impl PhysicalOperatorExec for PhysicalVectorSimilarityScan {
    fn operator_type(&self) -> &str {
        "vector_similarity_scan"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let tc = self
            .table_catalog
            .clone()
            .ok_or_else(|| "No table catalog available for VectorSimilarityScan".to_string())?;

        // Resolve the vector index — by name if given, or find first index on the table
        let vi = if self.index_name.is_empty() {
            // Find the first vector index on this table
            // Scan all vector indexes to find one matching this table
            let index_name = {
                let mut found_name = String::new();
                for entry in tc.all_vector_indexes() {
                    if entry.table_name == self.table_name {
                        found_name = entry.name.clone();
                        break;
                    }
                }
                if found_name.is_empty() {
                    return Err(format!("No vector index found on table '{}'", self.table_name));
                }
                found_name
            };
            tc.get_vector_index_by_name(&index_name)
                .ok_or_else(|| format!("Vector index '{}' not found", index_name))?
        } else {
            tc.get_vector_index_by_name(&self.index_name)
                .ok_or_else(|| format!("Vector index '{}' not found", self.index_name))?
        };

        // Search the HNSW index for top-K nearest neighbours
        let results = vi.hnsw().search(&self.query_vector, self.top_k as usize);
        drop(vi); // Release the DashMap reference

        if results.is_empty() {
            return Ok(vec![DataChunk::new(vec![])]);
        }

        // Look up rows from the node table
        let node_table = tc
            .get_node_table_by_name(&self.table_name)
            .ok_or_else(|| format!("Node table '{}' not found", self.table_name))?;

        let num_cols = node_table.columns.len();
        let num_results = results.len();

        // Build output columns: all table columns + distance column
        let mut output_columns: Vec<Vec<Value>> = vec![Vec::with_capacity(num_results); num_cols + 1];

        for (dist, row_id) in &results {
            // Add distance as the last column
            output_columns[num_cols].push(Value::Double(*dist));

            // Look up each column value from the node table
            for (col_idx, out_col) in output_columns.iter_mut().enumerate().take(num_cols) {
                match node_table.get_value(*row_id, col_idx) {
                    Some(val) => out_col.push(val.clone()),
                    None => out_col.push(Value::Null),
                }
            }
        }

        drop(node_table);

        // Convert column-major Vec<Vec<Value>> to DataChunks
        use kuzu_common::types::PhysicalTypeID;
        use kuzu_common::vector::{DataChunk, ValueVector};

        let mut fields = Vec::with_capacity(num_cols + 1);

        // Add table columns
        for (_col_idx, col_data) in output_columns.iter().enumerate().take(num_cols) {
            let mut v = ValueVector::new(PhysicalTypeID::Double, num_results);
            v.resize(num_results);
            for (i, val) in col_data.iter().enumerate() {
                match val {
                    Value::Double(d) => v.set_double(i, *d),
                    Value::Int64(x) => {
                        let buf = &mut v.data_mut()[i * 8..(i + 1) * 8];
                        buf.copy_from_slice(&x.to_le_bytes());
                        v.set_null(i, false);
                    }
                    Value::String(s) => {
                        let bytes = s.as_bytes();
                        let len = bytes.len().min(255) as u8;
                        v.data_mut()[i * 256] = len;
                        let copy_len = bytes.len().min(255);
                        v.data_mut()[i * 256 + 1..i * 256 + 1 + copy_len].copy_from_slice(&bytes[..copy_len]);
                        v.set_null(i, false);
                    }
                    Value::Null => {
                        v.set_null(i, true);
                    }
                    _ => {
                        v.set_null(i, true);
                    }
                }
            }
            fields.push(v);
        }

        // Add distance column
        let dist_data = &output_columns[num_cols];
        let mut dist_v = ValueVector::new(PhysicalTypeID::Double, num_results);
        dist_v.resize(num_results);
        for (i, val) in dist_data.iter().enumerate() {
            if let Value::Double(d) = val {
                dist_v.set_double(i, *d);
            } else {
                dist_v.set_null(i, true);
            }
        }
        fields.push(dist_v);

        Ok(vec![DataChunk {
            fields,
            size: num_results,
            field_names: vec![],
        }])
    }
}


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


// ==================== PhysicalExplain ====================

/// Physical EXPLAIN operator — serializes a logical plan tree to a human-readable
/// string and returns it as a single-row result.
///
/// Corresponds to C++ `PlanPrinter::printPlanToOstream` and `mapExplain`.
pub struct PhysicalExplain {
    /// The inner logical operator tree to serialize.
    pub inner_plan: String,
}

impl PhysicalOperatorExec for PhysicalExplain {
    fn operator_type(&self) -> &str {
        "explain"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        use kuzu_common::types::PhysicalTypeID;
        use kuzu_common::vector::{DataChunk, ValueVector};

        let plan_str = self.inner_plan.clone();
        let mut vv = ValueVector::new(PhysicalTypeID::String, 1);
        vv.resize(1);
        let bytes = plan_str.as_bytes();
        let copy_len = bytes.len().min(255);
        vv.data_mut()[0] = copy_len as u8;
        if copy_len > 0 {
            vv.data_mut()[1..1 + copy_len].copy_from_slice(&bytes[..copy_len]);
        }
        // For long strings, store the full string in the ValueVector's overflow
        // We store the original Value for the query result
        let chunk = DataChunk {
            fields: vec![vv],
            size: 1,
            field_names: vec![],
        };
        Ok(vec![chunk])
    }
}


// ==================== RecursiveExtend ====================

/// Physical operator for variable-length path matching (BFS traversal).
///
/// For each source node, performs BFS up to `upper_bound` depth and emits
/// result rows for all nodes reachable at depths between `lower_bound` and
/// `upper_bound`.
///
/// Uses GDS-style path tracking to record actual paths (node IDs + edge IDs)
/// and enforces path semantics (WALK/TRAIL/ACYCLIC).
///
/// Produces a DataChunk with columns:
///   (src_offset, dst_offset, length, path_node_ids, path_edge_ids[, cost])
///
/// When `weight_property` is `Some`, uses Dijkstra's algorithm for weighted
/// shortest path traversal (port of C++ `WeightedSPPathsFunction`).
/// The `cost` column is appended to the output.
pub struct PhysicalRecursiveExtend {
    pub source_table_id: u64,
    pub rel_table_ids: Vec<u64>,
    pub lower_bound: u64,
    pub upper_bound: u64,
    pub direction: kuzu_common::enums::ExtendDirection,
    pub semantic: kuzu_common::enums::PathSemantic,
    pub table_catalog: Option<Arc<TableCatalog>>,
    /// Optional edge weight property name for weighted shortest path.
    /// When set, Dijkstra traversal is used instead of BFS.
    pub weight_property: Option<String>,
    /// Optional name for the cost output column.
    pub cost_output_name: Option<String>,
}

impl PhysicalOperatorExec for PhysicalRecursiveExtend {
    fn operator_type(&self) -> &str {
        "recursive_extend"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        use kuzu_common::enums::ExtendDirection;
        use kuzu_common::enums::PathSemantic;
        use kuzu_common::types::Value;
        use kuzu_common::vector::ValueVector;
        use std::collections::{HashMap, VecDeque};

        let catalog = self
            .table_catalog
            .as_ref()
            .ok_or_else(|| "No table catalog available for RecursiveExtend".to_string())?;

        // Build adjacency with edge IDs: neighbor_offset -> (neighbor_offset, edge_id)
        let mut fwd_adj: HashMap<u64, Vec<(u64, u64)>> = HashMap::new();
        let mut rev_adj: HashMap<u64, Vec<(u64, u64)>> = HashMap::new();
        // Edge weight lookup: edge_id -> weight (for weighted shortest path)
        let mut edge_weights: HashMap<u64, f64> = HashMap::new();
        // Whether we're doing weighted shortest path
        let is_weighted = self.weight_property.is_some();
        // Resolve weight column index for each rel table
        let mut weight_col_idx: HashMap<u64, Option<usize>> = HashMap::new();

        for &rel_table_id in &self.rel_table_ids {
            if let Some(rel_table) = catalog.get_rel_table(rel_table_id) {
                // Resolve weight column index
                if let Some(ref wp) = self.weight_property {
                    let idx = rel_table.columns.iter().position(|c| c.name == *wp);
                    weight_col_idx.insert(rel_table_id, idx);
                }

                for (&src, neighbors) in rel_table.fwd_adj.iter() {
                    fwd_adj
                        .entry(src)
                        .or_default()
                        .extend(neighbors.iter().map(|(dst, edge_idx)| (*dst, *edge_idx as u64)));
                    // Pre-compute edge weights
                    if is_weighted && let Some(col_idx) = weight_col_idx.get(&rel_table_id).and_then(|&c| c) {
                        for &(_dst, edge_idx) in neighbors {
                            if let Some(weight_val) =
                                rel_table.properties.get(col_idx).and_then(|col| col.get(edge_idx))
                            {
                                let w = match weight_val {
                                    Value::Int64(i) => *i as f64,
                                    Value::Double(d) => *d,
                                    Value::Float(f) => *f as f64,
                                    Value::Int32(i) => *i as f64,
                                    _ => 1.0, // default weight for unrecognized types
                                };
                                edge_weights.insert(edge_idx as u64, w);
                            }
                        }
                    }
                }
                for (&dst, neighbors) in rel_table.rev_adj.iter() {
                    rev_adj
                        .entry(dst)
                        .or_default()
                        .extend(neighbors.iter().map(|(src, edge_idx)| (*src, *edge_idx as u64)));
                }
            }
        }

        // Collect source node offsets from input
        let source_offsets: Vec<i64> = if input.is_empty() || input[0].fields.is_empty() {
            let mut all: Vec<i64> = fwd_adj
                .keys()
                .chain(rev_adj.keys())
                .copied()
                .map(|k| k as i64)
                .collect();
            all.sort();
            all.dedup();
            all
        } else {
            let field = &input[0].fields[0];
            let num_rows = input[0].size;
            let mut offsets = Vec::with_capacity(num_rows);
            for i in 0..num_rows {
                if !field.is_null(i) {
                    let offset = i64::from_le_bytes(field.data()[i * 8..i * 8 + 8].try_into().unwrap());
                    offsets.push(offset);
                }
            }
            offsets
        };

        if source_offsets.is_empty() {
            return Ok(vec![DataChunk::new(vec![])]);
        }

        // Result columns
        let mut result_src: Vec<i64> = Vec::new();
        let mut result_dst: Vec<i64> = Vec::new();
        let mut result_len: Vec<i64> = Vec::new();
        let mut result_cost: Vec<f64> = Vec::new(); // only used for weighted
        // Path tracking: for each result, store the sequence of (node_id, edge_id) pairs
        let mut result_path_nodes: Vec<Vec<i64>> = Vec::new();
        let mut result_path_edges: Vec<Vec<i64>> = Vec::new();

        for &src in &source_offsets {
            let src_u = src as u64;

            if is_weighted {
                // === Weighted Shortest Path: Dijkstra ===
                use std::cmp::Reverse;
                use std::collections::BinaryHeap;

                // Use i64 for priority queue (cost * PRECISION) since f64 doesn't implement Ord.
                // PRECISION = 1000 captures 3 decimal places.
                const COST_PRECISION: i64 = 1000;

                // Helper to convert f64 cost to i64 for the pq
                let cost_to_i64 = |c: f64| -> i64 { (c * COST_PRECISION as f64).round() as i64 };

                // Parent map: child -> (parent, edge_id, depth, cumulative_cost)
                let mut parents: HashMap<u64, (u64, u64, u64, f64)> = HashMap::new();
                let mut pq: BinaryHeap<Reverse<(i64, u64)>> = BinaryHeap::new();

                pq.push(Reverse((cost_to_i64(0.0), src_u)));
                parents.insert(src_u, (u64::MAX, u64::MAX, 0, 0.0));

                while let Some(Reverse((cur_cost_i64, node))) = pq.pop() {
                    let cur_cost = cur_cost_i64 as f64 / COST_PRECISION as f64;
                    let cur_depth = parents.get(&node).map(|&(_, _, d, _)| d).unwrap_or(0);

                    // If we already found a better path to this node, skip
                    if let Some(&(_, _, _, best_cost)) = parents.get(&node)
                        && cur_cost > best_cost + 1e-9
                    {
                        continue;
                    }

                    if cur_depth >= self.upper_bound {
                        continue;
                    }

                    // Get neighbors
                    let neighbors: Vec<(u64, u64)> = match self.direction {
                        ExtendDirection::Fwd => fwd_adj.get(&node).cloned().unwrap_or_default(),
                        ExtendDirection::Bwd => rev_adj.get(&node).cloned().unwrap_or_default(),
                        ExtendDirection::Both => {
                            let mut nbrs = fwd_adj.get(&node).cloned().unwrap_or_default();
                            if let Some(bwd) = rev_adj.get(&node) {
                                nbrs.extend(bwd.iter().copied());
                            }
                            nbrs
                        }
                    };

                    for (nbr, edge_id) in neighbors {
                        let edge_w = edge_weights.get(&edge_id).copied().unwrap_or(1.0);
                        let new_cost = cur_cost + edge_w;
                        let new_depth = cur_depth + 1;

                        let should_visit = match parents.get(&nbr) {
                            Some(&(_, _, _, existing_cost)) => new_cost < existing_cost - 1e-9,
                            None => true,
                        };

                        if should_visit {
                            parents.insert(nbr, (node, edge_id, new_depth, new_cost));
                            pq.push(Reverse((cost_to_i64(new_cost), nbr)));
                        }
                    }
                }

                // Emit results
                for (&node, &(_parent, _eid, depth, cost)) in &parents {
                    if depth < self.lower_bound || depth > self.upper_bound {
                        continue;
                    }
                    if depth == 0 && self.lower_bound > 0 {
                        continue;
                    }

                    result_src.push(src);
                    result_dst.push(node as i64);
                    result_len.push(depth as i64);
                    result_cost.push(cost);

                    // Reconstruct path
                    let mut cur = node;
                    let mut temp_nodes = vec![node as i64];
                    let mut temp_edges = Vec::new();

                    while cur != src_u {
                        if let Some(&(parent, eid, _, _)) = parents.get(&cur) {
                            if parent == u64::MAX {
                                break;
                            }
                            temp_edges.push(eid as i64);
                            temp_nodes.push(parent as i64);
                            cur = parent;
                        } else {
                            break;
                        }
                    }

                    temp_nodes.reverse();
                    temp_edges.reverse();
                    let mut path_nodes = vec![src];
                    path_nodes.extend(temp_nodes);
                    result_path_nodes.push(path_nodes);
                    result_path_edges.push(temp_edges);
                }
            } else {
                // === Unweighted: BFS ===
                let mut queue = VecDeque::new();
                // Parent map: child -> (parent, edge_id, depth)
                let mut parents: HashMap<u64, (u64, u64, u64)> = HashMap::new();
                queue.push_back((src_u, 0u64));
                parents.insert(src_u, (u64::MAX, u64::MAX, 0));

                let semantic = self.semantic;

                while let Some((node, depth)) = queue.pop_front() {
                    if depth >= self.upper_bound {
                        continue;
                    }

                    let neighbors: Vec<(u64, u64)> = match self.direction {
                        ExtendDirection::Fwd => fwd_adj.get(&node).cloned().unwrap_or_default(),
                        ExtendDirection::Bwd => rev_adj.get(&node).cloned().unwrap_or_default(),
                        ExtendDirection::Both => {
                            let mut nbrs = fwd_adj.get(&node).cloned().unwrap_or_default();
                            if let Some(bwd) = rev_adj.get(&node) {
                                nbrs.extend(bwd.iter().copied());
                            }
                            nbrs
                        }
                    };

                    'neighbors: for (nbr, edge_id) in neighbors {
                        if parents.contains_key(&nbr) {
                            match semantic {
                                PathSemantic::Walk | PathSemantic::Acyclic => continue 'neighbors,
                                PathSemantic::Trail => {
                                    let mut cur = node;
                                    while let Some(&(p, eid, _)) = parents.get(&cur) {
                                        if eid == edge_id {
                                            continue 'neighbors;
                                        }
                                        if p == u64::MAX {
                                            break;
                                        }
                                        cur = p;
                                    }
                                }
                            }
                        }

                        let new_depth = depth + 1;
                        parents.insert(nbr, (node, edge_id, new_depth));
                        queue.push_back((nbr, new_depth));
                    }
                }

                // Emit results for nodes at valid depths
                for (&node, &(_parent_node, _edge_id, depth)) in &parents {
                    if depth < self.lower_bound || depth > self.upper_bound {
                        continue;
                    }
                    if depth == 0 && self.lower_bound > 0 {
                        continue;
                    }

                    result_src.push(src);
                    result_dst.push(node as i64);
                    result_len.push(depth as i64);

                    // Reconstruct path
                    let mut cur = node;
                    let mut temp_nodes = vec![node as i64];
                    let mut temp_edges = Vec::new();

                    while cur != src_u {
                        if let Some(&(parent, eid, _)) = parents.get(&cur) {
                            if parent == u64::MAX {
                                break;
                            }
                            temp_edges.push(eid as i64);
                            temp_nodes.push(parent as i64);
                            cur = parent;
                        } else {
                            break;
                        }
                    }

                    temp_nodes.reverse();
                    temp_edges.reverse();
                    let mut path_nodes = vec![src];
                    path_nodes.extend(temp_nodes);
                    result_path_nodes.push(path_nodes);
                    result_path_edges.push(temp_edges);
                }
            }
        }

        // Build output DataChunk
        let num_results = result_src.len();
        if num_results == 0 {
            return Ok(vec![DataChunk::new(vec![])]);
        }

        // Column 0-2: primitive Int64 vectors
        let mut src_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, num_results);
        let mut dst_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, num_results);
        let mut len_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, num_results);

        for i in 0..num_results {
            let offset = i * 8;
            src_v.data_mut()[offset..offset + 8].copy_from_slice(&result_src[i].to_le_bytes());
            src_v.set_null(i, false);
            dst_v.data_mut()[offset..offset + 8].copy_from_slice(&result_dst[i].to_le_bytes());
            dst_v.set_null(i, false);
            len_v.data_mut()[offset..offset + 8].copy_from_slice(&result_len[i].to_le_bytes());
            len_v.set_null(i, false);
        }
        src_v.resize(num_results);
        dst_v.resize(num_results);
        len_v.resize(num_results);

        // Column 3-4: create Value vectors for path lists, then convert to columns
        // Use Vec<Option<Value>> to store per-row path data
        let mut path_nodes_col: Vec<Value> = Vec::with_capacity(num_results);
        let mut path_edges_col: Vec<Value> = Vec::with_capacity(num_results);

        for i in 0..num_results {
            // Path nodes as List(Int64)
            let node_vals: Vec<Value> = result_path_nodes[i].iter().map(|&n| Value::Int64(n)).collect();
            path_nodes_col.push(Value::List(node_vals));
            // Path edges as List(Int64)
            let edge_vals: Vec<Value> = result_path_edges[i].iter().map(|&e| Value::Int64(e)).collect();
            path_edges_col.push(Value::List(edge_vals));
        }

        // Store List values in ValueVector via set_value
        let mut path_nodes_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::List, num_results);
        let mut path_edges_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::List, num_results);

        for (i, val) in path_nodes_col.iter().enumerate() {
            path_nodes_v.set_value(i, val).ok();
        }
        for (i, val) in path_edges_col.iter().enumerate() {
            path_edges_v.set_value(i, val).ok();
        }

        // When weighted, include cost column
        let has_cost = is_weighted;

        if has_cost {
            let mut cost_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Double, num_results);
            for (i, cost) in result_cost.iter().enumerate().take(num_results) {
                let offset = i * 8;
                cost_v.data_mut()[offset..offset + 8].copy_from_slice(&cost.to_le_bytes());
                cost_v.set_null(i, false);
            }
            cost_v.resize(num_results);

            Ok(vec![DataChunk {
                fields: vec![src_v, dst_v, len_v, path_nodes_v, path_edges_v, cost_v],
                size: num_results,
                field_names: vec![],
            }])
        } else {
            Ok(vec![DataChunk {
                fields: vec![src_v, dst_v, len_v, path_nodes_v, path_edges_v],
                size: num_results,
                field_names: vec![],
            }])
        }
    }
}

pub struct PhysicalCreateNode {
    pub table_name: String,
    pub table_id: u64,
    pub out_var_name: String,
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalCreateNode {
    pub fn execute(&self, input: Vec<DataChunk>) -> Result<Vec<DataChunk>, String> {
        if input.is_empty() {
            return Ok(input);
        }

        let mut table = self
            .table_catalog
            .get_node_table_by_name_mut(&self.table_name)
            .ok_or_else(|| format!("Node table {} not found", self.table_name))?;

        // For each input chunk, we create nodes and attach the new node IDs
        let mut output = Vec::with_capacity(input.len());

        for mut chunk in input {
            let mut node_ids = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, chunk.size);

            for i in 0..chunk.size {
                let mut values = vec![kuzu_common::types::Value::Null; table.columns.len()];
                for (prop_name, prop_expr) in &self.properties {
                    if let Some(col_idx) = table.columns.iter().position(|c| c.name == *prop_name) {
                        values[col_idx] = evaluate_expression_for_row(prop_expr, &chunk, i);
                    }
                }

                let row_offset = table.insert_row(values)?;
                node_ids.data_mut()[i * 8..(i + 1) * 8].copy_from_slice(&(row_offset as i64).to_le_bytes());
                node_ids.set_null(i, false);
            }
            node_ids.resize(chunk.size);

            chunk.fields.push(node_ids);
            chunk.field_names.push(self.out_var_name.clone());
            output.push(chunk);
        }

        Ok(output)
    }
}

pub struct PhysicalCreateRel {
    pub table_name: String,
    pub table_id: u64,
    pub src_node_name: String,
    pub dst_node_name: String,
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalCreateRel {
    pub fn execute(&self, input: Vec<DataChunk>) -> Result<Vec<DataChunk>, String> {
        if input.is_empty() {
            return Ok(input);
        }

        let mut table = self
            .table_catalog
            .get_rel_table_by_name_mut(&self.table_name)
            .ok_or_else(|| format!("Rel table {} not found", self.table_name))?;

        let mut output = Vec::with_capacity(input.len());

        for chunk in input {
            let src_name_id = format!("{}.{}", self.src_node_name, "_id");
            let src_name_pk = format!("{}.{}", self.src_node_name, "id");
            let src_idx = chunk
                .field_names
                .iter()
                .position(|name| name == &src_name_id)
                .or_else(|| chunk.field_names.iter().position(|name| name == &self.src_node_name))
                .or_else(|| chunk.field_names.iter().position(|name| name == &src_name_pk))
                .ok_or_else(|| format!("Source node variable {} not found", self.src_node_name))?;

            let dst_name_id = format!("{}.{}", self.dst_node_name, "_id");
            let dst_name_pk = format!("{}.{}", self.dst_node_name, "id");
            let dst_idx = chunk
                .field_names
                .iter()
                .position(|name| name == &dst_name_id)
                .or_else(|| chunk.field_names.iter().position(|name| name == &self.dst_node_name))
                .or_else(|| chunk.field_names.iter().position(|name| name == &dst_name_pk))
                .ok_or_else(|| format!("Destination node variable {} not found", self.dst_node_name))?;

            let src_vec = &chunk.fields[src_idx];
            let dst_vec = &chunk.fields[dst_idx];

            let mut inserted = 0;
            for i in 0..chunk.size {
                if src_vec.is_null(i) || dst_vec.is_null(i) {
                    continue; // Skip creating relationships involving NULL nodes
                }

                let mut src_bytes = [0u8; 8];
                src_bytes.copy_from_slice(&src_vec.data()[i * 8..(i + 1) * 8]);
                let src_id = i64::from_le_bytes(src_bytes) as u64;

                let mut dst_bytes = [0u8; 8];
                dst_bytes.copy_from_slice(&dst_vec.data()[i * 8..(i + 1) * 8]);
                let dst_id = i64::from_le_bytes(dst_bytes) as u64;

                let mut values = vec![kuzu_common::types::Value::Null; table.columns.len()];
                for (prop_name, prop_expr) in &self.properties {
                    if let Some(col_idx) = table.columns.iter().position(|c| c.name == *prop_name) {
                        values[col_idx] = evaluate_expression_for_row(prop_expr, &chunk, i);
                    }
                }

                table.insert_rel(src_id, dst_id, values)?;
                inserted += 1;
            }
            println!(
                "PhysicalCreateRel inserted {} relationships from chunk of size {}",
                inserted, chunk.size
            );

            output.push(chunk);
        }

        Ok(output)
    }
}

/// Physical operator for extending from a source node through a relationship.
///
/// Takes input chunks from the source node scan, and for each source row,
/// looks up adjacency list entries in the relationship table, producing
/// output rows that include the source fields, relationship properties,
/// and destination node properties.
///
/// Ported from C++ `ScanRelTable` (the physical extend operator).
pub struct PhysicalExtend {
    /// Name of the relationship table.
    pub rel_table_name: String,
    /// ID of the relationship table.
    pub rel_table_id: u64,
    /// Variable name of the bound (source) node.
    pub bound_node_var: String,
    /// Direction of the extend.
    pub direction: kuzu_parser::ast::EdgeDirection,
    /// Variable name of the destination node.
    pub dst_node_var: String,
    /// Table name of the destination node.
    pub dst_table_name: String,
    /// Table ID of the destination node.
    pub dst_table_id: u64,
    /// Table catalog for data access.
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalExtend {
    pub fn execute(&self, input: Vec<DataChunk>) -> Result<Vec<DataChunk>, String> {
        if input.is_empty() || input.iter().all(|c| c.size == 0) {
            return Ok(input);
        }

        // Collect rel table data upfront (owned)
        let (fwd_adj, rev_adj, rel_props, rel_cols) = {
            let rel_table = self
                .table_catalog
                .get_rel_table_by_name(&self.rel_table_name)
                .ok_or_else(|| format!("Rel table {} not found", self.rel_table_name))?;
            let fwd = rel_table.fwd_adj.clone();
            let rev = rel_table.rev_adj.clone();
            let props = rel_table.properties.clone();
            let cols = rel_table.columns.clone();
            (fwd, rev, props, cols)
        };

        // Collect dest node table data upfront (owned)
        let (dest_data, dest_cols, dest_pk_col) = {
            let dest_table = self
                .table_catalog
                .get_node_table_by_name(&self.dst_table_name)
                .ok_or_else(|| format!("Node table {} not found", self.dst_table_name))?;
            let data = dest_table.to_column_major_data();
            let cols = dest_table.columns.clone();
            let pk = dest_table.primary_key_column;
            (data, cols, pk)
        };

        // Build PK → row offset map for destination lookups
        let pk_to_row: std::collections::HashMap<u64, usize> = if dest_pk_col < dest_data.len() {
            dest_data[dest_pk_col]
                .iter()
                .enumerate()
                .filter_map(|(row, val)| {
                    if let Value::Int64(id) = val {
                        Some((*id as u64, row))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            std::collections::HashMap::new()
        };

        let mut output = Vec::with_capacity(input.len());

        for chunk in input {
            // Find the bound node column in the chunk
            let bound_name_id = format!("{}.{}", self.bound_node_var, "_id");
            let bound_name_pk = format!("{}.{}", self.bound_node_var, "id");
            let bound_idx = chunk
                .field_names
                .iter()
                .position(|name| name == &bound_name_id)
                .or_else(|| chunk.field_names.iter().position(|name| name == &self.bound_node_var))
                .or_else(|| chunk.field_names.iter().position(|name| name == &bound_name_pk))
                .ok_or_else(|| {
                    format!(
                        "Bound node variable {} not found in Extend input. Available fields: {:?}",
                        self.bound_node_var, chunk.field_names
                    )
                })?;

            // Calculate total output rows and build row mapping
            let mut total_rows = 0;
            let mut row_mappings: Vec<(usize, u64, usize)> = Vec::new(); // (input_row, dst_offset, edge_idx)

            for i in 0..chunk.size {
                if chunk.fields[bound_idx].is_null(i) {
                    continue;
                }
                let offset = i * 8;
                let src_vec_data = chunk.fields[bound_idx].data();
                if offset + 8 > src_vec_data.len() {
                    continue;
                }
                let mut src_bytes = [0u8; 8];
                src_bytes.copy_from_slice(&src_vec_data[offset..offset + 8]);
                let src_id = i64::from_le_bytes(src_bytes) as u64;

                let edges: Vec<(u64, usize)> = match self.direction {
                    kuzu_parser::ast::EdgeDirection::LeftToRight => fwd_adj.get(&src_id).cloned().unwrap_or_default(),
                    kuzu_parser::ast::EdgeDirection::RightToLeft => rev_adj.get(&src_id).cloned().unwrap_or_default(),
                    kuzu_parser::ast::EdgeDirection::Both => {
                        let mut all = fwd_adj.get(&src_id).cloned().unwrap_or_default();
                        if let Some(rev) = rev_adj.get(&src_id) {
                            all.extend(rev.clone());
                        }
                        all
                    }
                };

                for &(dst_offset, edge_idx) in &edges {
                    if !pk_to_row.contains_key(&dst_offset)
                        && dst_offset as usize >= dest_data.first().map(|c| c.len()).unwrap_or(0)
                    {
                        continue;
                    }
                    total_rows += 1;
                    row_mappings.push((i, dst_offset, edge_idx));
                }
            }

            if total_rows == 0 {
                output.push(DataChunk::new(vec![]));
                continue;
            }

            // Build output:
            // Column layout: [input_fields | rel_properties | dest_node_fields]
            let num_input_fields = chunk.fields.len();
            let num_rel_cols = rel_cols.len();
            let num_dest_cols = dest_cols.len();
            let num_out_cols = num_input_fields + num_rel_cols + num_dest_cols;

            // Build column-major data
            let mut out_data: Vec<Vec<Value>> = vec![Vec::with_capacity(total_rows); num_out_cols];

            for &(input_row, dst_offset, edge_idx) in &row_mappings {
                // Copy input fields
                for col in 0..num_input_fields {
                    let val = chunk.fields[col].get_value(input_row).unwrap_or(Value::Null);
                    out_data[col].push(val);
                }
                // Copy rel properties
                for col in 0..num_rel_cols {
                    let val = rel_props
                        .get(col)
                        .and_then(|c| c.get(edge_idx))
                        .cloned()
                        .unwrap_or(Value::Null);
                    out_data[num_input_fields + col].push(val);
                }
                // Copy dest node properties
                let dest_row = pk_to_row.get(&dst_offset).copied();
                for col in 0..num_dest_cols {
                    let val = dest_row
                        .and_then(|r| dest_data.get(col).and_then(|c| c.get(r)))
                        .cloned()
                        .unwrap_or_else(|| {
                            dest_data
                                .get(col)
                                .and_then(|c| c.get(dst_offset as usize))
                                .cloned()
                                .unwrap_or(Value::Null)
                        });
                    out_data[num_input_fields + num_rel_cols + col].push(val);
                }
            }

            // Convert column-major data to ValueVectors
            let mut fields = Vec::with_capacity(num_out_cols);
            let mut field_names = Vec::with_capacity(num_out_cols);

            // Input field names (already prefixed)
            for col in 0..num_input_fields {
                let phys_type = chunk.fields[col].physical_type();
                let mut v = ValueVector::new(phys_type, total_rows);
                v.resize(total_rows);
                for row in 0..total_rows {
                    store_value_in_vector(&mut v, row, &out_data[col][row]);
                }
                fields.push(v);
                if col < chunk.field_names.len() {
                    field_names.push(chunk.field_names[col].clone());
                } else {
                    field_names.push(format!("field_{}", col));
                }
            }

            // Rel field names (prefixed with rel table name)
            for col in 0..num_rel_cols {
                let phys_type = if col < rel_cols.len() {
                    PhysicalScan::logical_to_physical(&rel_cols[col].logical_type)
                } else {
                    PhysicalTypeID::Int64
                };
                let mut v = ValueVector::new(phys_type, total_rows);
                v.resize(total_rows);
                for row in 0..total_rows {
                    store_value_in_vector(&mut v, row, &out_data[num_input_fields + col][row]);
                }
                fields.push(v);
                let rel_prefix = &self.rel_table_name;
                let col_name = rel_cols.get(col).map(|c| c.name.as_str()).unwrap_or("");
                field_names.push(format!("{}.{}", rel_prefix, col_name));
            }

            // Dest field names (prefixed with dest variable)
            for col in 0..num_dest_cols {
                let phys_type = if col < dest_cols.len() {
                    PhysicalScan::logical_to_physical(&dest_cols[col].logical_type)
                } else {
                    PhysicalTypeID::Int64
                };
                let mut v = ValueVector::new(phys_type, total_rows);
                v.resize(total_rows);
                for row in 0..total_rows {
                    store_value_in_vector(&mut v, row, &out_data[num_input_fields + num_rel_cols + col][row]);
                }
                fields.push(v);
                let prefix = &self.dst_node_var;
                let col_name = dest_cols.get(col).map(|c| c.name.as_str()).unwrap_or("");
                field_names.push(format!("{}.{}", prefix, col_name));
            }

            output.push(DataChunk {
                fields,
                size: total_rows,
                field_names,
            });
        }

        Ok(output)
    }
}


// ==================== DDL & FTS ====================

/// Physical COUNT on rel table — optimized via CSR metadata (Ladybug).
/// Instead of scanning all edges, directly reads the edge count from the RelTable.
pub struct PhysicalCountRelTable {
    pub table_name: String,
    pub table_id: u64,
    pub table_catalog: Option<Arc<TableCatalog>>,
}

impl PhysicalOperatorExec for PhysicalCountRelTable {
    fn operator_type(&self) -> &str {
        "count_rel_table"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let tc = self
            .table_catalog
            .as_ref()
            .ok_or_else(|| "No table catalog for CountRelTable".to_string())?;

        let count = if let Some(table) = tc.get_rel_table(self.table_id) {
            table.num_rows as i64
        } else {
            0
        };

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, count);
        Ok(vec![DataChunk::new(vec![v])])
    }
}

/// Physical operator for `CREATE FTS INDEX` — builds 3 macro tables:
/// 1. `fts_{idx}_docs`: node table (doc_id INT64, text STRING)
/// 2. `fts_{idx}_terms`: node table (term_id INT64, term STRING, doc_freq INT64)
/// 3. `fts_{idx}_appears_in`: rel table (FROM terms TO docs, term_freq INT64)
pub struct PhysicalCreateFtsIndex {
    pub index_name: String,
    pub table_name: String,
    pub column_name: String,
    pub docs_table: String,
    pub terms_table: String,
    pub posting_table: String,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalCreateFtsIndex {
    fn operator_type(&self) -> &str {
        "create_fts_index"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // Locate source table
        let source_table = match self.table_catalog.get_node_table_by_name(&self.table_name) {
            Some(t) => t,
            None => return Err(format!("Table '{}' not found", self.table_name)),
        };
        let col_idx = source_table
            .columns
            .iter()
            .position(|c| c.name == self.column_name)
            .ok_or_else(|| format!("Column '{}' not found in '{}'", self.column_name, self.table_name))?;

        // Ensure macro tables exist; create if needed
        if self.table_catalog.get_node_table_by_name(&self.docs_table).is_none() {
            let docs_cols = vec![
                kuzu_storage::table::ColumnDefinition {
                    name: "doc_id".into(),
                    logical_type: kuzu_common::types::LogicalTypeID::Int64,
                    is_primary_key: true,
                },
                kuzu_storage::table::ColumnDefinition {
                    name: "text".into(),
                    logical_type: kuzu_common::types::LogicalTypeID::String,
                    is_primary_key: false,
                },
            ];
            self.table_catalog.create_node_table(self.docs_table.clone(), docs_cols);
        }
        if self.table_catalog.get_node_table_by_name(&self.terms_table).is_none() {
            let terms_cols = vec![
                kuzu_storage::table::ColumnDefinition {
                    name: "term_id".into(),
                    logical_type: kuzu_common::types::LogicalTypeID::Int64,
                    is_primary_key: true,
                },
                kuzu_storage::table::ColumnDefinition {
                    name: "term".into(),
                    logical_type: kuzu_common::types::LogicalTypeID::String,
                    is_primary_key: false,
                },
                kuzu_storage::table::ColumnDefinition {
                    name: "doc_freq".into(),
                    logical_type: kuzu_common::types::LogicalTypeID::Int64,
                    is_primary_key: false,
                },
            ];
            self.table_catalog
                .create_node_table(self.terms_table.clone(), terms_cols);
        }

        // Collect docs data
        let source_data = source_table.to_column_major_data();
        let num_rows = source_table.num_rows as usize;

        // term -> (term_id, doc_freq)
        let mut term_map: std::collections::HashMap<String, (i64, i64)> = std::collections::HashMap::new();
        // (doc_id, text) rows
        let mut doc_rows: Vec<Vec<Value>> = Vec::new();
        // posting: (term_id, doc_id, term_freq)
        let mut postings: Vec<(i64, i64, i64)> = Vec::new();

        for row_idx in 0..num_rows {
            let text = if let Some(col_data) = source_data.get(col_idx) {
                if let Some(Value::String(s)) = col_data.get(row_idx) {
                    s.clone()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let doc_id = row_idx as i64;
            doc_rows.push(vec![Value::Int64(doc_id), Value::String(text.clone())]);

            // Tokenize using kuzu-fts utilities
            let tokens = kuzu_fts::tokenize(&text);
            let mut freq_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
            for token in tokens {
                let stemmed = kuzu_fts::stem_word(&token);
                if !kuzu_fts::STOP_WORDS.contains(&stemmed.as_str()) {
                    *freq_map.entry(stemmed).or_insert(0) += 1;
                }
            }

            for (term, freq) in freq_map {
                let next_id = term_map.len() as i64;
                let (term_id, doc_freq) = term_map.entry(term).or_insert((next_id, 0));
                *doc_freq += 1;
                postings.push((*term_id, doc_id, freq));
            }
        }

        // Insert docs
        {
            let mut docs_table = self.table_catalog.get_node_table_by_name_mut(&self.docs_table).unwrap();
            for row in doc_rows {
                docs_table.insert_row(row)?;
            }
        }

        // Insert terms
        if self.table_catalog.get_node_table_by_name(&self.terms_table).is_some() {
            let mut terms_table = self
                .table_catalog
                .get_node_table_by_name_mut(&self.terms_table)
                .unwrap();
            let mut term_list: Vec<(String, i64, i64)> =
                term_map.into_iter().map(|(t, (id, df))| (t, id, df)).collect();
            term_list.sort_by_key(|(_, id, _)| *id);
            for (term, term_id, doc_freq) in term_list {
                terms_table.insert_row(vec![Value::Int64(term_id), Value::String(term), Value::Int64(doc_freq)])?;
            }
        }

        // Create and populate posting (appears_in) table
        let docs_table_id = self
            .table_catalog
            .get_node_table_by_name(&self.docs_table)
            .unwrap()
            .table_id;
        let terms_table_id = self
            .table_catalog
            .get_node_table_by_name(&self.terms_table)
            .unwrap()
            .table_id;

        if self.table_catalog.get_rel_table_by_name(&self.posting_table).is_none() {
            let posting_cols = vec![kuzu_storage::table::ColumnDefinition {
                name: "term_freq".into(),
                logical_type: kuzu_common::types::LogicalTypeID::Int64,
                is_primary_key: false,
            }];
            // FROM terms TO docs
            self.table_catalog.create_rel_table(
                self.posting_table.clone(),
                terms_table_id,
                docs_table_id,
                posting_cols,
            );
        }

        {
            let mut posting_table = self
                .table_catalog
                .get_rel_table_by_name_mut(&self.posting_table)
                .unwrap();
            for (term_id, doc_id, freq) in postings {
                posting_table.insert_rel(term_id as u64, doc_id as u64, vec![Value::Int64(freq)])?;
            }
        }

        let mut result_vec = kuzu_common::vector::ValueVector::new(kuzu_common::types::PhysicalTypeID::String, 1);
        result_vec.resize(1);
        result_vec
            .set_value(
                0,
                &Value::String(format!("FTS index '{}' built successfully.", self.index_name)),
            )
            .unwrap();
        let mut result = DataChunk::new(vec![result_vec]);
        result.size = 1;
        result.field_names = vec!["result".to_string()];
        Ok(vec![result])
    }
}

/// Physical operator for `USING FTS INDEX` scan — queries the 3 macro tables
/// and returns ranked (node_id, score) pairs using BM25 scoring.
#[derive(Debug, Clone)]
pub struct PhysicalFtsScan {
    pub index_name: String,
    pub query_string: String,
    pub docs_table: String,
    pub terms_table: String,
    pub posting_table: String,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalFtsScan {
    fn operator_type(&self) -> &str {
        "fts_scan"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // Tokenize query
        let query_tokens: Vec<String> = kuzu_fts::tokenize(&self.query_string)
            .into_iter()
            .map(|t| kuzu_fts::stem_word(&t))
            .filter(|t| !kuzu_fts::STOP_WORDS.contains(&t.as_str()))
            .collect();

        // Lookup terms table for matching terms
        let terms_table = match self.table_catalog.get_node_table_by_name(&self.terms_table) {
            Some(t) => t,
            None => {
                return Err(format!(
                    "FTS terms table '{}' not found. Has the index been created?",
                    self.terms_table
                ));
            }
        };

        // Get total doc count from docs table
        let num_docs = self
            .table_catalog
            .get_node_table_by_name(&self.docs_table)
            .map(|t| t.num_rows as f64)
            .unwrap_or(1.0);

        // Build map: term -> (term_id, doc_freq)
        let terms_data = terms_table.to_column_major_data();
        let num_terms = terms_table.num_rows as usize;
        let mut matching_terms: Vec<(i64, i64)> = Vec::new(); // (term_id, doc_freq)

        for row_idx in 0..num_terms {
            let term_val = terms_data.get(1).and_then(|d| d.get(row_idx));
            let term_str = if let Some(Value::String(s)) = term_val {
                s.clone()
            } else {
                continue;
            };
            if query_tokens.contains(&term_str) {
                let term_id = if let Some(Value::Int64(id)) = terms_data.first().and_then(|d| d.get(row_idx)) {
                    *id
                } else {
                    continue;
                };
                let doc_freq = if let Some(Value::Int64(df)) = terms_data.get(2).and_then(|d| d.get(row_idx)) {
                    *df
                } else {
                    0
                };
                matching_terms.push((term_id, doc_freq));
            }
        }

        // Accumulate per-doc BM25 scores from posting table
        let mut doc_scores: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();

        if let Some(posting_table) = self.table_catalog.get_rel_table_by_name(&self.posting_table) {
            for &(term_id, doc_freq) in &matching_terms {
                let idf = ((num_docs - doc_freq as f64 + 0.5) / (doc_freq as f64 + 0.5) + 1.0).ln();
                // Scan posting table for this term using get_outgoing_edges(term_id)
                let posting_rels = posting_table.get_outgoing_edges(term_id as u64);
                for (doc_id, rel_vals) in posting_rels {
                    let tf = if let Some(Value::Int64(freq)) = rel_vals.first() {
                        *freq as f64
                    } else {
                        1.0
                    };
                    // BM25: k1=1.5, b=0.75 (simplified, no avg doc len)
                    let k1 = 1.5_f64;
                    let score = idf * (tf * (k1 + 1.0)) / (tf + k1);
                    *doc_scores.entry(doc_id as i64).or_insert(0.0) += score;
                }
            }
        }

        // Sort by score descending
        let mut ranked: Vec<(i64, f64)> = doc_scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Return (doc_id, score) data chunks
        let n = ranked.len();
        let mut id_vec = kuzu_common::vector::ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, n);
        let mut score_vec = kuzu_common::vector::ValueVector::new(kuzu_common::types::PhysicalTypeID::Double, n);
        id_vec.resize(n);
        score_vec.resize(n);
        for (i, (doc_id, score)) in ranked.into_iter().enumerate() {
            id_vec.set_i64(i, doc_id);
            score_vec.set_double(i, score);
        }
        let mut chunk = DataChunk::new(vec![id_vec, score_vec]);
        chunk.size = n;
        chunk.field_names = vec!["doc_id".to_string(), "score".to_string()];
        Ok(vec![chunk])
    }
}

// ==================== PackedExtend ====================

/// Physical operator for multi-rel extend, producing packed columns.
///
/// Extends from a source node using a `CsrIndex` and produces a packed 
/// list of destination nodes for each source node.
pub struct PhysicalPackedExtend {
    pub rel_table_name: String,
    pub rel_table_id: u64,
    pub bound_node_var: String,
    pub direction: kuzu_parser::ast::EdgeDirection,
    pub dst_node_var: String,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalPackedExtend {
    fn operator_type(&self) -> &str {
        "packed_extend"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() || input.iter().all(|c| c.size == 0) {
            return Ok(input);
        }

        let rel_table = self
            .table_catalog
            .get_rel_table_by_name(&self.rel_table_name)
            .ok_or_else(|| format!("Rel table {} not found", self.rel_table_name))?;
            
        let mut output_chunks = Vec::new();

        for chunk in input {
            if chunk.size == 0 {
                continue;
            }

            // Find bound node column index
            let bound_idx = chunk
                .field_names
                .iter()
                .position(|name| name == &self.bound_node_var)
                .unwrap_or(0); // Fallback to 0

            let bound_field = &chunk.fields[bound_idx];
            
            // Create output chunk fields (copy input fields, append packed dst field)
            let mut new_fields = Vec::with_capacity(chunk.fields.len() + 1);
            for field in &chunk.fields {
                let mut new_v = kuzu_common::vector::ValueVector::new(field.physical_type(), chunk.size);
                new_v.resize(chunk.size);
                for i in 0..chunk.size {
                    if field.is_null(i) {
                        new_v.set_null(i, true);
                    } else if let Some(val) = field.get_value(i) {
                        crate::physical::common::store_value_in_vector(&mut new_v, i, &val);
                    }
                }
                new_fields.push(new_v);
            }
            
            // Output field for packed destination nodes (using String for now until List is fully supported)
            let mut dst_field = kuzu_common::vector::ValueVector::new(kuzu_common::types::PhysicalTypeID::String, chunk.size);
            dst_field.resize(chunk.size);
            
            for i in 0..chunk.size {
                if bound_field.is_null(i) {
                    dst_field.set_null(i, true);
                    continue;
                }
                
                let src_id = if let Some(kuzu_common::types::Value::Int64(id)) = bound_field.get_value(i) {
                    id as u64
                } else if let Some(kuzu_common::types::Value::UInt64(id)) = bound_field.get_value(i) {
                    id
                } else {
                    continue;
                };

                // Read from CsrIndex if available, otherwise fallback to adjacency list
                let neighbors = if let Some(csr) = &rel_table.csr_index {
                    csr.get_neighbors(src_id).unwrap_or_default()
                } else {
                    // Fallback to simple adjacency lookup
                    match self.direction {
                        kuzu_parser::ast::EdgeDirection::LeftToRight => {
                            rel_table.get_outgoing_edges(src_id).into_iter().map(|(dst, _)| dst).collect()
                        }
                        kuzu_parser::ast::EdgeDirection::RightToLeft => {
                            rel_table.get_incoming_edges(src_id).into_iter().map(|(dst, _)| dst).collect()
                        }
                        kuzu_parser::ast::EdgeDirection::Both => {
                            let mut n = rel_table.get_outgoing_edges(src_id).into_iter().map(|(dst, _)| dst).collect::<Vec<_>>();
                            n.extend(rel_table.get_incoming_edges(src_id).into_iter().map(|(dst, _)| dst));
                            n
                        }
                    }
                };

                // Serialize to string as packed representation
                let packed_str = format!("{:?}", neighbors);
                crate::physical::common::store_value_in_vector(&mut dst_field, i, &kuzu_common::types::Value::String(packed_str));
            }
            
            new_fields.push(dst_field);
            
            let mut new_names = chunk.field_names.clone();
            new_names.push(self.dst_node_var.clone());
            
            output_chunks.push(DataChunk {
                fields: new_fields,
                size: chunk.size,
                field_names: new_names,
            });
        }
        
        Ok(output_chunks)
    }
}
