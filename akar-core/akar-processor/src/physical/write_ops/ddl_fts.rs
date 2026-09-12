//! Auto-extracted from physical_operator.rs
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};
use akar_storage::table::TableCatalog;
use std::sync::Arc;

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
        let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
        Ok(vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])])
    }
}

/// Physical operator for `CREATE FTS INDEX` — builds the **Tantivy** FTS index
/// over the source column (P104.1/P104.4).
///
/// The index persists as a Tantivy directory under `<db_path>/fts/<index_name>`
/// for disk-backed catalogs. The legacy `fts_{idx}_docs` / `fts_{idx}_terms` /
/// `fts_{idx}_appears_in` macro tables are **gone** (P104.2 clean break) — the
/// Tantivy directory is the *only* FTS representation.
pub struct PhysicalCreateFtsIndex {
    pub index_name: String,
    pub table_name: String,
    pub column_name: String,
    pub if_not_exists: bool,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalCreateFtsIndex {
    fn operator_type(&self) -> &str {
        "create_fts_index"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // Locate the source table and snapshot its schema + data. The DashMap
        // `Ref` MUST be dropped before any write lock below — DashMap is not
        // re-entrant, and holding a read `Ref` while acquiring a write lock on
        // the same shard self-deadlocks (the FTS test flake, P53.x).
        let (columns, col_idx, num_rows, source_data) = {
            let source_table = match self.table_catalog.get_node_table_by_name(&self.table_name) {
                Some(t) => t,
                None => return Err(format!("Table '{}' not found", self.table_name).into()),
            };
            let col_idx = source_table
                .columns
                .iter()
                .position(|c| c.name == self.column_name)
                .ok_or_else(|| format!("Column '{}' not found in '{}'", self.column_name, self.table_name))?;
            (
                source_table.columns.clone(),
                col_idx,
                source_table.num_rows as usize,
                source_table.to_column_major_data(),
            )
        };

        // Materialize the (doc_id, text) rows from the source snapshot.
        let mut rows: Vec<(i64, String)> = Vec::with_capacity(num_rows);
        for row_idx in 0..num_rows {
            let text = source_data
                .get(col_idx)
                .and_then(|col| col.get(row_idx))
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            rows.push((row_idx as i64, text));
        }

        // Build the Tantivy index (P104.1) — the only FTS representation
        // (P104.2 clean break). Disk-backed catalogs persist under
        // `<db_path>/fts/<index_name>`; in-memory ones build an ephemeral index
        // (searching it is not supported and errors clearly).
        let index_dir = self
            .table_catalog
            .db_path()
            .filter(|p| p.to_string_lossy() != ":memory:")
            .map(|p| p.join("fts").join(&self.index_name));
        akar_fts::build::build_index(&columns, &self.column_name, index_dir.as_deref(), &rows)?;

        let mut result_vec = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::String, 1);
        result_vec.resize(1);
        result_vec
            .set_value(
                0,
                &Value::String(format!("FTS index '{}' built successfully.", self.index_name)),
            )
            .unwrap();
        let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&result_vec).array;
        let mut result = DataChunk::new(vec![arr], vec![akar_common::types::PhysicalTypeID::String]);
        result.size = 1;
        result.field_names = vec!["result".to_string()];
        Ok(vec![result])
    }
}

/// Physical operator for `USING FTS INDEX` scan — queries the **Tantivy**
/// index and returns ranked (doc_id, score) pairs (P104.2 clean break).
#[derive(Debug, Clone)]
pub struct PhysicalFtsScan {
    pub index_name: String,
    pub query_string: String,
    /// Source node table/column the index was created on (P52.39) — used to
    /// catch up newly inserted rows and filter deleted ones at query time.
    pub table_name: String,
    pub column_name: String,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalFtsScan {
    fn operator_type(&self) -> &str {
        "fts_scan"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let index_dir = self.index_dir().ok_or_else(|| {
            "FTS scan requires a disk-backed database (the FTS index lives on disk; in-memory DBs are not supported — P104.2)"
                .to_string()
        })?;

        let columns = {
            let source_table = self
                .table_catalog
                .get_node_table_by_name(&self.table_name)
                .ok_or_else(|| format!("Table '{}' not found", self.table_name))?;
            source_table.columns.clone()
        };

        // Catch up rows appended after CREATE FTS INDEX (P52.39) — now an
        // incremental Tantivy write (P105.3), no macro tables to rebuild.
        let (_index, reader) = self.catch_up(&index_dir, &columns)?;

        // Parse and run the query against the Tantivy searcher (P105.1).
        let searcher = reader.searcher();
        let search_schema = searcher.schema();
        let text_field = search_schema.get_field(&self.column_name).map_err(|_| {
            format!(
                "FTS: column '{}' not found in index '{}'",
                self.column_name, self.index_name
            )
        })?;
        let doc_id_field = search_schema
            .get_field(akar_fts::schema::DOC_ID_FIELD)
            .map_err(|_| "FTS: internal doc_id field missing".to_string())?;

        let limit = searcher.num_docs() as usize;
        let hits = akar_fts::index::TantivyIndex::search_doc_ids(
            &reader,
            &self.query_string,
            vec![text_field],
            doc_id_field,
            limit,
        )
        .map_err(|e| format!("FTS: search '{}': {e}", self.query_string))?;

        // Doc validity: a doc is searchable only while its source row still
        // exists and its text column is non-NULL (soft-deleted rows are
        // filtered out, P52.39).
        let source_table = self.table_catalog.get_node_table_by_name(&self.table_name);
        let source_col = source_table
            .as_ref()
            .and_then(|t| t.columns.iter().position(|c| c.name == self.column_name));
        let doc_valid = |doc_id: i64| -> bool {
            let Ok(r) = usize::try_from(doc_id) else {
                return false;
            };
            match (&source_table, source_col) {
                (Some(t), Some(ci)) => r < t.num_rows as usize && matches!(t.get_value(r, ci), Some(Value::String(_))),
                _ => true, // no source info → keep everything
            }
        };

        // Tantivy BM25 (k1=1.2, b=0.75) ranks by descending relevance (P106.1
        // verifies parity); keep the doc_id (source row index) contract intact.
        let mut ranked: Vec<(i64, f64)> = Vec::with_capacity(hits.len());
        for (doc_id, score) in hits {
            if doc_valid(doc_id) {
                ranked.push((doc_id, score as f64));
            }
        }

        // Return (doc_id, score) data chunks
        let n = ranked.len();
        let mut id_vec = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, n);
        let mut score_vec = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Double, n);
        id_vec.resize(n);
        score_vec.resize(n);
        for (i, (doc_id, score)) in ranked.into_iter().enumerate() {
            id_vec.set_i64(i, doc_id);
            score_vec.set_double(i, score);
        }
        let arr1 = akar_common::arrow_vector::ArrowVector::from_legacy(&id_vec).array;
        let arr2 = akar_common::arrow_vector::ArrowVector::from_legacy(&score_vec).array;
        let mut chunk = DataChunk::new(
            vec![arr1, arr2],
            vec![
                akar_common::types::PhysicalTypeID::Int64,
                akar_common::types::PhysicalTypeID::Double,
            ],
        );
        chunk.size = n;
        chunk.field_names = vec!["doc_id".to_string(), "score".to_string()];
        Ok(vec![chunk])
    }
}

impl PhysicalFtsScan {
    /// On-disk location of this index's Tantivy directory
    /// (`<db_path>/fts/<index_name>`).
    fn index_dir(&self) -> Option<std::path::PathBuf> {
        self.table_catalog
            .db_path()
            .filter(|p| p.to_string_lossy() != ":memory:")
            .map(|p| p.join("fts").join(&self.index_name))
    }

    /// Incrementally bring the Tantivy index in line with the source node
    /// table (P52.39, re-implemented on the Tantivy writer — P105.3).
    ///
    /// Rows appended to the source after `CREATE FTS INDEX` are added via
    /// [`akar_fts::build::append_docs`] and committed once. Soft-deleted rows
    /// are simply not re-inserted, and the scoring pass filters them by source
    /// state, so no deletion propagation is required.
    fn catch_up(
        &self,
        index_dir: &std::path::Path,
        columns: &[akar_storage::table::ColumnDefinition],
    ) -> Result<(akar_fts::index::TantivyIndex, akar_fts::index::IndexReader), String> {
        if !index_dir.join("meta.json").exists() {
            return Err(format!(
                "FTS index '{}' not found on disk at '{}' (run CREATE FTS INDEX first)",
                self.index_name,
                index_dir.display()
            ));
        }

        let schema = akar_fts::schema::build_index_schema(columns);
        let index = akar_fts::index::TantivyIndex::create_on_disk(index_dir, schema)
            .map_err(|e| format!("FTS: open index '{}': {e}", self.index_name))?;
        let reader = index.reader().map_err(|e| format!("FTS: reader: {e}"))?;
        reader.reload().map_err(|e| format!("FTS: reload: {e}"))?;

        let indexed_count = reader.searcher().num_docs() as usize;

        // Snapshot the source rows not yet indexed. The read `Ref` is scoped
        // out before the Tantivy writer is opened (DashMap not re-entrant).
        let new_docs: Vec<(i64, String)> = {
            let source_table = match self.table_catalog.get_node_table_by_name(&self.table_name) {
                Some(t) => t,
                None => return Ok((index, reader)),
            };
            let Some(col_idx) = source_table.columns.iter().position(|c| c.name == self.column_name) else {
                return Ok((index, reader));
            };
            let source_count = source_table.num_rows as usize;
            let mut docs = Vec::new();
            for row_id in indexed_count..source_count {
                if let Some(Value::String(s)) = source_table.get_value(row_id, col_idx) {
                    docs.push((row_id as i64, s.clone()));
                }
            }
            docs
        };

        if !new_docs.is_empty() {
            akar_fts::build::append_docs(&index, columns, &self.column_name, &new_docs)?;
            reader
                .reload()
                .map_err(|e| format!("FTS: reload after catch-up: {e}"))?;
        }

        Ok((index, reader))
    }
}
