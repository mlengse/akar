//! Auto-extracted from physical_operator.rs
use crate::physical::scan_filter::PhysicalFilter;
use kuzu_common::types::{LogicalTypeID, PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_parser::ast::Expression;
use kuzu_storage::table::ColumnDefinition;
use std::sync::{Arc, Mutex};
use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical::types::{OperatorResult, NodeSemiMask, PhysicalOperatorExec};
use crate::physical::write_ops::PhysicalFtsScan;

// ==================== Scan ====================

#[derive(Debug, Clone)]
pub struct PhysicalScan {
    pub table_name: String,
    pub table_id: u64,
    pub column_ids: Vec<u32>,
    pub estimated_cardinality: u64,
    /// Column-major table data: data[col_idx][row_idx].
    /// When present, PhysicalScan reads actual data instead of generating synthetic values.
    pub table_data: Option<Vec<Vec<Value>>>,
    /// Column definitions to map column names to physical types.
    pub table_columns: Vec<ColumnDefinition>,
    /// Optional semi-mask for SIP optimization. If present, only rows whose
    /// internal node ID offset is in the mask will be emitted.
    pub semi_mask: Option<NodeSemiMask>,
    /// Column index of the internal ID field to test against the mask.
    /// Only used when `semi_mask` is `Some`.
    pub mask_id_column: usize,
    pub fts_query: Option<PhysicalFtsScan>,
    pub predicate: Option<Expression>,
    pub evaluator: Option<Arc<Mutex<ExpressionEvaluator>>>,
}

impl PhysicalScan {
    pub fn new(table_name: String, table_id: u64, estimated_cardinality: u64) -> Self {
        Self {
            table_name,
            table_id,
            column_ids: Vec::new(),
            estimated_cardinality,
            table_data: None,
            table_columns: Vec::new(),
            semi_mask: None,
            mask_id_column: 0,
            fts_query: None,
            predicate: None,
            evaluator: None,
        }
    }

    /// Set column IDs to scan. These map to column indices in the table data.
    pub fn with_columns(mut self, column_ids: Vec<u32>) -> Self {
        self.column_ids = column_ids;
        self
    }

    /// Attach table data for the scan to read from.
    pub fn with_data(mut self, data: Vec<Vec<Value>>, columns: Vec<ColumnDefinition>) -> Self {
        self.table_data = Some(data);
        self.table_columns = columns;
        self
    }

    /// Attach a semi-mask for SIP optimization.
    /// When set, only rows whose internal node ID at `mask_id_column` is in the mask will be emitted.
    pub fn with_semi_mask(mut self, mask: NodeSemiMask, mask_id_column: usize) -> Self {
        self.semi_mask = Some(mask);
        self.mask_id_column = mask_id_column;
        self
    }

    pub fn with_fts_query(mut self, fts_query: PhysicalFtsScan) -> Self {
        self.fts_query = Some(fts_query);
        self
    }

    pub fn with_predicate(mut self, predicate: Expression) -> Self {
        self.predicate = Some(predicate);
        self
    }

    pub fn with_evaluator(mut self, evaluator: Arc<Mutex<ExpressionEvaluator>>) -> Self {
        self.evaluator = Some(evaluator);
        self
    }

    /// Convert a Value to bytes in a ValueVector at the given row index.
    fn write_value_to_vector(v: &mut ValueVector, row: usize, val: &Value) {
        match val {
            Value::Null => {
                v.set_null(row, true);
            }
            Value::Bool(x) => {
                if v.physical_type() == PhysicalTypeID::Bool {
                    v.data_mut()[row] = if *x { 1 } else { 0 };
                    v.set_null(row, false);
                }
            }
            Value::Int64(x) => {
                let offset = row * 8;
                if offset + 8 <= v.data().len() {
                    v.data_mut()[offset..offset + 8].copy_from_slice(&x.to_le_bytes());
                    v.set_null(row, false);
                }
            }
            Value::Int32(x) => {
                let offset = row * 4;
                if offset + 4 <= v.data().len() {
                    v.data_mut()[offset..offset + 4].copy_from_slice(&x.to_le_bytes());
                    v.set_null(row, false);
                }
            }
            Value::Double(x) => {
                let offset = row * 8;
                if offset + 8 <= v.data().len() {
                    v.data_mut()[offset..offset + 8].copy_from_slice(&x.to_le_bytes());
                    v.set_null(row, false);
                }
            }
            Value::Float(x) => {
                let offset = row * 4;
                if offset + 4 <= v.data().len() {
                    v.data_mut()[offset..offset + 4].copy_from_slice(&x.to_le_bytes());
                    v.set_null(row, false);
                }
            }
            Value::String(s) => {
                let offset = row * 256;
                let bytes = s.as_bytes();
                let len = bytes.len().min(255) as u8;
                if offset < v.data().len() {
                    v.data_mut()[offset] = len;
                    let copy_len = bytes.len().min(255);
                    if offset + 1 + copy_len <= v.data().len() {
                        v.data_mut()[offset + 1..offset + 1 + copy_len].copy_from_slice(&bytes[..copy_len]);
                    }
                    v.set_null(row, false);
                }
            }
            _ => {
                // For complex types, set null
                v.set_null(row, true);
            }
        }
    }

    /// Determine the PhysicalTypeID for a Value, with fallback.
    pub(crate) fn value_to_physical_type(val: &Value) -> PhysicalTypeID {
        match val {
            Value::Null => PhysicalTypeID::Int64,
            Value::Bool(_) => PhysicalTypeID::Bool,
            Value::Int64(_) | Value::UInt64(_) | Value::Int128(_) => PhysicalTypeID::Int64,
            Value::Int32(_) | Value::UInt32(_) => PhysicalTypeID::Int32,
            Value::Int16(_) | Value::UInt16(_) => PhysicalTypeID::Int16,
            Value::Int8(_) | Value::UInt8(_) => PhysicalTypeID::Int8,
            Value::Double(_) => PhysicalTypeID::Double,
            Value::Float(_) => PhysicalTypeID::Float,
            Value::String(_)
            | Value::Date(_)
            | Value::Timestamp(_)
            | Value::TimestampTz(_)
            | Value::TimestampNs(_)
            | Value::TimestampMs(_)
            | Value::TimestampSec(_)
            | Value::Interval(_) => PhysicalTypeID::String,
            Value::Blob(_) => PhysicalTypeID::Blob,
            Value::UInt128(_) => PhysicalTypeID::Int128,
            Value::Json(_) => PhysicalTypeID::String,
            Value::DTime(_) => PhysicalTypeID::Int64,
            Value::Union(_, _) => PhysicalTypeID::Struct,
            Value::InternalID(_) | Value::List(_) | Value::Map(_) | Value::Struct(_) => PhysicalTypeID::Int64,
        }
    }

    /// Determine PhysicalTypeID from a LogicalTypeID.
    pub(crate) fn logical_to_physical(logical: &LogicalTypeID) -> PhysicalTypeID {
        match logical {
            LogicalTypeID::Bool => PhysicalTypeID::Bool,
            LogicalTypeID::Int64 | LogicalTypeID::UInt64 | LogicalTypeID::Int128 | LogicalTypeID::Serial | LogicalTypeID::UInt128 => {
                PhysicalTypeID::Int64
            }
            LogicalTypeID::Int32 | LogicalTypeID::UInt32 => PhysicalTypeID::Int32,
            LogicalTypeID::Int16 | LogicalTypeID::UInt16 => PhysicalTypeID::Int16,
            LogicalTypeID::Int8 | LogicalTypeID::UInt8 => PhysicalTypeID::Int8,
            LogicalTypeID::Double | LogicalTypeID::Decimal => PhysicalTypeID::Double,
            LogicalTypeID::Float => PhysicalTypeID::Float,
            LogicalTypeID::String
            | LogicalTypeID::Date
            | LogicalTypeID::Timestamp
            | LogicalTypeID::TimestampTz
            | LogicalTypeID::TimestampMs
            | LogicalTypeID::TimestampNs
            | LogicalTypeID::TimestampSec
            | LogicalTypeID::Interval
            | LogicalTypeID::Time
            | LogicalTypeID::Json => PhysicalTypeID::String,
            LogicalTypeID::Blob => PhysicalTypeID::Blob,
            LogicalTypeID::Any
            | LogicalTypeID::Node
            | LogicalTypeID::Rel
            | LogicalTypeID::RecursiveRel
            | LogicalTypeID::List
            | LogicalTypeID::Array
            | LogicalTypeID::Map
            | LogicalTypeID::Struct
            | LogicalTypeID::Union
            | LogicalTypeID::Uuid
            | LogicalTypeID::InternalID => PhysicalTypeID::Int64,
        }
    }
}

impl PhysicalOperatorExec for PhysicalScan {
    fn operator_type(&self) -> &str {
        "scan"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // If we have real table data, read from it
        if let Some(ref data) = self.table_data {
            if data.is_empty() || data[0].is_empty() {
                return Ok(vec![DataChunk::new(vec![])]);
            }

            let num_rows = data[0].len();

            // If FTS query is active, run it to get matching row offsets (doc_ids) in order of relevance
            let mut fts_doc_ids = None;
            if let Some(ref fts) = self.fts_query {
                let fts_chunks = fts.execute(vec![])?;
                let mut doc_ids = Vec::new();
                if let Some(chunk) = fts_chunks.first() {
                    if let Some(id_vec) = chunk.fields.first() {
                        for row in 0..chunk.size {
                            if let Some(doc_id) = id_vec.get_i64(row) {
                                doc_ids.push(doc_id);
                            }
                        }
                    }
                }
                fts_doc_ids = Some(doc_ids);
            }

            // Build a row inclusion mask if semi_mask is active
            let row_filter: Option<Vec<bool>> = if let Some(ref mask) = self.semi_mask {
                if self.mask_id_column < data.len() {
                    Some(
                        (0..num_rows)
                            .map(|row| {
                                if let Value::InternalID(id) = &data[self.mask_id_column][row] {
                                    mask.is_masked(id.offset)
                                } else {
                                    true
                                }
                            })
                            .collect(),
                    )
                } else {
                    None
                }
            } else {
                None
            };

            // Determine final list of row indices to emit
            let rows_to_emit: Vec<usize> = if let Some(doc_ids) = fts_doc_ids {
                doc_ids
                    .into_iter()
                    .filter_map(|doc_id| {
                        let row_idx = doc_id as usize;
                        if row_idx < num_rows {
                            if let Some(ref filter) = row_filter {
                                if filter[row_idx] { Some(row_idx) } else { None }
                            } else {
                                Some(row_idx)
                            }
                        } else {
                            None
                        }
                    })
                    .collect()
            } else if let Some(ref filter) = row_filter {
                (0..num_rows).filter(|&r| filter[r]).collect()
            } else {
                (0..num_rows).collect()
            };

            let _valid_count = rows_to_emit.len();

            // Find columns needed for the predicate
            let mut predicate_col_names = Vec::new();
            if let Some(ref pred) = self.predicate {
                fn get_vars(e: &Expression, out: &mut Vec<String>) {
                    match e {
                        Expression::PropertyAccess(_, prop) => out.push(prop.clone()),
                        Expression::Variable(v) => out.push(v.clone()),
                        Expression::BinaryOp(_, l, r) => { get_vars(l, out); get_vars(r, out); }
                        Expression::UnaryOp(_, inner) => get_vars(inner, out),
                        Expression::FunctionCall(_, args) => { for a in args { get_vars(a, out); } }
                        _ => {}
                    }
                }
                get_vars(pred, &mut predicate_col_names);
            }

            let mut rows_to_emit = rows_to_emit;

            // Use column_ids if specified, otherwise scan all columns
            let cols_to_scan: Vec<usize> = if self.column_ids.is_empty() {
                (0..data.len()).collect()
            } else {
                self.column_ids.iter().map(|&id| id as usize).collect()
            };

            let mut fields = vec![None; cols_to_scan.len()];
            
            // Helper to materialize a single column
            let materialize_col = |col_idx: usize, current_rows: &[usize]| -> ValueVector {
                let col_data = &data[col_idx];
                let phys_type = if let Some(col_def) = self.table_columns.get(col_idx) {
                    Self::logical_to_physical(&col_def.logical_type)
                } else {
                    col_data.iter().find_map(|v| if !matches!(v, Value::Null) { Some(Self::value_to_physical_type(v)) } else { None }).unwrap_or(PhysicalTypeID::Int64)
                };
                let mut v = ValueVector::new(phys_type, current_rows.len().max(1));
                v.resize(current_rows.len());
                for (write_row, &r_idx) in current_rows.iter().enumerate() {
                    if let Some(val) = col_data.get(r_idx) {
                        Self::write_value_to_vector(&mut v, write_row, val);
                    }
                }
                v
            };

            // 1. Materialize predicate columns
            let mut pred_chunk_fields = Vec::new();
            let mut pred_chunk_names = Vec::new();
            if self.predicate.is_some() {
                for (i, &col_idx) in cols_to_scan.iter().enumerate() {
                    if col_idx >= data.len() { continue; }
                    if let Some(col_def) = self.table_columns.get(col_idx) {
                        if predicate_col_names.contains(&col_def.name) {
                            let v = materialize_col(col_idx, &rows_to_emit);
                            pred_chunk_fields.push(v);
                            pred_chunk_names.push(col_def.name.clone());
                            fields[i] = Some(true); // Mark as materialized temporarily for evaluation
                        }
                    }
                }
            }

            // 2. Evaluate predicate and filter rows
            let mut final_fields = Vec::new();
            if let Some(ref pred) = self.predicate {
                let pred_chunk = DataChunk::new(pred_chunk_fields).with_names(pred_chunk_names.clone());
                let evaluator_guard = self.evaluator.as_ref().map(|e| e.lock().unwrap());
                let mask = PhysicalFilter::evaluate_expression(pred, &pred_chunk, evaluator_guard.as_deref()).unwrap_or_else(|_| vec![true; rows_to_emit.len()]);
                
                let mut filtered_rows = Vec::with_capacity(rows_to_emit.len());
                for (i, &keep) in mask.iter().enumerate() {
                    if keep {
                        filtered_rows.push(rows_to_emit[i]);
                    }
                }
                rows_to_emit = filtered_rows;
                
                // We re-materialize ALL columns with the final rows_to_emit
                for i in 0..fields.len() {
                    fields[i] = None; 
                }
            }

            // 3. Materialize remaining columns with final rows_to_emit
            for (i, &col_idx) in cols_to_scan.iter().enumerate() {
                if col_idx >= data.len() { continue; }
                if fields[i].is_none() {
                    let v = materialize_col(col_idx, &rows_to_emit);
                    final_fields.push(v);
                }
            }

            let names: Vec<String> = cols_to_scan
                .iter()
                .filter_map(|&ci| self.table_columns.get(ci).map(|c| c.name.clone()))
                .collect();
            let chunk = DataChunk::new(final_fields).with_names(names);
            return Ok(vec![chunk]);
        }

        // Fallback: no data available — return empty result
        Ok(vec![DataChunk::new(vec![])])
    }
}


