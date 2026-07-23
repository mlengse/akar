//! Shadow file — copy-on-write versioning for pages during transactions.
//!
//! During a transaction, shadow pages track modifications to database pages.
//! On commit, `apply()` finalises the changes (writes shadow data to the
//! BufferManager). On rollback, `discard()` drops all shadow pages.

use crate::buffer_manager::BufferManager;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
    /// Name of the file in the BufferManager these shadow pages belong to.
    file_name: Option<String>,
}

impl ShadowFile {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            file_name: None,
        }
    }

    /// Set the BufferManager file name that this shadow file tracks.
    pub fn set_file_name(&mut self, name: &str) {
        self.file_name = Some(name.to_string());
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

    /// Number of shadow entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all shadow entries (used during rollback to discard changes).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Apply all dirty shadow pages to the BufferManager.
    ///
    /// For each shadow entry, pins the page, writes the shadow data, marks it
    /// dirty, and unpins. This makes the transaction's writes visible.
    ///
    /// Call this on commit **after** the WAL has been flushed.
    pub fn apply(&self, buffer_manager: &Arc<Mutex<BufferManager>>) -> std::io::Result<()> {
        let file_name = match &self.file_name {
            Some(n) => n.clone(),
            None => return Ok(()), // No file to write to
        };

        let mut bm = buffer_manager.lock().unwrap();
        for entry in self.entries.values().filter(|e| e.is_dirty) {
            let frame = bm.pin_mut(&file_name, entry.original_page_id)?;
            // Copy shadow data into the frame
            let copy_len = entry.shadow_data.len().min(frame.data.len());
            frame.data[..copy_len].copy_from_slice(&entry.shadow_data[..copy_len]);
            frame.mark_dirty();
            bm.unpin(&file_name, entry.original_page_id);
        }
        Ok(())
    }

    /// Discard all shadow entries (rollback).
    ///
    /// Simply clears the entries — the original pages in the BufferManager
    /// were never modified, so no undo is needed at the page level.
    pub fn discard(&mut self) {
        self.entries.clear();
    }

    pub fn dirty_pages(&self) -> impl Iterator<Item = &ShadowEntry> {
        self.entries.values().filter(|e| e.is_dirty)
    }
}
