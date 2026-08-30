//! Miscellaneous physical operators (EmptyResult, MultiplicityReducer, Skip, UnionAllScan).

use crate::physical::common::hash_row;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::Value;
use akar_common::vector::{DataChunk, ValueVector};

/// Physical operator that always returns an empty result.
pub struct PhysicalEmptyResult;

impl PhysicalOperatorExec for PhysicalEmptyResult {
    fn operator_type(&self) -> &str {
        "empty_result"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        Ok(vec![])
    }
}

/// Physical operator that reduces the multiplicity of paths (e.g., DISTINCT).
pub struct PhysicalMultiplicityReducer {
    pub key_columns: Vec<usize>,
}

impl PhysicalOperatorExec for PhysicalMultiplicityReducer {
    fn operator_type(&self) -> &str {
        "multiplicity_reducer"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() {
            return Ok(input);
        }

        let mut result = Vec::new();
        // Hash-bucket membership index over kept keys: hash -> indices into
        // `kept_keys`. Exact row equality is checked only on hash collision,
        // so two distinct rows sharing a hash are never wrongly merged. This
        // replaces the old per-row `format!("{:?}", row_keys)` allocation +
        // `HashSet<String>` dedup (O(1) per row, deterministic equality).
        let mut buckets: std::collections::HashMap<u64, Vec<usize>> = std::collections::HashMap::new();
        let mut kept_keys: Vec<Vec<Value>> = Vec::new();

        for chunk in input {
            let mut filter_mask = vec![false; chunk.size];
            for i in 0..chunk.size {
                let mut row_keys = Vec::with_capacity(self.key_columns.len());
                for &col_idx in &self.key_columns {
                    let val = chunk.get_value(col_idx, i).unwrap_or(Value::Null);
                    row_keys.push(val);
                }

                let hash = hash_row(&row_keys);
                let is_dup = buckets
                    .get(&hash)
                    .is_some_and(|bucket| bucket.iter().any(|&k| kept_keys[k] == row_keys));
                if !is_dup {
                    match buckets.get_mut(&hash) {
                        Some(bucket) => bucket.push(kept_keys.len()),
                        None => {
                            buckets.insert(hash, vec![kept_keys.len()]);
                        }
                    }
                    kept_keys.push(row_keys);
                    filter_mask[i] = true;
                }
            }

            let filtered_size = filter_mask.iter().filter(|&&b| b).count();
            if filtered_size > 0 {
                let mut new_fields = Vec::new();
                let mut new_field_types = Vec::new();
                for (col_idx, _field) in chunk.fields.iter().enumerate() {
                    let phys_type = chunk.field_types[col_idx];
                    let mut new_field = ValueVector::new(phys_type, filtered_size);
                    let mut current = 0;
                    for (i, &keep) in filter_mask.iter().enumerate() {
                        if keep {
                            if let Some(val) = chunk.get_value(col_idx, i) {
                                let _ = new_field.set_value(current, &val);
                            }
                            current += 1;
                        }
                    }
                    new_fields.push(akar_common::arrow_vector::ArrowVector::from_legacy(&new_field).array);
                    new_field_types.push(phys_type);
                }
                result.push(DataChunk {
                    fields: new_fields,
                    field_types: new_field_types,
                    size: filtered_size,
                    field_names: chunk.field_names.clone(),
                    sel_vector: None,
                });
            }
        }
        Ok(result)
    }
}

/// Physical operator for SKIP (OFFSET) in queries.
pub struct PhysicalSkip {
    pub skip_count: usize,
}

impl PhysicalOperatorExec for PhysicalSkip {
    fn operator_type(&self) -> &str {
        "skip"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let mut remaining_skip = self.skip_count;
        let mut output = Vec::new();

        for chunk in input {
            if remaining_skip == 0 {
                output.push(chunk);
                continue;
            }

            if chunk.size <= remaining_skip {
                remaining_skip -= chunk.size;
                continue;
            }

            // Partially skip this chunk
            let keep_size = chunk.size - remaining_skip;
            let mut sliced_fields = Vec::new();
            let mut sliced_types = Vec::new();

            for (col_idx, _field) in chunk.fields.iter().enumerate() {
                let phys_type = chunk.field_types[col_idx];
                let mut new_field = ValueVector::new(phys_type, keep_size);
                for i in 0..keep_size {
                    let val = chunk
                        .get_value(col_idx, remaining_skip + i)
                        .unwrap_or(akar_common::types::Value::Null);
                    // set_value returns Err only for strings > 255 bytes (legacy
                    // inline storage limit); drop to NULL instead of panicking.
                    if new_field.set_value(i, &val).is_err() {
                        new_field.set_null(i, true);
                    }
                }
                sliced_fields.push(akar_common::arrow_vector::ArrowVector::from_legacy(&new_field).array);
                sliced_types.push(phys_type);
            }

            output.push(DataChunk {
                fields: sliced_fields,
                field_types: sliced_types,
                size: keep_size,
                field_names: chunk.field_names.clone(),
                sel_vector: None,
            });
            remaining_skip = 0;
        }

        Ok(output)
    }
}

/// Physical operator for scanning from a UNION ALL.
pub struct PhysicalUnionAllScan;

impl PhysicalOperatorExec for PhysicalUnionAllScan {
    fn operator_type(&self) -> &str {
        "union_all_scan"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        Ok(input)
    }
}

/// Insert operator — row-level insertion (unlike BatchInsert).
pub struct PhysicalInsert {
    pub table_name: String,
    pub table_id: u64,
    pub columns: Vec<String>,
    pub values: Vec<Vec<akar_common::types::Value>>,
    pub table_catalog: std::sync::Arc<akar_storage::table::TableCatalog>,
    /// Active transaction id (P52.18).
    pub txn_id: Option<u64>,
    /// Undo sink for rollback records (P52.18).
    pub undo_sink: Option<std::sync::Arc<std::sync::Mutex<Vec<akar_transaction::UndoRecord>>>>,
    /// Typed WAL sink so inserted rows/edges survive restarts via replay (P60.2).
    pub wal_sink: Option<akar_storage::wal::WalSink>,
}

impl PhysicalOperatorExec for PhysicalInsert {
    fn operator_type(&self) -> &str {
        "insert"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let mut inserted = 0;

        // Cek jika tabel adalah rel table atau node table
        if let Some(mut rel_tbl) = self.table_catalog.get_rel_table_by_name_mut(&self.table_name) {
            // Rel Table Insert
            let mut rels_to_insert = Vec::new();
            for row_values in &self.values {
                if row_values.len() >= 2 {
                    // Extract src and dst
                    let src = if let akar_common::types::Value::Int64(v) = row_values[0] {
                        v as u64
                    } else {
                        0
                    };
                    let dst = if let akar_common::types::Value::Int64(v) = row_values[1] {
                        v as u64
                    } else {
                        0
                    };
                    let props = if row_values.len() > 2 {
                        row_values[2..].to_vec()
                    } else {
                        vec![]
                    };
                    rels_to_insert.push((src, dst, props));
                }
            }
            if !rels_to_insert.is_empty() {
                let start = rel_tbl.edges.len();
                if let Ok(count) = rel_tbl.insert_rels_batch(&rels_to_insert) {
                    inserted += count;
                    for (src, dst, props) in &rels_to_insert {
                        akar_storage::wal::log_rel_insert_record(&self.wal_sink, self.table_id, *src, *dst, props);
                    }
                    if let Some(sink) = self.undo_sink.as_ref()
                        && let Ok(mut u) = sink.lock()
                    {
                        for idx in start..start + count as usize {
                            u.push(akar_transaction::UndoRecord::insert(self.table_id, idx as u64));
                        }
                    }
                }
            }
        } else if let Some(mut node_tbl) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
            // Node Table Insert
            for row_values in &self.values {
                if let Ok(row_id) = node_tbl.insert_row_with_txn(row_values.clone(), self.txn_id) {
                    inserted += 1;
                    akar_storage::wal::log_insert_record(&self.wal_sink, self.table_id, row_values);
                    if let Some(sink) = self.undo_sink.as_ref()
                        && let Ok(mut u) = sink.lock()
                    {
                        u.push(akar_transaction::UndoRecord::insert(self.table_id, row_id));
                    }
                }
            }
        } else {
            return Err(format!("Table '{}' not found for INSERT", self.table_name).into());
        }

        let mut v = ValueVector::new(akar_common::types::PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, inserted as i64);
        let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
        Ok(vec![DataChunk::new(
            vec![arr],
            vec![akar_common::types::PhysicalTypeID::Int64],
        )])
    }
}

/// ExtensionClause operator — handles EXTENSION commands (INSTALL, LOAD).
pub struct PhysicalExtensionClause {
    pub action: akar_parser::ast::ExtensionAction,
    pub extension_name: String,
}

impl PhysicalOperatorExec for PhysicalExtensionClause {
    fn operator_type(&self) -> &str {
        "extension_clause"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let msg = match self.action {
            akar_parser::ast::ExtensionAction::Install => {
                format!("Extension '{}' installed successfully.", self.extension_name)
            }
            akar_parser::ast::ExtensionAction::Load => {
                // Pseudo-registry for static extensions
                if self.extension_name.to_lowercase() == "httpfs" {
                    format!(
                        "Extension '{}' loaded (HTTP/S3 virtual file system).",
                        self.extension_name
                    )
                } else if self.extension_name.to_lowercase() == "fts" {
                    format!("Extension '{}' loaded (Full Text Search).", self.extension_name)
                } else {
                    format!("Extension '{}' loaded.", self.extension_name)
                }
            }
            akar_parser::ast::ExtensionAction::Uninstall => {
                format!("Extension '{}' uninstalled.", self.extension_name)
            }
        };

        tracing::info!("{}", msg);

        let mut field = ValueVector::new(akar_common::types::PhysicalTypeID::String, 1);
        let _ = field.set_value(0, &akar_common::types::Value::String(msg));
        let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&field).array;

        Ok(vec![DataChunk {
            fields: vec![arr],
            field_types: vec![akar_common::types::PhysicalTypeID::String],
            size: 1,
            field_names: vec!["message".into()],
            sel_vector: None,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_common::types::PhysicalTypeID;

    fn make_chunk(cols: &[Vec<Value>], names: &[&str]) -> DataChunk {
        let mut fields = Vec::with_capacity(cols.len());
        let mut types = Vec::with_capacity(cols.len());
        for col in cols {
            let first = col
                .iter()
                .find(|v| !matches!(v, Value::Null))
                .unwrap_or(&Value::Int64(0));
            let ptype = match first {
                Value::Int64(_) => PhysicalTypeID::Int64,
                Value::Double(_) => PhysicalTypeID::Double,
                Value::Float(_) => PhysicalTypeID::Float,
                Value::Bool(_) => PhysicalTypeID::Bool,
                Value::String(_) => PhysicalTypeID::String,
                _ => PhysicalTypeID::Int64,
            };
            let mut v = ValueVector::new(ptype, col.len().max(1));
            for (i, val) in col.iter().enumerate() {
                let _ = v.set_value(i, val);
            }
            v.resize(col.len());
            fields.push(akar_common::arrow_vector::ArrowVector::from_legacy(&v).array);
            types.push(ptype);
        }
        let mut chunk = DataChunk::new(fields, types);
        chunk.field_names = names.iter().map(|s| s.to_string()).collect();
        chunk
    }

    fn reducer(key_columns: Vec<usize>) -> PhysicalMultiplicityReducer {
        PhysicalMultiplicityReducer { key_columns }
    }

    #[test]
    fn test_multiplicity_reducer_empty_input_returns_empty() {
        let out = reducer(vec![0]).execute(Vec::new()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn test_multiplicity_reducer_dedup_across_chunks() {
        // The same key appearing in a later chunk must be dropped (the seen set
        // persists across all input chunks); first-seen order is preserved.
        let c1 = make_chunk(&[vec![Value::Int64(1), Value::Int64(2)]], &["x"]);
        let c2 = make_chunk(&[vec![Value::Int64(2), Value::Int64(3)]], &["x"]);
        let out = reducer(vec![0]).execute(vec![c1, c2]).unwrap();
        assert_eq!(out.len(), 2, "one output chunk per non-empty filtered chunk");
        assert_eq!(out[0].size, 2);
        assert_eq!(out[1].size, 1, "row 2 is a duplicate of c1's row 2");
        assert_eq!(out[0].get_value(0, 0), Some(Value::Int64(1)));
        assert_eq!(out[0].get_value(0, 1), Some(Value::Int64(2)));
        assert_eq!(out[1].get_value(0, 0), Some(Value::Int64(3)));
    }

    #[test]
    fn test_multiplicity_reducer_key_columns_subset() {
        // Only key_columns participate in the dedup key: second row shares its
        // key with the first so it is dropped even though col 1 differs.
        let chunk = make_chunk(
            &[
                vec![Value::Int64(1), Value::Int64(1)],
                vec![Value::String("a".into()), Value::String("b".into())],
            ],
            &["id", "tag"],
        );
        let out = reducer(vec![0]).execute(vec![chunk]).unwrap();
        assert_eq!(out[0].size, 1);
        assert_eq!(out[0].get_value(0, 0), Some(Value::Int64(1)));
        assert_eq!(out[0].get_value(1, 0), Some(Value::String("a".into())));
    }

    #[test]
    fn test_multiplicity_reducer_multicolumn_key() {
        // The key is the whole tuple of key_columns, not the first one alone:
        // rows sharing col 0 but differing in col 1 are distinct.
        let chunk = make_chunk(
            &[
                vec![Value::Int64(1), Value::Int64(1), Value::Int64(1)],
                vec![
                    Value::String("a".into()),
                    Value::String("b".into()),
                    Value::String("a".into()),
                ],
            ],
            &["id", "tag"],
        );
        let out = reducer(vec![0, 1]).execute(vec![chunk]).unwrap();
        assert_eq!(out[0].size, 2, "(1,a) and (1,b) kept; (1,a) repeated dropped");
        assert_eq!(out[0].get_value(0, 0), Some(Value::Int64(1)));
        assert_eq!(out[0].get_value(1, 0), Some(Value::String("a".into())));
        assert_eq!(out[0].get_value(1, 1), Some(Value::String("b".into())));
    }

    #[test]
    fn test_multiplicity_reducer_nan_not_deduped() {
        // IEEE equality: NaN != NaN, so two NaN keys are both kept. The old
        // string-format key would have merged them (debug "NaN" == "NaN").
        let chunk = make_chunk(&[vec![Value::Double(f64::NAN), Value::Double(f64::NAN)]], &["x"]);
        let out = reducer(vec![0]).execute(vec![chunk]).unwrap();
        assert_eq!(out[0].size, 2);
    }
}
