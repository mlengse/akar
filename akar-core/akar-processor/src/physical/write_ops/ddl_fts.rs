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
        // Locate the source table and snapshot its schema + data. The DashMap
        // `Ref` MUST be dropped before the write locks below: creating/updating
        // the macro tables (create_node_table / get_*_mut) takes exclusive
        // shard locks, and DashMap is not re-entrant — holding the read `Ref`
        // while acquiring a write lock on the same shard self-deadlocks on this
        // thread. Because DashMap's hasher is random-seeded per catalog, the
        // shard collision is intermittent (the FTS test flake) (P53.x).
        let (col_idx, num_rows, source_data) = {
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
                col_idx,
                source_table.num_rows as usize,
                source_table.to_column_major_data(),
            )
        };

        // Ensure macro tables exist; create if needed
        if self.table_catalog.get_node_table_by_name(&self.docs_table).is_none() {
            let docs_cols = vec![
                akar_storage::table::ColumnDefinition {
                    name: "doc_id".into(),
                    logical_type: akar_common::types::LogicalTypeID::Int64,
                    is_primary_key: true,
                    compression: akar_common::enums::CompressionType::Uncompressed,
                },
                akar_storage::table::ColumnDefinition {
                    name: "text".into(),
                    logical_type: akar_common::types::LogicalTypeID::String,
                    is_primary_key: false,
                    compression: akar_common::enums::CompressionType::Uncompressed,
                },
            ];
            self.table_catalog.create_node_table(self.docs_table.clone(), docs_cols);
        }
        if self.table_catalog.get_node_table_by_name(&self.terms_table).is_none() {
            let terms_cols = vec![
                akar_storage::table::ColumnDefinition {
                    name: "term_id".into(),
                    logical_type: akar_common::types::LogicalTypeID::Int64,
                    is_primary_key: true,
                    compression: akar_common::enums::CompressionType::Uncompressed,
                },
                akar_storage::table::ColumnDefinition {
                    name: "term".into(),
                    logical_type: akar_common::types::LogicalTypeID::String,
                    is_primary_key: false,
                    compression: akar_common::enums::CompressionType::Uncompressed,
                },
                akar_storage::table::ColumnDefinition {
                    name: "doc_freq".into(),
                    logical_type: akar_common::types::LogicalTypeID::Int64,
                    is_primary_key: false,
                    compression: akar_common::enums::CompressionType::Uncompressed,
                },
            ];
            self.table_catalog
                .create_node_table(self.terms_table.clone(), terms_cols);
        }

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

            // Tokenize using Akar-fts utilities
            let tokens = akar_fts::tokenize(&text);
            let mut freq_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
            for token in tokens {
                let stemmed = akar_fts::stem_word(&token);
                if !akar_fts::STOP_WORDS.contains(&stemmed.as_str()) {
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
            let posting_cols = vec![akar_storage::table::ColumnDefinition {
                name: "term_freq".into(),
                logical_type: akar_common::types::LogicalTypeID::Int64,
                is_primary_key: false,
                compression: akar_common::enums::CompressionType::Uncompressed,
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

/// Physical operator for `USING FTS INDEX` scan — queries the 3 macro tables
/// and returns ranked (node_id, score) pairs using BM25 scoring.
#[derive(Debug, Clone)]
pub struct PhysicalFtsScan {
    pub index_name: String,
    pub query_string: String,
    pub docs_table: String,
    pub terms_table: String,
    pub posting_table: String,
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
        // Keep the derived index in sync with the source table first: any
        // rows inserted after CREATE FTS INDEX must be searchable (P52.39).
        self.sync_index_with_source()?;

        // Tokenize query
        let query_tokens: Vec<String> = akar_fts::tokenize(&self.query_string)
            .into_iter()
            .map(|t| akar_fts::stem_word(&t))
            .filter(|t| !akar_fts::STOP_WORDS.contains(&t.as_str()))
            .collect();

        // Lookup terms table for matching terms
        let terms_table = match self.table_catalog.get_node_table_by_name(&self.terms_table) {
            Some(t) => t,
            None => {
                return Err(format!(
                    "FTS terms table '{}' not found. Has the index been created?",
                    self.terms_table
                )
                .into());
            }
        };

        // Get total doc count from docs table
        let num_docs = self
            .table_catalog
            .get_node_table_by_name(&self.docs_table)
            .map(|t| t.num_rows as f64)
            .unwrap_or(1.0);

        // Build map: term -> (term_id, doc_freq). One pass over the vocabulary,
        // then O(1) lookups per query token — the old code scanned the whole
        // terms table per token (O(vocab x tokens), P52.39).
        let terms_data = terms_table.to_column_major_data();
        let num_terms = terms_table.num_rows as usize;
        let mut term_index: std::collections::HashMap<String, (i64, i64)> = std::collections::HashMap::new();
        for row_idx in 0..num_terms {
            let term_id = match terms_data.first().and_then(|d| d.get(row_idx)) {
                Some(Value::Int64(id)) => *id,
                _ => continue,
            };
            let term_str = match terms_data.get(1).and_then(|d| d.get(row_idx)) {
                Some(Value::String(s)) => s.clone(),
                _ => continue,
            };
            let doc_freq = match terms_data.get(2).and_then(|d| d.get(row_idx)) {
                Some(Value::Int64(df)) => *df,
                _ => 0,
            };
            term_index.insert(term_str, (term_id, doc_freq));
        }
        drop(terms_data);
        drop(terms_table);

        let mut matching_terms: Vec<(i64, i64)> = Vec::new(); // (term_id, doc_freq)
        for token in &query_tokens {
            if let Some(&(term_id, doc_freq)) = term_index.get(token.as_str()) {
                matching_terms.push((term_id, doc_freq));
            }
        }

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

        // Accumulate per-doc BM25 scores from posting table
        let mut doc_scores: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();

        if let Some(posting_table) = self.table_catalog.get_rel_table_by_name(&self.posting_table) {
            for &(term_id, doc_freq) in &matching_terms {
                let idf = ((num_docs - doc_freq as f64 + 0.5) / (doc_freq as f64 + 0.5) + 1.0).ln();
                // Scan posting table for this term using get_outgoing_edges(term_id)
                let posting_rels = posting_table.get_outgoing_edges(term_id as u64);
                for (doc_id, rel_vals) in posting_rels {
                    if !doc_valid(doc_id as i64) {
                        continue;
                    }
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
    /// Incrementally bring the derived FTS macro tables (docs/terms/postings)
    /// in line with the source node table (P52.39).
    ///
    /// Rows appended to the source after `CREATE FTS INDEX` are tokenized and
    /// added; existing terms get their `doc_freq` bumped. Rows that were
    /// soft-deleted in the source are simply not re-inserted, and the scoring
    /// pass filters them by source state, so no posting cleanup is required.
    fn sync_index_with_source(&self) -> Result<(), String> {
        let Some(source_table) = self.table_catalog.get_node_table_by_name(&self.table_name) else {
            return Ok(());
        };
        let Some(col_idx) = source_table.columns.iter().position(|c| c.name == self.column_name) else {
            return Ok(());
        };
        let source_count = source_table.num_rows as usize;

        let already_indexed = match self.table_catalog.get_node_table_by_name(&self.docs_table) {
            Some(docs) => docs.num_rows as usize,
            None => return Ok(()),
        };

        if already_indexed >= source_count {
            return Ok(());
        }

        // Collect the text of source rows not yet indexed.
        let mut new_docs: Vec<(usize, String)> = Vec::new();
        for row_id in already_indexed..source_count {
            if let Some(Value::String(s)) = source_table.get_value(row_id, col_idx) {
                new_docs.push((row_id, s.clone()));
            }
        }
        drop(source_table);

        if new_docs.is_empty() {
            return Ok(());
        }

        // term -> (term_id, doc_freq, terms-table row) from the current terms.
        let mut term_info: std::collections::HashMap<String, (i64, i64, usize)> = std::collections::HashMap::new();
        let mut max_term_id: i64 = -1;
        {
            let terms = self
                .table_catalog
                .get_node_table_by_name(&self.terms_table)
                .ok_or_else(|| format!("Terms table '{}' not found", self.terms_table))?;
            let data = terms.to_column_major_data();
            for row_idx in 0..terms.num_rows as usize {
                let term_id = match data.first().and_then(|d| d.get(row_idx)) {
                    Some(Value::Int64(id)) => *id,
                    _ => continue,
                };
                let term = match data.get(1).and_then(|d| d.get(row_idx)) {
                    Some(Value::String(s)) => s.clone(),
                    _ => continue,
                };
                let df = match data.get(2).and_then(|d| d.get(row_idx)) {
                    Some(Value::Int64(v)) => *v,
                    _ => 0,
                };
                max_term_id = max_term_id.max(term_id);
                term_info.insert(term, (term_id, df, row_idx));
            }
        }

        let mut next_term_id = max_term_id + 1;
        let mut new_postings: Vec<(i64, usize, i64)> = Vec::new();
        let mut new_terms: Vec<(i64, String)> = Vec::new();
        let mut df_updates: Vec<(usize, i64)> = Vec::new(); // (terms-table row, new doc_freq)

        for (doc_id, text) in &new_docs {
            let mut freq: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
            for token in akar_fts::tokenize(text) {
                let stemmed = akar_fts::stem_word(&token);
                if !akar_fts::STOP_WORDS.contains(&stemmed.as_str()) {
                    *freq.entry(stemmed).or_insert(0) += 1;
                }
            }
            for (term, f) in freq {
                if let Some((term_id, df, row_idx)) = term_info.get_mut(&term) {
                    *df += 1;
                    df_updates.push((*row_idx, *df));
                    new_postings.push((*term_id, *doc_id, f));
                } else {
                    let tid = next_term_id;
                    next_term_id += 1;
                    term_info.insert(term.clone(), (tid, 1, usize::MAX));
                    new_terms.push((tid, term));
                    new_postings.push((tid, *doc_id, f));
                }
            }
        }

        // Apply writes to the macro tables.
        {
            let mut docs = self
                .table_catalog
                .get_node_table_by_name_mut(&self.docs_table)
                .ok_or_else(|| format!("Docs table '{}' not found", self.docs_table))?;
            for (doc_id, text) in &new_docs {
                docs.insert_row(vec![Value::Int64(*doc_id as i64), Value::String(text.clone())])?;
            }
        }
        {
            let mut terms = self
                .table_catalog
                .get_node_table_by_name_mut(&self.terms_table)
                .ok_or_else(|| format!("Terms table '{}' not found", self.terms_table))?;
            for (term_id, term) in &new_terms {
                terms.insert_row(vec![
                    Value::Int64(*term_id),
                    Value::String(term.clone()),
                    Value::Int64(1),
                ])?;
            }
            for (row_idx, df) in df_updates {
                terms.update_cell(row_idx as u64, 2, Value::Int64(df))?;
            }
        }
        {
            let mut posting = self
                .table_catalog
                .get_rel_table_by_name_mut(&self.posting_table)
                .ok_or_else(|| format!("Posting table '{}' not found", self.posting_table))?;
            for (term_id, doc_id, f) in &new_postings {
                posting.insert_rel(*term_id as u64, *doc_id as u64, vec![Value::Int64(*f)])?;
            }
        }
        Ok(())
    }
}
