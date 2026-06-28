//! Shadow file — copy-on-write versioning for pages during transactions.

use std::collections::HashMap;

/// A shadow file entry tracking an original → modified page mapping.
#[derive(Debug, Clone)]
pub struct ShadowEntry {
    pub original_page_id: u64,
    pub shadow_data: Vec<u8>,
    pub is_dirty: bool,
}

/// Manages shadow pages for copy-on-write during a transaction.
#[derive(Debug, Default)]
pub struct ShadowFile {
    entries: HashMap<u64, ShadowEntry>,
}

impl ShadowFile {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn create_shadow(&mut self, page_id: u64, data: Vec<u8>) {
        self.entries.insert(
            page_id,
            ShadowEntry {
                original_page_id: page_id,
                shadow_data: data,
                is_dirty: true,
            },
        );
    }

    pub fn get_shadow(&self, page_id: u64) -> Option<&ShadowEntry> {
        self.entries.get(&page_id)
    }

    pub fn has_shadow(&self, page_id: u64) -> bool {
        self.entries.contains_key(&page_id)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn dirty_pages(&self) -> impl Iterator<Item = &ShadowEntry> {
        self.entries.values().filter(|e| e.is_dirty)
    }
}
