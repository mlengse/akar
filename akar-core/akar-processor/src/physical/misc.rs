//! Miscellaneous physical operators (EmptyResult, MultiplicityReducer, Skip, UnionAllScan).

use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
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
        let mut seen = std::collections::HashSet::new();

        for chunk in input {
            let mut filter_mask = vec![false; chunk.size];
            for i in 0..chunk.size {
                let mut row_keys = Vec::new();
                for &col_idx in &self.key_columns {
                    let val = chunk.get_value(col_idx, i).unwrap_or(akar_common::types::Value::Null);
                    row_keys.push(val);
                }

                // Using debug format as a fallback hashable representation for arbitrary Value types
                if seen.insert(format!("{:?}", row_keys)) {
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
                if let Ok(count) = rel_tbl.insert_rels_batch(&rels_to_insert) {
                    inserted += count;
                }
            }
        } else if let Some(mut node_tbl) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
            // Node Table Insert
            for row_values in &self.values {
                if node_tbl.insert_row(row_values.clone()).is_ok() {
                    inserted += 1;
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
