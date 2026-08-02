//! Auto-extracted from physical_operator.rs
use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical::scan_filter::PhysicalFilter;
use crate::physical::types::{NodeSemiMask, OperatorResult, PhysicalOperatorExec};
use crate::physical::write_ops::PhysicalFtsScan;
use akar_common::types::{LogicalTypeID, PhysicalTypeID, Value};
use akar_common::vector::DataChunk;
use akar_parser::ast::Expression;
use akar_storage::table::ColumnDefinition;
use arrow::array::{
    Array, BooleanBuilder, Float32Builder, Float64Builder, Int32Builder, Int64Builder, StringBuilder, UInt64Array,
};
use arrow::compute;
use std::sync::{Arc, Mutex};

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
    /// Pre-built Arrow arrays (one per column). When present, skips the
    /// `Vec<Vec<Value>>` → Arrow conversion, reading directly from storage.
    pub table_arrow_data: Option<Vec<arrow::array::ArrayRef>>,
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
            table_arrow_data: None,
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

    /// Attach pre-built Arrow arrays, bypassing the `Vec<Vec<Value>>` intermediate.
    pub fn with_arrow_data(mut self, arrays: Vec<arrow::array::ArrayRef>, columns: Vec<ColumnDefinition>) -> Self {
        self.table_arrow_data = Some(arrays);
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
            Value::String(_) => PhysicalTypeID::String,
            Value::Date(_)
            | Value::Timestamp(_)
            | Value::TimestampTz(_)
            | Value::TimestampNs(_)
            | Value::TimestampMs(_)
            | Value::TimestampSec(_)
            | Value::DTime(_) => PhysicalTypeID::Int64,
            Value::Interval(_) => PhysicalTypeID::String,
            Value::Blob(_) => PhysicalTypeID::Blob,
            Value::UInt128(_) => PhysicalTypeID::Int128,
            Value::Json(_) => PhysicalTypeID::String,
            Value::Union(_, _) => PhysicalTypeID::Struct,
            Value::InternalID(_) | Value::List(_) | Value::Map(_) | Value::Struct(_) => PhysicalTypeID::Int64,
        }
    }

    /// Determine PhysicalTypeID from a LogicalTypeID.
    pub(crate) fn logical_to_physical(logical: &LogicalTypeID) -> PhysicalTypeID {
        match logical {
            LogicalTypeID::Bool => PhysicalTypeID::Bool,
            LogicalTypeID::Int64
            | LogicalTypeID::UInt64
            | LogicalTypeID::Int128
            | LogicalTypeID::Serial
            | LogicalTypeID::UInt128 => PhysicalTypeID::Int64,
            LogicalTypeID::Int32 | LogicalTypeID::UInt32 => PhysicalTypeID::Int32,
            LogicalTypeID::Int16 | LogicalTypeID::UInt16 => PhysicalTypeID::Int16,
            LogicalTypeID::Int8 | LogicalTypeID::UInt8 => PhysicalTypeID::Int8,
            LogicalTypeID::Double | LogicalTypeID::Decimal => PhysicalTypeID::Double,
            LogicalTypeID::Float => PhysicalTypeID::Float,
            LogicalTypeID::Date
            | LogicalTypeID::Timestamp
            | LogicalTypeID::TimestampTz
            | LogicalTypeID::TimestampMs
            | LogicalTypeID::TimestampNs
            | LogicalTypeID::TimestampSec
            | LogicalTypeID::Time => PhysicalTypeID::Int64,
            LogicalTypeID::String
            | LogicalTypeID::Interval
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

    /// Build an Arrow ArrayRef directly from Vec<Value> bypassing ValueVector.
    fn build_arrow_array(phys_type: PhysicalTypeID, col_data: &[Value], rows: &[usize]) -> arrow::array::ArrayRef {
        let size = rows.len();
        match phys_type {
            PhysicalTypeID::Bool => {
                let mut builder = BooleanBuilder::with_capacity(size);
                for &r in rows {
                    match col_data.get(r) {
                        Some(Value::Bool(b)) => builder.append_value(*b),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            PhysicalTypeID::Int64 => {
                let mut builder = Int64Builder::with_capacity(size);
                for &r in rows {
                    match col_data.get(r) {
                        Some(Value::Int64(v)) => builder.append_value(*v),
                        Some(Value::Int32(v)) => builder.append_value(*v as i64),
                        Some(Value::Int16(v)) => builder.append_value(*v as i64),
                        Some(Value::Int8(v)) => builder.append_value(*v as i64),
                        Some(Value::UInt64(v)) => builder.append_value(*v as i64),
                        Some(Value::UInt32(v)) => builder.append_value(*v as i64),
                        Some(Value::UInt16(v)) => builder.append_value(*v as i64),
                        Some(Value::UInt8(v)) => builder.append_value(*v as i64),
                        Some(Value::Date(v)) => builder.append_value(v.0 as i64),
                        Some(Value::Timestamp(v))
                        | Some(Value::TimestampNs(v))
                        | Some(Value::TimestampMs(v))
                        | Some(Value::TimestampSec(v)) => builder.append_value(v.0),
                        Some(Value::TimestampTz(v)) => builder.append_value(v.0),
                        Some(Value::DTime(v)) => builder.append_value(*v),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            PhysicalTypeID::Int32 => {
                let mut builder = Int32Builder::with_capacity(size);
                for &r in rows {
                    match col_data.get(r) {
                        Some(Value::Int32(v)) => builder.append_value(*v),
                        Some(Value::Int16(v)) => builder.append_value(*v as i32),
                        Some(Value::Int8(v)) => builder.append_value(*v as i32),
                        Some(Value::Int64(v)) => builder.append_value(*v as i32),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            PhysicalTypeID::Double => {
                let mut builder = Float64Builder::with_capacity(size);
                for &r in rows {
                    match col_data.get(r) {
                        Some(Value::Double(v)) => builder.append_value(*v),
                        Some(Value::Float(v)) => builder.append_value(*v as f64),
                        Some(Value::Int64(v)) => builder.append_value(*v as f64),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            PhysicalTypeID::Float => {
                let mut builder = Float32Builder::with_capacity(size);
                for &r in rows {
                    match col_data.get(r) {
                        Some(Value::Float(v)) => builder.append_value(*v),
                        Some(Value::Double(v)) => builder.append_value(*v as f32),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            PhysicalTypeID::String => {
                let mut builder = StringBuilder::with_capacity(size, size * 16);
                for &r in rows {
                    match col_data.get(r) {
                        Some(Value::String(s)) => builder.append_value(s),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            _ => {
                let mut builder = Int64Builder::with_capacity(size);
                for _ in 0..size {
                    builder.append_null();
                }
                std::sync::Arc::new(builder.finish())
            }
        }
    }
}

impl PhysicalScan {
    /// Execute scan using pre-built Arrow arrays (fast path).
    /// Skips the `Vec<Vec<Value>>` → Arrow conversion entirely.
    fn execute_with_arrow_arrays(&self, arrays: &[arrow::array::ArrayRef]) -> OperatorResult {
        if arrays.is_empty() || arrays[0].is_empty() {
            return Ok(vec![DataChunk::new(vec![], vec![])]);
        }

        let num_rows = arrays[0].len();

        // Determine column indices to scan
        let cols_to_scan: Vec<usize> = if self.column_ids.is_empty() {
            (0..arrays.len()).collect()
        } else {
            self.column_ids.iter().map(|&id| id as usize).collect()
        };

        // FTS query — execute and filter rows by matching doc_ids
        let mut fts_doc_ids = None;
        if let Some(ref fts) = self.fts_query {
            let fts_chunks = fts.execute(vec![])?;
            let mut doc_ids = Vec::new();
            if let Some(chunk) = fts_chunks.first() {
                for row in 0..chunk.size {
                    if let Some(doc_id) = chunk.get_i64(0, row) {
                        doc_ids.push(doc_id);
                    }
                }
            }
            fts_doc_ids = Some(doc_ids);
        }

        // Initial rows to emit (all rows, or FTS-filtered)
        let mut rows_to_emit: Vec<usize> = if let Some(doc_ids) = fts_doc_ids {
            doc_ids
                .into_iter()
                .filter(|&row| (row as usize) < num_rows)
                .map(|row| row as usize)
                .collect()
        } else {
            (0..num_rows).collect()
        };

        // Find columns needed for the predicate
        let mut predicate_col_names = Vec::new();
        if let Some(ref pred) = self.predicate {
            fn get_vars(e: &Expression, out: &mut Vec<String>) {
                match e {
                    Expression::PropertyAccess(_, prop) => out.push(prop.clone()),
                    Expression::Variable(v) => out.push(v.clone()),
                    Expression::BinaryOp(_, l, r) => {
                        get_vars(l, out);
                        get_vars(r, out);
                    }
                    Expression::UnaryOp(_, inner) => get_vars(inner, out),
                    Expression::FunctionCall(_, args) => {
                        for a in args {
                            get_vars(a, out);
                        }
                    }
                    _ => {}
                }
            }
            get_vars(pred, &mut predicate_col_names);
        }

        // Evaluate predicate if present
        if let Some(ref pred) = self.predicate {
            let mut pred_fields = Vec::new();
            let mut pred_types = Vec::new();
            let mut pred_names = Vec::new();
            for &col_idx in &cols_to_scan {
                if col_idx >= arrays.len() {
                    continue;
                }
                if let Some(col_def) = self.table_columns.get(col_idx) {
                    if predicate_col_names.contains(&col_def.name) {
                        let arr = &arrays[col_idx];
                        let ptype = Self::logical_to_physical(&col_def.logical_type);
                        pred_fields.push(arr.clone());
                        pred_types.push(ptype);
                        pred_names.push(col_def.name.clone());
                    }
                }
            }

            let pred_chunk = DataChunk::new(pred_fields, pred_types).with_names(pred_names);
            let evaluator_guard = self.evaluator.as_ref().map(|e| e.lock().unwrap());
            let mask = PhysicalFilter::evaluate_expression(pred, &pred_chunk, evaluator_guard.as_deref())
                .unwrap_or_else(|_| vec![true; rows_to_emit.len()]);

            let mut filtered_rows = Vec::with_capacity(rows_to_emit.len());
            for (i, &keep) in mask.iter().enumerate() {
                if keep {
                    filtered_rows.push(rows_to_emit[i]);
                }
            }
            rows_to_emit = filtered_rows;
        }

        // Take filtered rows from pre-built Arrow arrays using take kernel
        let mut final_fields = Vec::new();
        let mut final_types = Vec::new();
        let indices: Vec<u64> = rows_to_emit.iter().map(|&r| r as u64).collect();
        let indices_arr = UInt64Array::from(indices);
        for &col_idx in &cols_to_scan {
            if col_idx >= arrays.len() {
                continue;
            }
            let arr = &arrays[col_idx];
            let ptype = self
                .table_columns
                .get(col_idx)
                .map(|c| Self::logical_to_physical(&c.logical_type))
                .unwrap_or(PhysicalTypeID::Int64);
            let taken = compute::take(arr.as_ref(), &indices_arr, None).unwrap_or_else(|_| arr.slice(0, 0));
            final_fields.push(taken);
            final_types.push(ptype);
        }

        let names: Vec<String> = cols_to_scan
            .iter()
            .filter_map(|&ci| self.table_columns.get(ci).map(|c| c.name.clone()))
            .collect();
        let chunk = DataChunk::new(final_fields, final_types).with_names(names);
        Ok(vec![chunk])
    }

    /// Execute scan using legacy Vec<Vec<Value>> data.
    fn execute_with_value_data(&self, data: &[Vec<Value>]) -> OperatorResult {
        if data.is_empty() || data[0].is_empty() {
            return Ok(vec![DataChunk::new(vec![], vec![])]);
        }

        let num_rows = data[0].len();

        let mut fts_doc_ids = None;
        if let Some(ref fts) = self.fts_query {
            let fts_chunks = fts.execute(vec![])?;
            let mut doc_ids = Vec::new();
            if let Some(chunk) = fts_chunks.first() {
                if let Some(_id_vec) = chunk.fields.first() {
                    for row in 0..chunk.size {
                        if let Some(doc_id) = chunk.get_i64(self.column_ids.first().copied().unwrap_or(0) as usize, row)
                        {
                            doc_ids.push(doc_id);
                        }
                    }
                }
            }
            fts_doc_ids = Some(doc_ids);
        }

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

        let mut predicate_col_names = Vec::new();
        if let Some(ref pred) = self.predicate {
            fn get_vars(e: &Expression, out: &mut Vec<String>) {
                match e {
                    Expression::PropertyAccess(_, prop) => out.push(prop.clone()),
                    Expression::Variable(v) => out.push(v.clone()),
                    Expression::BinaryOp(_, l, r) => {
                        get_vars(l, out);
                        get_vars(r, out);
                    }
                    Expression::UnaryOp(_, inner) => get_vars(inner, out),
                    Expression::FunctionCall(_, args) => {
                        for a in args {
                            get_vars(a, out);
                        }
                    }
                    _ => {}
                }
            }
            get_vars(pred, &mut predicate_col_names);
        }

        let mut rows_to_emit = rows_to_emit;

        let cols_to_scan: Vec<usize> = if self.column_ids.is_empty() {
            (0..data.len()).collect()
        } else {
            self.column_ids.iter().map(|&id| id as usize).collect()
        };

        let mut fields = vec![None; cols_to_scan.len()];

        let materialize_col = |col_idx: usize, current_rows: &[usize]| -> (arrow::array::ArrayRef, PhysicalTypeID) {
            let col_data = &data[col_idx];
            let phys_type = if let Some(col_def) = self.table_columns.get(col_idx) {
                Self::logical_to_physical(&col_def.logical_type)
            } else {
                col_data
                    .iter()
                    .find_map(|v| {
                        if !matches!(v, Value::Null) {
                            Some(Self::value_to_physical_type(v))
                        } else {
                            None
                        }
                    })
                    .unwrap_or(PhysicalTypeID::Int64)
            };
            let array = Self::build_arrow_array(phys_type, col_data, current_rows);
            (array, phys_type)
        };

        let mut pred_chunk_fields = Vec::new();
        let mut pred_chunk_types = Vec::new();
        let mut pred_chunk_names = Vec::new();
        if self.predicate.is_some() {
            for (i, &col_idx) in cols_to_scan.iter().enumerate() {
                if col_idx >= data.len() {
                    continue;
                }
                if let Some(col_def) = self.table_columns.get(col_idx) {
                    if predicate_col_names.contains(&col_def.name) {
                        let (arr, ptype) = materialize_col(col_idx, &rows_to_emit);
                        pred_chunk_fields.push(arr);
                        pred_chunk_types.push(ptype);
                        pred_chunk_names.push(col_def.name.clone());
                        fields[i] = Some(true);
                    }
                }
            }
        }

        let mut final_fields = Vec::new();
        if let Some(ref pred) = self.predicate {
            let pred_chunk = DataChunk::new(pred_chunk_fields, pred_chunk_types).with_names(pred_chunk_names.clone());
            let evaluator_guard = self.evaluator.as_ref().map(|e| e.lock().unwrap());
            let mask = PhysicalFilter::evaluate_expression(pred, &pred_chunk, evaluator_guard.as_deref())
                .unwrap_or_else(|_| vec![true; rows_to_emit.len()]);

            let mut filtered_rows = Vec::with_capacity(rows_to_emit.len());
            for (i, &keep) in mask.iter().enumerate() {
                if keep {
                    filtered_rows.push(rows_to_emit[i]);
                }
            }
            rows_to_emit = filtered_rows;

            fields.fill(None);
        }

        let mut final_types = Vec::new();
        for (i, &col_idx) in cols_to_scan.iter().enumerate() {
            if col_idx >= data.len() {
                continue;
            }
            if fields[i].is_none() {
                let (arr, ptype) = materialize_col(col_idx, &rows_to_emit);
                final_fields.push(arr);
                final_types.push(ptype);
            }
        }

        let names: Vec<String> = cols_to_scan
            .iter()
            .filter_map(|&ci| self.table_columns.get(ci).map(|c| c.name.clone()))
            .collect();
        let chunk = DataChunk::new(final_fields, final_types).with_names(names);
        Ok(vec![chunk])
    }
}

impl PhysicalOperatorExec for PhysicalScan {
    fn operator_type(&self) -> &str {
        "scan"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // Fast path: pre-built Arrow arrays (avoids Vec<Vec<Value>> intermediate and double materialization)
        if let Some(ref arrays) = self.table_arrow_data {
            return self.execute_with_arrow_arrays(arrays);
        }

        // Legacy path: Vec<Vec<Value>> data
        if let Some(ref data) = self.table_data {
            return self.execute_with_value_data(data);
        }

        // Fallback: no data available — return empty result
        Ok(vec![DataChunk::new(vec![], vec![])])
    }
}
