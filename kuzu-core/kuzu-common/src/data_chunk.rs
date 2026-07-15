use crate::selection::SelectionVector;
use crate::types::{PhysicalTypeID, Value};
use arrow::array::{ArrayRef, AsArray};
use arrow::compute::take;
use arrow::datatypes::*;

#[derive(Debug, Clone)]
pub struct DataChunk {
    pub fields: Vec<ArrayRef>,
    pub field_types: Vec<PhysicalTypeID>,
    pub size: usize,
    pub field_names: Vec<String>,
    /// Optional selection vector for zero-copy filtering.
    /// When set, only the rows at `sel_vector.indices[0..sel_vector.size]`
    /// are considered active. Operators should iterate using the sel_vector
    /// rather than `size` for correct results.
    pub sel_vector: Option<SelectionVector>,
}

/// Resize a DataChunk to the given number of rows.
pub fn resize_chunk(chunk: &mut DataChunk, new_size: usize) {
    chunk.size = new_size;
    for field in &mut chunk.fields {
        if new_size <= field.len() {
            *field = field.slice(0, new_size);
        }
    }
}

impl DataChunk {
    pub fn new(fields: Vec<ArrayRef>, field_types: Vec<PhysicalTypeID>) -> Self {
        let size = fields.first().map(|f| f.len()).unwrap_or(0);
        Self {
            fields,
            field_types,
            size,
            field_names: vec![],
            sel_vector: None,
        }
    }

    /// Construct a DataChunk from legacy vectors (for backward-compatibility in benchmarks)
    pub fn from_legacy(legacy_fields: Vec<crate::vector::ValueVector>) -> Self {
        let field_types = legacy_fields.iter().map(|f| f.physical_type()).collect();
        let arrow_fields = legacy_fields.iter().map(|f| crate::arrow_vector::ArrowVector::from_legacy(f).array.clone()).collect();
        Self::new(arrow_fields, field_types)
    }

    /// Attach column names to this chunk (builder pattern).
    pub fn with_names(mut self, names: Vec<String>) -> Self {
        self.field_names = names;
        self
    }

    /// Attach a selection vector to this chunk.
    pub fn with_sel(mut self, sel: SelectionVector) -> Self {
        self.sel_vector = Some(sel);
        self
    }

    pub fn field(&self, idx: usize) -> &ArrayRef {
        &self.fields[idx]
    }

    pub fn num_fields(&self) -> usize {
        self.fields.len()
    }

    pub fn resize(&mut self, new_size: usize) {
        resize_chunk(self, new_size);
    }

    /// Return the number of active rows, taking the selection vector into account.
    pub fn active_rows(&self) -> usize {
        self.sel_vector.as_ref().map_or(self.size, |s| s.size)
    }

    /// Iterate over active row indices.
    /// When a selection vector is present, yields the selected indices.
    /// Otherwise, yields 0..size.
    pub fn iter_rows(&self) -> RowIter<'_> {
        RowIter {
            sel: self.sel_vector.as_ref(),
            size: self.size,
            pos: 0,
        }
    }

    /// Materialize the selection vector into physical data.
    /// After calling this, sel_vector is None and only the selected rows remain
    /// in the data buffers. This is a no-op if sel_vector is already None.
    pub fn materialize(&mut self) {
        let sel = match self.sel_vector.take() {
            Some(s) => s,
            None => return,
        };
        if sel.size == self.size {
            return;
        }
        let indices = arrow::array::UInt32Array::from_iter_values(
            sel.indices[..sel.size].iter().copied()
        );
        
        for field in &mut self.fields {
            *field = take(field.as_ref(), &indices, None).unwrap();
        }
        self.size = sel.size;
    }
    
    pub fn is_null(&self, field_idx: usize, row_idx: usize) -> bool {
        self.fields[field_idx].is_null(row_idx)
    }

    pub fn get_i64(&self, field_idx: usize, row_idx: usize) -> Option<i64> {
        if self.is_null(field_idx, row_idx) { return None; }
        let arr = self.fields[field_idx].as_primitive::<Int64Type>();
        Some(arr.value(row_idx))
    }

    pub fn get_i32(&self, field_idx: usize, row_idx: usize) -> Option<i32> {
        if self.is_null(field_idx, row_idx) { return None; }
        let arr = self.fields[field_idx].as_primitive::<Int32Type>();
        Some(arr.value(row_idx))
    }

    pub fn get_f64(&self, field_idx: usize, row_idx: usize) -> Option<f64> {
        if self.is_null(field_idx, row_idx) { return None; }
        let arr = self.fields[field_idx].as_primitive::<Float64Type>();
        Some(arr.value(row_idx))
    }

    pub fn get_bool(&self, field_idx: usize, row_idx: usize) -> Option<bool> {
        if self.is_null(field_idx, row_idx) { return None; }
        let arr = self.fields[field_idx].as_boolean();
        Some(arr.value(row_idx))
    }

    pub fn get_string(&self, field_idx: usize, row_idx: usize) -> Option<&str> {
        if self.is_null(field_idx, row_idx) { return None; }
        let arr = self.fields[field_idx].as_string::<i32>();
        Some(arr.value(row_idx))
    }

    pub fn get_value(&self, field_idx: usize, row_idx: usize) -> Option<Value> {
        if self.is_null(field_idx, row_idx) { return None; }
        match self.field_types[field_idx] {
            PhysicalTypeID::Int64 => self.get_i64(field_idx, row_idx).map(Value::Int64),
            PhysicalTypeID::Int32 => self.get_i32(field_idx, row_idx).map(Value::Int32),
            PhysicalTypeID::Double => self.get_f64(field_idx, row_idx).map(Value::Double),
            PhysicalTypeID::Bool => self.get_bool(field_idx, row_idx).map(Value::Bool),
            PhysicalTypeID::String => self.get_string(field_idx, row_idx).map(|s| Value::String(s.to_string())),
            _ => None, // Expand as needed
        }
    }
}

pub struct RowIter<'a> {
    sel: Option<&'a SelectionVector>,
    size: usize,
    pos: usize,
}

impl<'a> Iterator for RowIter<'a> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(sel) = self.sel {
            if self.pos < sel.size {
                let idx = sel.indices[self.pos] as usize;
                self.pos += 1;
                Some(idx)
            } else {
                None
            }
        } else {
            if self.pos < self.size {
                let idx = self.pos;
                self.pos += 1;
                Some(idx)
            } else {
                None
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = if let Some(sel) = self.sel {
            sel.size.saturating_sub(self.pos)
        } else {
            self.size.saturating_sub(self.pos)
        };
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for RowIter<'a> {}
