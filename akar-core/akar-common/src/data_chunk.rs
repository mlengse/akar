use crate::selection::SelectionVector;
use crate::types::{PhysicalTypeID, Value};
use arrow::array::{ArrayRef, AsArray};
use arrow::compute::take;
use arrow::datatypes::*;

/// A contiguous batch of columnar data — the fundamental unit of data exchange
/// between physical operators.
///
/// Each `DataChunk` holds zero or more Arrow `ArrayRef` columns of equal length.
/// Operators consume input chunks and produce output chunks, forming a pipeline.
///
/// # Selection vectors
/// When `sel_vector` is set, only the rows indicated by the selection are "active".
/// Call `active_rows()` or `iter_rows()` to respect the selection; do not iterate
/// `0..size` directly.
///
/// # Example
/// ```text
/// let chunk = DataChunk::new(fields, field_types).with_names(names);
/// for row in chunk.iter_rows() {
///     let val = chunk.get_value(0, row); // column 0, current row
/// }
/// ```
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
    /// Create a new `DataChunk` from Arrow arrays and their physical types.
    ///
    /// The chunk `size` is set to the length of the first field. All fields
    /// must have the same length.
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
        let arrow_fields = legacy_fields
            .iter()
            .map(|f| crate::arrow_vector::ArrowVector::from_legacy(f).array.clone())
            .collect();
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

    /// Return a reference to the Arrow array at the given column index.
    #[inline(always)]
    pub fn field(&self, idx: usize) -> &ArrayRef {
        &self.fields[idx]
    }

    /// Return the number of columns in this chunk.
    #[inline(always)]
    pub fn num_fields(&self) -> usize {
        self.fields.len()
    }

    /// Resize the chunk (and all its fields) to the given number of rows.
    #[inline(always)]
    pub fn resize(&mut self, new_size: usize) {
        resize_chunk(self, new_size);
    }

    /// Return the number of active rows, taking the selection vector into account.
    #[inline(always)]
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
        let indices = arrow::array::UInt32Array::from_iter_values(sel.indices[..sel.size].iter().copied());

        for field in &mut self.fields {
            *field = take(field.as_ref(), &indices, None).unwrap();
        }
        self.size = sel.size;
    }

    /// Return `true` if the value at `(field_idx, row_idx)` is null.
    #[inline(always)]
    pub fn is_null(&self, field_idx: usize, row_idx: usize) -> bool {
        self.fields[field_idx].is_null(row_idx)
    }

    /// Read an `i64` value from column `field_idx` at `row_idx`, or `None` if null.
    ///
    /// Returns `None` if the column is not physically an `Int64` array (e.g. a
    /// `UInt64` column) — callers reading `UInt64` columns must use `get_u64`.
    #[inline]
    pub fn get_i64(&self, field_idx: usize, row_idx: usize) -> Option<i64> {
        if self.is_null(field_idx, row_idx) {
            return None;
        }
        if self.fields[field_idx].data_type() != &DataType::Int64 {
            return None;
        }
        let arr = self.fields[field_idx].as_primitive::<Int64Type>();
        Some(arr.value(row_idx))
    }

    /// Read a `u64` value from column `field_idx` at `row_idx`, or `None` if null.
    #[inline]
    pub fn get_u64(&self, field_idx: usize, row_idx: usize) -> Option<u64> {
        if self.is_null(field_idx, row_idx) {
            return None;
        }
        if self.fields[field_idx].data_type() != &DataType::UInt64 {
            return None;
        }
        let arr = self.fields[field_idx].as_primitive::<UInt64Type>();
        Some(arr.value(row_idx))
    }

    /// Read an `i32` value from column `field_idx` at `row_idx`, or `None` if null.
    #[inline]
    pub fn get_i32(&self, field_idx: usize, row_idx: usize) -> Option<i32> {
        if self.is_null(field_idx, row_idx) {
            return None;
        }
        let arr = self.fields[field_idx].as_primitive::<Int32Type>();
        Some(arr.value(row_idx))
    }

    /// Read an `f64` value from column `field_idx` at `row_idx`, or `None` if null.
    #[inline]
    pub fn get_f64(&self, field_idx: usize, row_idx: usize) -> Option<f64> {
        if self.is_null(field_idx, row_idx) {
            return None;
        }
        let arr = self.fields[field_idx].as_primitive::<Float64Type>();
        Some(arr.value(row_idx))
    }

    /// Read a `bool` value from column `field_idx` at `row_idx`, or `None` if null.
    #[inline]
    pub fn get_bool(&self, field_idx: usize, row_idx: usize) -> Option<bool> {
        if self.is_null(field_idx, row_idx) {
            return None;
        }
        let arr = self.fields[field_idx].as_boolean();
        Some(arr.value(row_idx))
    }

    /// Read a string slice from column `field_idx` at `row_idx`, or `None` if null.
    #[inline]
    pub fn get_string(&self, field_idx: usize, row_idx: usize) -> Option<&str> {
        if self.is_null(field_idx, row_idx) {
            return None;
        }
        let arr = self.fields[field_idx].as_string::<i32>();
        Some(arr.value(row_idx))
    }

    /// Read a value from column `field_idx` at `row_idx`, returning a boxed `Value`.
    ///
    /// Returns `None` if the cell is null. The physical type of the column
    /// determines which `Value` variant is produced.
    pub fn get_value(&self, field_idx: usize, row_idx: usize) -> Option<Value> {
        if self.is_null(field_idx, row_idx) {
            return None;
        }
        match self.field_types[field_idx] {
            PhysicalTypeID::Int64 => self.get_i64(field_idx, row_idx).map(Value::Int64),
            PhysicalTypeID::UInt64 => self.get_u64(field_idx, row_idx).map(Value::UInt64),
            PhysicalTypeID::Int32 => self.get_i32(field_idx, row_idx).map(Value::Int32),
            PhysicalTypeID::Double => self.get_f64(field_idx, row_idx).map(Value::Double),
            PhysicalTypeID::Bool => self.get_bool(field_idx, row_idx).map(Value::Bool),
            PhysicalTypeID::String => self
                .get_string(field_idx, row_idx)
                .map(|s| Value::String(s.to_string())),
            _ => None, // Expand as needed
        }
    }
}

/// Iterator over active row indices of a `DataChunk`.
///
/// When a selection vector is present, yields the selected indices.
/// Otherwise yields `0..size`.
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
