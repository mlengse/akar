use std::ops::Range;

#[derive(Debug, Clone)]
pub struct SelectionVector {
    pub indices: Vec<u32>,
    pub size: usize,
}

impl SelectionVector {
    pub fn new(capacity: usize) -> Self {
        Self {
            indices: Vec::with_capacity(capacity),
            size: 0,
        }
    }

    pub fn from_slice(indices: &[u32]) -> Self {
        let mut sv = Self::new(indices.len());
        sv.indices.extend_from_slice(indices);
        sv.size = indices.len();
        sv
    }

    pub fn from_range(len: usize) -> Self {
        let indices: Vec<u32> = (0..len as u32).collect();
        Self {
            size: len,
            indices,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub fn clear(&mut self) {
        self.indices.clear();
        self.size = 0;
    }

    pub fn push(&mut self, idx: u32) {
        if self.size < self.indices.len() {
            self.indices[self.size] = idx;
        } else {
            self.indices.push(idx);
        }
        self.size += 1;
    }

    pub fn iter(&self) -> SelectionIter<'_> {
        SelectionIter {
            indices: &self.indices[..self.size],
            pos: 0,
        }
    }

    pub fn get(&self, pos: usize) -> Option<u32> {
        if pos < self.size {
            Some(self.indices[pos])
        } else {
            None
        }
    }

    pub fn slice(&self, range: Range<usize>) -> &[u32] {
        let end = range.end.min(self.size);
        &self.indices[range.start..end]
    }
}

pub struct SelectionIter<'a> {
    indices: &'a [u32],
    pos: usize,
}

impl<'a> Iterator for SelectionIter<'a> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos < self.indices.len() {
            let val = self.indices[self.pos];
            self.pos += 1;
            Some(val)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.indices.len() - self.pos;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for SelectionIter<'a> {}

impl<'a> IntoIterator for &'a SelectionVector {
    type Item = u32;
    type IntoIter = SelectionIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
