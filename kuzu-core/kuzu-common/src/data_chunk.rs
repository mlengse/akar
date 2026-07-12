use crate::selection::SelectionVector;
use crate::vector::ValueVector;

#[derive(Debug, Clone)]
pub struct DataChunk {
    pub fields: Vec<ValueVector>,
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
        field.resize(new_size);
    }
}

impl DataChunk {
    pub fn new(fields: Vec<ValueVector>) -> Self {
        let size = fields.first().map(|f| f.size()).unwrap_or(0);
        Self {
            fields,
            size,
            field_names: vec![],
            sel_vector: None,
        }
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

    pub fn field(&self, idx: usize) -> &ValueVector {
        &self.fields[idx]
    }

    pub fn field_mut(&mut self, idx: usize) -> &mut ValueVector {
        &mut self.fields[idx]
    }

    pub fn num_fields(&self) -> usize {
        self.fields.len()
    }

    pub fn resize(&mut self, new_size: usize) {
        self.size = new_size;
        for field in &mut self.fields {
            field.resize(new_size);
        }
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
        for field in &mut self.fields {
            let elem_size = crate::vector::physical_type_size(field.physical_type());
            let old_size = field.size();
            let old_nulls: Vec<bool> = (0..old_size).map(|i| field.is_null(i)).collect();
            let old_data = field.data().to_vec();

            field.resize(sel.size);
            let new_data = field.data_mut();
            for (new_idx, &src_idx) in sel.indices[..sel.size].iter().enumerate() {
                let src_off = src_idx as usize * elem_size;
                let dst_off = new_idx * elem_size;
                new_data[dst_off..dst_off + elem_size]
                    .copy_from_slice(&old_data[src_off..src_off + elem_size]);
            }
            for (new_idx, &src_idx) in sel.indices[..sel.size].iter().enumerate() {
                field.set_null(new_idx, old_nulls[src_idx as usize]);
            }
        }
        self.size = sel.size;
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
