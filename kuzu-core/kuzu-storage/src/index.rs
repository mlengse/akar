//! Hash index for primary key lookups.

use hashbrown::HashMap;
use std::hash::Hash;

/// A hash index mapping a key to a row offset in a table.
#[derive(Debug, Clone)]
pub struct HashIndex<K: Hash + Eq + Clone> {
    entries: HashMap<K, u64>,
}

impl<K: Hash + Eq + Clone> HashIndex<K> {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn insert(&mut self, key: K, row_offset: u64) {
        self.entries.insert(key, row_offset);
    }

    pub fn lookup(&self, key: &K) -> Option<u64> {
        self.entries.get(key).copied()
    }

    pub fn delete(&mut self, key: &K) {
        self.entries.remove(key);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<K: Hash + Eq + Clone> Default for HashIndex<K> {
    fn default() -> Self {
        Self::new()
    }
}
