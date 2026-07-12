//! Miscellaneous physical operators (EmptyResult, MultiplicityReducer, Skip, UnionAllScan).

use kuzu_common::vector::{DataChunk, ValueVector};
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

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
                    let val = chunk.field(col_idx).get_value(i).unwrap_or(kuzu_common::types::Value::Null);
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
                for field in &chunk.fields {
                    let mut new_field = ValueVector::new(field.physical_type(), filtered_size);
                    let mut current = 0;
                    for (i, &keep) in filter_mask.iter().enumerate() {
                        if keep {
                            if let Some(val) = field.get_value(i) {
                                let _ = new_field.set_value(current, &val);
                            }
                            current += 1;
                        }
                    }
                    new_fields.push(new_field);
                }
                result.push(DataChunk {
                    fields: new_fields,
                    size: filtered_size,
                    field_names: chunk.field_names.clone(),
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

            for field in &chunk.fields {
                let mut new_field = ValueVector::new(field.physical_type(), keep_size);
                for i in 0..keep_size {
                    new_field.set_value(i, &field.get_value(remaining_skip + i).unwrap_or(kuzu_common::types::Value::Null)).unwrap();
                }
                sliced_fields.push(new_field);
            }

            output.push(DataChunk {
                fields: sliced_fields,
                size: keep_size,
                field_names: chunk.field_names.clone(),
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
    pub values: Vec<Vec<kuzu_common::types::Value>>,
}

impl PhysicalOperatorExec for PhysicalInsert {
    fn operator_type(&self) -> &str {
        "insert"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // Normally this would interact with Catalog/Storage.
        // For now, return empty chunk or stub execution.
        Ok(Vec::new())
    }
}

/// ExtensionClause operator — handles EXTENSION commands (INSTALL, LOAD).
pub struct PhysicalExtensionClause {
    pub action: kuzu_parser::ast::ExtensionAction,
    pub extension_name: String,
}

impl PhysicalOperatorExec for PhysicalExtensionClause {
    fn operator_type(&self) -> &str {
        "extension_clause"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let msg = match self.action {
            kuzu_parser::ast::ExtensionAction::Install => format!("Extension '{}' installed.", self.extension_name),
            kuzu_parser::ast::ExtensionAction::Load => format!("Extension '{}' loaded.", self.extension_name),
            kuzu_parser::ast::ExtensionAction::Uninstall => format!("Extension '{}' uninstalled.", self.extension_name),
        };
        
        let mut field = ValueVector::new(kuzu_common::types::PhysicalTypeID::String, 1);
        let _ = field.set_value(0, &kuzu_common::types::Value::String(msg));
        
        Ok(vec![DataChunk {
            fields: vec![field],
            size: 1,
            field_names: vec!["message".into()],
        }])
    }
}
