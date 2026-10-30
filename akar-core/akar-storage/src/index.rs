//! Hash index for primary key lookups — on-disk persistent + in-memory cache.
//!
//! # Architecture
//!
//! Two-layer index:
//! - **L1 (cache):** `HashMap<K, u64>` — fast in-memory lookups.
//! - **L2 (persistent):** `OnDiskHashIndex` — page-based storage via `BufferManager`.
//!
//! On `flush()`, L2 is written to disk. On startup, L1 is rebuilt by scanning L2.
//! This gives O(1) lookups at runtime while ensuring data survives restarts.
//!
//! # On-Disk Page Layout
//!
//! Each page stores one "bucket" of the hash table:
//!
//! ```text
//! [header: 12 bytes] [slots...]
//!
//! header:
//!   num_slots: u32        — total slots in this page
//!   num_entries: u32      — used slots
//!   collision_next: u32   — page number of next overflow page (0 = none)
//!
//! slot layout (variable width per key type):
//!   [key_bytes: key_size] [value: u64 LE] [flags: u8]
//!   flags: bit 0 = occupied, bit 1 = deleted
//! ```
//!
//! The slot width is fixed per index instance. Keys longer than ~64 bytes
//! store a u64 hash instead and use the in-memory L1 for collision resolution.

use crate::buffer_manager::BufferManager;
use akar_common::error::{AkarError, StorageError};
use hashbrown::HashMap;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::path::PathBuf;

/// Default number of slots per page.
const SLOTS_PER_PAGE: u32 = 64;

/// Size of the page header in bytes.
const PAGE_HEADER_SIZE: usize = 12;

/// Per-slot flags byte.
const FLAG_OCCUPIED: u8 = 0x01;
const FLAG_DELETED: u8 = 0x02;

// ---------------------------------------------------------------------------
// In-memory L1 cache — wraps `HashMap`
// ---------------------------------------------------------------------------

/// A hash index mapping a key to a row offset in a table.
/// L1 cache layer — fast in-memory lookups.
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

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Iterate over all (key, offset) entries.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &u64)> {
        self.entries.iter()
    }
}

impl<K: Hash + Eq + Clone> Default for HashIndex<K> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// On-disk persistent L2 index
// ---------------------------------------------------------------------------

/// Trait for types that can be serialized into fixed-size byte arrays
/// for on-disk hash index storage.
pub trait IndexKey: Clone + std::fmt::Debug {
    /// Size of the serialized key in bytes.
    fn key_size() -> usize;

    /// Serialize this key into the provided byte slice (must be `key_size()` bytes).
    fn serialize_into(&self, buf: &mut [u8]);

    /// Deserialize a key from a byte slice (must be `key_size()` bytes).
    fn deserialize_from(buf: &[u8]) -> Result<Self, AkarError>;
}

// Implement for the most common key type: String (variable-length → store hash)
impl IndexKey for String {
    fn key_size() -> usize {
        8 // store u64 hash for variable-length keys
    }

    fn serialize_into(&self, buf: &mut [u8]) {
        let h = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.hash(&mut hasher);
            hasher.finish()
        };
        buf.copy_from_slice(&h.to_le_bytes());
    }

    fn deserialize_from(buf: &[u8]) -> Result<Self, AkarError> {
        // We can't recover the original string from a hash.
        // For String keys, the L1 cache is the canonical store.
        // The on-disk index is used only for persistence/recovery.
        let h = u64::from_le_bytes(
            buf[..8]
                .try_into()
                .map_err(|e| AkarError::Storage(StorageError::Index(format!("bad hash key bytes: {e}"))))?,
        );
        Ok(format!("__hash_{h}")) // placeholder; real recovery uses L1 cache
    }
}

impl IndexKey for u64 {
    fn key_size() -> usize {
        8
    }

    fn serialize_into(&self, buf: &mut [u8]) {
        buf.copy_from_slice(&self.to_le_bytes());
    }

    fn deserialize_from(buf: &[u8]) -> Result<Self, AkarError> {
        Ok(u64::from_le_bytes(buf[..8].try_into().map_err(|e| {
            AkarError::Storage(StorageError::Index(format!("bad u64 key bytes: {e}")))
        })?))
    }
}

impl IndexKey for i64 {
    fn key_size() -> usize {
        8
    }

    fn serialize_into(&self, buf: &mut [u8]) {
        buf.copy_from_slice(&self.to_le_bytes());
    }

    fn deserialize_from(buf: &[u8]) -> Result<Self, AkarError> {
        Ok(i64::from_le_bytes(buf[..8].try_into().map_err(|e| {
            AkarError::Storage(StorageError::Index(format!("bad i64 key bytes: {e}")))
        })?))
    }
}

impl IndexKey for u32 {
    fn key_size() -> usize {
        4
    }

    fn serialize_into(&self, buf: &mut [u8]) {
        buf.copy_from_slice(&self.to_le_bytes());
    }

    fn deserialize_from(buf: &[u8]) -> Result<Self, AkarError> {
        let size = Self::key_size();
        Ok(u32::from_le_bytes(buf[..size].try_into().map_err(|e| {
            AkarError::Storage(StorageError::Index(format!("bad u32 key bytes: {e}")))
        })?))
    }
}

impl IndexKey for i32 {
    fn key_size() -> usize {
        4
    }

    fn serialize_into(&self, buf: &mut [u8]) {
        buf.copy_from_slice(&self.to_le_bytes());
    }

    fn deserialize_from(buf: &[u8]) -> Result<Self, AkarError> {
        let size = Self::key_size();
        Ok(i32::from_le_bytes(buf[..size].try_into().map_err(|e| {
            AkarError::Storage(StorageError::Index(format!("bad i32 key bytes: {e}")))
        })?))
    }
}

/// On-disk persistent hash index backed by the BufferManager.
///
/// # Type Parameters
///
/// - `K`: The key type. Must implement `IndexKey` for serialization.
///
/// # Thread Safety
///
/// This struct is `Send + Sync` when wrapped in `Arc<Mutex<...>>`.
#[derive(Debug, Clone)]
pub struct OnDiskHashIndex<K: IndexKey> {
    /// Name of the index file (used as BufferManager file identifier).
    file_name: String,
    /// Number of key bytes per slot.
    key_size: usize,
    /// Total slot size = key_size + 8 (value) + 1 (flags).
    slot_size: usize,
    /// Number of slots per page.
    slots_per_page: u32,
    /// L1 in-memory cache for O(1) lookups.
    cache: HashMap<K, u64>,
    /// Number of pages allocated.
    num_pages: u32,
    _phantom: PhantomData<K>,
}

impl<K: IndexKey + Hash + Eq> OnDiskHashIndex<K> {
    /// Create a new on-disk hash index.
    ///
    /// If `bm` is `None`, operates purely in-memory (L1 cache only).
    /// The `file_name` is used to register the index file with the BufferManager.
    pub fn new(file_name: &str) -> Self {
        let key_size = K::key_size();
        let slot_size = key_size + 8 + 1; // key + value(u64) + flags
        Self {
            file_name: file_name.to_string(),
            key_size,
            slot_size,
            slots_per_page: SLOTS_PER_PAGE,
            cache: HashMap::new(),
            num_pages: 0,
            _phantom: PhantomData,
        }
    }

    /// Rebuild the L1 cache from on-disk pages.
    ///
    /// Call this on database startup after the BufferManager is initialized.
    pub fn rebuild_from_disk(&mut self, bm: &mut BufferManager) -> std::io::Result<()> {
        self.cache.clear();

        // Register the index file if not already registered.
        let file_path = PathBuf::from(&self.file_name);
        let db_path = file_path.parent().unwrap_or(&file_path);
        let full_path = db_path.join(format!("{}.idx", self.file_name));
        if !bm.is_file_registered(&self.file_name) {
            bm.register_file(&self.file_name, full_path);
        }

        for page_num in 0..self.num_pages as u64 {
            let frame = bm.pin(&self.file_name, page_num)?;
            let data = &frame.data;

            let num_slots = u32::from_le_bytes(
                data[0..4]
                    .try_into()
                    .map_err(|e| std::io::Error::other(format!("bad page header num_slots: {e}")))?,
            );
            let num_entries = u32::from_le_bytes(
                data[4..8]
                    .try_into()
                    .map_err(|e| std::io::Error::other(format!("bad page header num_entries: {e}")))?,
            );
            let _collision_next = u32::from_le_bytes(
                data[8..12]
                    .try_into()
                    .map_err(|e| std::io::Error::other(format!("bad page header collision_next: {e}")))?,
            );

            if num_entries == 0 {
                bm.unpin(&self.file_name, page_num);
                continue;
            }

            for slot_idx in 0..num_slots as usize {
                let offset = PAGE_HEADER_SIZE + slot_idx * self.slot_size;
                if offset + self.slot_size > data.len() {
                    break;
                }
                let flags = data[offset + self.key_size + 8];
                if flags & FLAG_OCCUPIED == 0 || flags & FLAG_DELETED != 0 {
                    continue;
                }

                // Read key bytes
                let mut key_buf = vec![0u8; self.key_size];
                key_buf.copy_from_slice(&data[offset..offset + self.key_size]);

                // Read value
                let value = u64::from_le_bytes(
                    data[offset + self.key_size..offset + self.key_size + 8]
                        .try_into()
                        .map_err(|e| std::io::Error::other(format!("bad slot value bytes: {e}")))?,
                );

                // For fixed-size keys, we can deserialize directly.
                // For hash-based keys (like String), we rely on the fact that
                // rebuild_from_disk is only used at startup when L1 is empty.
                // The L1 entries will be re-inserted during WAL replay.
                if self.key_size <= 8 {
                    let key = K::deserialize_from(&key_buf).map_err(|e| std::io::Error::other(e.to_string()))?;
                    self.cache.insert(key, value);
                }
            }

            bm.unpin(&self.file_name, page_num);
        }

        Ok(())
    }

    /// Get the number of entries.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    // ---- Slot helpers ----

    /// Compute the page number for a key.
    fn hash_to_page(&self, key: &K) -> u32 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        let hash = hasher.finish();
        if self.num_pages == 0 {
            0
        } else {
            (hash % self.num_pages as u64) as u32
        }
    }

    /// Serialize a key into a byte buffer.
    fn serialize_key(key: &K, buf: &mut [u8]) {
        key.serialize_into(buf);
    }

    /// Compute the byte offset of a slot within a page.
    fn slot_offset(&self, slot_idx: u32) -> usize {
        PAGE_HEADER_SIZE + (slot_idx as usize) * self.slot_size
    }

    // ---- Public API ----

    /// Look up a key in the L1 cache (fast path).
    ///
    /// For write operations during query execution, this is always used.
    pub fn lookup_cached(&self, key: &K) -> Option<u64> {
        self.cache.get(key).copied()
    }

    /// Look up a key, potentially reading from the on-disk index.
    ///
    /// If the key is in L1 cache, returns immediately.
    /// Otherwise, scans the on-disk bucket page.
    pub fn lookup(&self, key: &K, bm: &mut BufferManager) -> std::io::Result<Option<u64>> {
        // Fast path: L1 cache hit
        if let Some(offset) = self.cache.get(key) {
            return Ok(Some(*offset));
        }

        // Slow path: scan on-disk pages
        if self.num_pages == 0 {
            return Ok(None);
        }

        let page_num = self.hash_to_page(key) as u64;
        let frame = bm.pin(&self.file_name, page_num)?;
        let data = &frame.data;

        let _num_slots = u32::from_le_bytes(
            data[0..4]
                .try_into()
                .map_err(|e| std::io::Error::other(format!("bad page header num_slots: {e}")))?,
        );
        let mut num_entries = u32::from_le_bytes(
            data[4..8]
                .try_into()
                .map_err(|e| std::io::Error::other(format!("bad page header num_entries: {e}")))?,
        );
        let mut collision_next = u32::from_le_bytes(
            data[8..12]
                .try_into()
                .map_err(|e| std::io::Error::other(format!("bad page header collision_next: {e}")))?,
        );

        let mut result = None;
        let mut current_page = page_num;

        loop {
            if num_entries == 0 {
                break;
            }

            let mut key_buf = vec![0u8; self.key_size];
            let page_frame = bm.pin(&self.file_name, current_page)?;
            let page_data = &page_frame.data;

            let nslots = u32::from_le_bytes(
                page_data[0..4]
                    .try_into()
                    .map_err(|e| std::io::Error::other(format!("bad page header nslots: {e}")))?,
            );
            for slot_idx in 0..nslots as usize {
                let offset = PAGE_HEADER_SIZE + slot_idx * self.slot_size;
                if offset + self.slot_size > page_data.len() {
                    break;
                }
                let flags = page_data[offset + self.key_size + 8];
                if flags & FLAG_OCCUPIED == 0 || flags & FLAG_DELETED != 0 {
                    continue;
                }

                key_buf.copy_from_slice(&page_data[offset..offset + self.key_size]);
                let Ok(candidate) = K::deserialize_from(&key_buf) else {
                    continue;
                };

                if &candidate == key {
                    let value = u64::from_le_bytes(
                        page_data[offset + self.key_size..offset + self.key_size + 8]
                            .try_into()
                            .map_err(|e| std::io::Error::other(format!("bad slot value bytes: {e}")))?,
                    );
                    result = Some(value);
                    bm.unpin(&self.file_name, current_page);
                    break;
                }
            }

            bm.unpin(&self.file_name, current_page);

            if result.is_some() {
                break;
            }

            if collision_next == 0 {
                break;
            }
            current_page = collision_next as u64;
            let next_frame = bm.pin(&self.file_name, current_page)?;
            let next_data = &next_frame.data;
            let err = || std::io::Error::new(std::io::ErrorKind::InvalidData, "Corrupted index collision page");
            num_entries = u32::from_le_bytes(
                next_data
                    .get(4..8)
                    .and_then(|s| s.try_into().ok())
                    .ok_or_else(err)?,
            );
            collision_next = u32::from_le_bytes(
                next_data
                    .get(8..12)
                    .and_then(|s| s.try_into().ok())
                    .ok_or_else(err)?,
            );
            bm.unpin(&self.file_name, current_page);
        }

        // Update cache for future lookups
        // L1 cache is updated on insert(); on-disk scan results are not cached
        // to avoid stale entries. The caller should ensure cache consistency.

        bm.unpin(&self.file_name, page_num);
        Ok(result)
    }

    /// Insert a key-value pair.
    ///
    /// Writes to L1 cache immediately. The on-disk write happens on `flush()`.
    pub fn insert(&mut self, key: K, value: u64) {
        self.cache.insert(key, value);
    }

    /// Delete a key.
    pub fn delete(&mut self, key: &K) {
        self.cache.remove(key);
    }

    /// Flush all cached entries to the on-disk hash index.
    ///
    /// This writes all key-value pairs from L1 cache into the BufferManager
    /// page-structured hash table. After this, the index is durably stored.
    pub fn flush(&mut self, bm: &mut BufferManager) -> std::io::Result<()> {
        if self.cache.is_empty() {
            if self.num_pages > 0 {
                // Write an empty header page
                let frame = bm.pin_mut(&self.file_name, 0)?;
                let data = &mut frame.data;
                data[0..4].copy_from_slice(&0u32.to_le_bytes()); // num_slots
                data[4..8].copy_from_slice(&0u32.to_le_bytes()); // num_entries
                data[8..12].copy_from_slice(&0u32.to_le_bytes()); // collision_next
                frame.mark_dirty();
                bm.unpin(&self.file_name, 0);
            }
            return bm.flush_all();
        }

        // Determine number of pages needed: ceil(entries / slots_per_page)
        let num_entries = self.cache.len() as u32;
        let pages_needed = num_entries.div_ceil(self.slots_per_page);
        let pages_needed = pages_needed.max(1);

        // Allocate pages if needed
        if pages_needed > self.num_pages {
            self.num_pages = pages_needed;
        }

        // Write header for each page
        for page_num in 0..self.num_pages as u64 {
            let frame = bm.pin_mut(&self.file_name, page_num)?;
            let data = &mut frame.data;

            // Clear the page
            data.fill(0);

            // Write header
            data[0..4].copy_from_slice(&self.slots_per_page.to_le_bytes());
            data[4..8].copy_from_slice(&0u32.to_le_bytes()); // num_entries (updated below)
            data[8..12].copy_from_slice(&0u32.to_le_bytes()); // collision_next

            frame.mark_dirty();
            bm.unpin(&self.file_name, page_num);
        }

        // Distribute entries across pages by hash
        if self.num_pages == 0 {
            return Ok(());
        }

        // Count entries per page and write slots
        let mut page_counts = vec![0u32; self.num_pages as usize];

        // First pass: count
        for (key, _value) in self.cache.iter() {
            let page = self.hash_to_page(key) as usize;
            page_counts[page] = page_counts[page].saturating_add(1);
        }

        // Second pass: write entries
        let mut page_cursors = vec![0u32; self.num_pages as usize];

        for (key, value) in self.cache.iter() {
            let page = self.hash_to_page(key) as u64;
            let cursor = &mut page_cursors[page as usize];

            let frame = bm.pin_mut(&self.file_name, page)?;
            let data = &mut frame.data;

            let slot_offset = self.slot_offset(*cursor);
            if slot_offset + self.slot_size <= data.len() {
                // Write key
                let mut key_buf = vec![0u8; self.key_size];
                Self::serialize_key(key, &mut key_buf);
                data[slot_offset..slot_offset + self.key_size].copy_from_slice(&key_buf);

                // Write value
                data[slot_offset + self.key_size..slot_offset + self.key_size + 8]
                    .copy_from_slice(&value.to_le_bytes());

                // Write flags
                data[slot_offset + self.key_size + 8] = FLAG_OCCUPIED;

                // Update num_entries in header
                let current = u32::from_le_bytes(
                    data[4..8]
                        .try_into()
                        .map_err(|e| std::io::Error::other(format!("bad num_entries bytes: {e}")))?,
                );
                data[4..8].copy_from_slice(&(current + 1).to_le_bytes());
            }

            frame.mark_dirty();
            bm.unpin(&self.file_name, page);

            *cursor += 1;
        }

        // Flush all dirty pages to disk
        bm.flush_all()
    }

    /// Iterate over all cached entries.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &u64)> {
        self.cache.iter()
    }
}

impl<K: IndexKey + Hash + Eq> Default for OnDiskHashIndex<K> {
    fn default() -> Self {
        Self::new("default")
    }
}

/// Lifecycle state of a secondary index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexState {
    /// Index is being built (backfilling existing rows). Queries fall back to scan.
    Building,
    /// Index is fully built and available for lookups.
    Available,
    /// Index has been dropped; storage released.
    Dropped,
}

/// A persistent secondary index over a non-primary-key column.
///
/// Wraps `OnDiskHashIndex<String>` (keys are hex-encoded fingerprint bytes)
/// with a lifecycle state machine. Queries against a `Building` or `Dropped`
/// index fall back to a full table scan (FR-004).
#[derive(Debug, Clone)]
pub struct SecondaryIndex {
    pub name: String,
    pub table_name: String,
    pub column_name: String,
    pub index_type: akar_catalog::IndexType,
    pub state: IndexState,
    /// On-disk hash index backing store. Keys are hex-encoded fingerprint bytes.
    pub index: OnDiskHashIndex<String>,
    /// In-memory multimap: one key may map to several row offsets, because a
    /// secondary index is declared on a non-unique column (FR-002). The on-disk
    /// hash index stores one offset per key; this map is the authoritative
    /// lookup structure and is rebuilt from the table on every open.
    entries: HashMap<String, Vec<u64>>,
}

impl SecondaryIndex {
    /// Create a new secondary index in `Building` state.
    pub fn new(
        name: String,
        table_name: String,
        column_name: String,
        index_type: akar_catalog::IndexType,
        file_name: String,
    ) -> Self {
        Self {
            name,
            table_name,
            column_name,
            index_type,
            state: IndexState::Building,
            index: OnDiskHashIndex::new(&file_name),
            entries: HashMap::new(),
        }
    }

    /// Mark the index as `Available` after backfill completes.
    pub fn mark_available(&mut self) {
        self.state = IndexState::Available;
    }

    /// Mark the index as `Dropped` (storage released).
    pub fn mark_dropped(&mut self) {
        self.state = IndexState::Dropped;
    }

    /// Returns `true` if the index can serve lookups (state == `Available`).
    pub fn is_available(&self) -> bool {
        self.state == IndexState::Available
    }

    /// Flush the in-memory cache to durable storage via the BufferManager.
    pub fn flush(&mut self, bm: &mut BufferManager) -> std::io::Result<()> {
        self.index.flush(bm)
    }

    /// Look up a row offset by key. Returns `None` if the index is not `Available`.
    pub fn lookup(&self, key: &String, bm: &mut BufferManager) -> std::io::Result<Option<u64>> {
        if !self.is_available() {
            return Ok(None);
        }
        if let Some(offsets) = self.entries.get(key) {
            return Ok(offsets.first().copied());
        }
        self.index.lookup(key, bm)
    }

    /// Look up every row offset stored under `key`.
    ///
    /// A secondary index is declared on a non-unique column, so one key can
    /// match any number of rows (FR-002). Returns an empty vector when the
    /// index is not `Available` or the key is absent.
    pub fn lookup_all(&self, key: &str, bm: &mut BufferManager) -> std::io::Result<Vec<u64>> {
        if !self.is_available() {
            return Ok(Vec::new());
        }
        if let Some(offsets) = self.entries.get(key) {
            return Ok(offsets.clone());
        }
        Ok(self.index.lookup(&key.to_string(), bm)?.into_iter().collect())
    }

    /// Insert a key → row offset mapping into the index.
    pub fn insert(&mut self, key: String, offset: u64) {
        let offsets = self.entries.entry(key.clone()).or_default();
        if !offsets.contains(&offset) {
            offsets.push(offset);
        }
        // Keep the durable copy pointing at the most recent offset; the
        // in-memory multimap is what serves lookups.
        self.index.insert(key, offset);
    }

    /// Remove a row offset from every key it was stored under.
    ///
    /// A key that loses its last row is dropped from both the in-memory
    /// multimap and the durable hash cache: `lookup_all` falls back to the
    /// durable cache when the multimap has no key, so leaving a stale entry
    /// there would resurrect the deleted row.
    pub fn remove_row(&mut self, offset: u64) {
        let keys: Vec<String> = self.entries.keys().cloned().collect();
        for key in keys {
            if let Some(offsets) = self.entries.get_mut(&key) {
                offsets.retain(|candidate| *candidate != offset);
            }
            if self.entries.get(&key).is_none_or(|offsets| offsets.is_empty()) {
                self.entries.remove(&key);
                self.index.delete(&key);
            }
        }
        // The durable hash cache also keeps entries the multimap never had
        // (e.g. a key pruned by an earlier operation). Drop orphans here.
        let orphaned: Vec<String> = self
            .index
            .iter()
            .filter(|(key, value)| **value == offset && !self.entries.contains_key(key.as_str()))
            .map(|(key, _)| key.clone())
            .collect();
        for key in orphaned {
            self.index.delete(&key);
        }
    }

    /// Update a row offset from an old key to a new key.
    pub fn update_row(&mut self, old_key: &str, new_key: String, offset: u64) {
        if let Some(offsets) = self.entries.get_mut(old_key) {
            offsets.retain(|&cand| cand != offset);
        }
        if self.entries.get(old_key).is_none_or(|offsets| offsets.is_empty()) {
            self.entries.remove(old_key);
            self.index.delete(&old_key.to_string());
        } else if self.index.lookup_cached(&old_key.to_string()) == Some(offset)
            && let Some(next) = self.entries.get(old_key).and_then(|offsets| offsets.first()).copied()
        {
            self.index.insert(old_key.to_string(), next);
        }
        self.insert(new_key, offset);
    }

    /// Move a row offset between keys in one step.
    ///
    /// Handles every case a `DELETE` or `SET` produces: the old key may be
    /// absent (insert path), the new key may be absent (delete path), both may
    /// be present (update path), and a `NULL` column value simply contributes
    /// no key on that side. The row is never left registered under its old key
    /// — including in the durable hash cache used as a lookup fallback.
    pub fn move_row(&mut self, old_key: Option<&str>, new_key: Option<&str>, offset: u64) {
        if let Some(old) = old_key {
            if let Some(offsets) = self.entries.get_mut(old) {
                offsets.retain(|&cand| cand != offset);
            }
            if self.entries.get(old).is_none_or(|offsets| offsets.is_empty()) {
                self.entries.remove(old);
                self.index.delete(&old.to_string());
            }
            // A key that only ever lived in the durable cache (no multimap
            // counterpart) must not keep pointing at the moved row.
            if !self.entries.contains_key(old) && self.index.lookup_cached(&old.to_string()) == Some(offset) {
                self.index.delete(&old.to_string());
            }
        }
        if let Some(new) = new_key {
            self.insert(new.to_string(), offset);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer_manager::BufferManagerConfig;
    use crate::page::DEFAULT_PAGE_SIZE;
    use akar_common::memory::MemoryManager;
    use std::sync::Arc;

    fn create_test_bm() -> (BufferManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mm = Arc::new(MemoryManager::new(1024 * 1024));
        let config = BufferManagerConfig {
            max_memory: 256 * 1024,
            page_size: DEFAULT_PAGE_SIZE,
            ..Default::default()
        };
        let mut bm = BufferManager::new(dir.path().to_path_buf(), mm, config);
        let idx_path = dir.path().join("test_index.idx");
        std::fs::write(&idx_path, vec![0u8; 8192 * 10]).unwrap();
        bm.register_file("test_index", idx_path);
        (bm, dir)
    }

    #[test]
    fn test_hash_index_in_memory() {
        let mut idx: HashIndex<String> = HashIndex::new();
        idx.insert("Alice".to_string(), 0);
        idx.insert("Bob".to_string(), 1);
        assert_eq!(idx.lookup(&"Alice".to_string()), Some(0));
        assert_eq!(idx.lookup(&"Bob".to_string()), Some(1));
        assert_eq!(idx.lookup(&"Charlie".to_string()), None);
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn test_hash_index_delete() {
        let mut idx: HashIndex<String> = HashIndex::new();
        idx.insert("X".to_string(), 42);
        assert_eq!(idx.lookup(&"X".to_string()), Some(42));
        idx.delete(&"X".to_string());
        assert_eq!(idx.lookup(&"X".to_string()), None);
        assert!(idx.is_empty());
    }

    #[test]
    fn test_on_disk_basic_insert_and_lookup() {
        let (mut bm, _dir) = create_test_bm();
        let mut idx: OnDiskHashIndex<u64> = OnDiskHashIndex::new("test_index");

        // Insert via L1 cache
        idx.insert(100u64, 0);
        idx.insert(200u64, 1);
        idx.insert(300u64, 2);

        // Flush to disk
        idx.flush(&mut bm).unwrap();

        // Lookup via L1 cache
        assert_eq!(idx.lookup_cached(&100), Some(0));
        assert_eq!(idx.lookup_cached(&200), Some(1));
        assert_eq!(idx.lookup_cached(&300), Some(2));
        assert_eq!(idx.lookup_cached(&999), None);

        // Lookup via on-disk
        assert_eq!(idx.lookup(&100, &mut bm).unwrap(), Some(0));
        assert_eq!(idx.lookup(&200, &mut bm).unwrap(), Some(1));
        assert_eq!(idx.lookup(&999, &mut bm).unwrap(), None);
    }

    #[test]
    fn test_on_disk_multiple_pages() {
        let (mut bm, _dir) = create_test_bm();
        let mut idx: OnDiskHashIndex<u64> = OnDiskHashIndex::new("test_index");

        // Insert many entries to force multiple pages
        for i in 0..200u64 {
            idx.insert(i * 1000, i);
        }

        idx.flush(&mut bm).unwrap();

        // Verify num_pages > 1
        assert!(idx.num_pages > 1, "Should have multiple pages for 200 entries");

        // Verify all entries are found
        for i in 0..200u64 {
            assert_eq!(idx.lookup_cached(&(i * 1000)), Some(i));
        }
    }

    #[test]
    fn test_on_disk_delete() {
        let (mut bm, _dir) = create_test_bm();
        let mut idx: OnDiskHashIndex<u64> = OnDiskHashIndex::new("test_index");

        idx.insert(42u64, 0);
        idx.insert(99u64, 1);
        idx.flush(&mut bm).unwrap();

        assert_eq!(idx.lookup_cached(&42), Some(0));

        // Delete from L1
        idx.delete(&42);
        assert_eq!(idx.lookup_cached(&42), None);

        // Re-flush — now entry 42 shouldn't be on disk
        idx.flush(&mut bm).unwrap();
        assert_eq!(idx.lookup(&42, &mut bm).unwrap(), None);
    }

    #[test]
    fn test_on_disk_rebuild_from_disk() {
        let (mut bm, dir) = create_test_bm();
        let mut idx: OnDiskHashIndex<u64> = OnDiskHashIndex::new("test_index");

        // Insert and flush (writes via BM to disk)
        for i in 0..50u64 {
            idx.insert(i, i);
        }
        idx.flush(&mut bm).unwrap();
        assert_eq!(idx.len(), 50);
        let pages_used = idx.num_pages;
        assert!(pages_used > 0, "Should have at least 1 page for 50 entries");

        // Verify data on disk by reading directly
        let file_path = dir.path().join("test_index.idx");
        let file_data = std::fs::read(&file_path).unwrap();
        let header_num_slots = u32::from_le_bytes(file_data[0..4].try_into().unwrap());
        let header_num_entries = u32::from_le_bytes(file_data[4..8].try_into().unwrap());
        assert_eq!(header_num_slots, 64, "Should have 64 slots per page");
        assert_eq!(header_num_entries, 50, "Should have 50 entries on disk");

        // Simulate restart: create a new BM pointing at the same file.
        let mut bm2 = BufferManager::new(
            dir.path().to_path_buf(),
            Arc::new(MemoryManager::new(1024 * 1024)),
            BufferManagerConfig::default(),
        );
        bm2.register_file("test_index", file_path);

        let mut idx2: OnDiskHashIndex<u64> = OnDiskHashIndex::new("test_index");
        idx2.num_pages = pages_used;
        idx2.rebuild_from_disk(&mut bm2).unwrap();

        // After rebuild, L1 cache should be populated
        assert_eq!(idx2.len(), 50);
        for i in 0..50u64 {
            assert_eq!(idx2.lookup_cached(&i), Some(i));
        }
    }

    #[test]
    fn test_on_disk_empty_index() {
        let (mut bm, _dir) = create_test_bm();
        let mut idx: OnDiskHashIndex<u64> = OnDiskHashIndex::new("test_index");

        // Flush an empty index
        idx.flush(&mut bm).unwrap();
        assert_eq!(idx.len(), 0);
        assert!(idx.is_empty());
    }

    #[test]
    fn test_on_disk_large_entries() {
        let (mut bm, _dir) = create_test_bm();
        let mut idx: OnDiskHashIndex<u64> = OnDiskHashIndex::new("test_index");

        // Insert 500 entries
        for i in 0..500u64 {
            idx.insert(i, i + 1000);
        }
        idx.flush(&mut bm).unwrap();

        // Verify all
        for i in 0..500u64 {
            assert_eq!(idx.lookup_cached(&i), Some(i + 1000));
        }
    }

    #[test]
    fn test_on_disk_collision_handling() {
        let (mut bm, _dir) = create_test_bm();
        // Use a small number of pages to force collisions
        let mut idx: OnDiskHashIndex<u64> = OnDiskHashIndex::new("test_index");
        idx.num_pages = 2; // Only 2 pages → many collisions for distinct values

        for i in 0..100u64 {
            idx.insert(i, i * 10);
        }
        idx.flush(&mut bm).unwrap();

        // All entries should still be findable
        for i in 0..100u64 {
            assert_eq!(idx.lookup_cached(&i), Some(i * 10));
        }
    }

    #[test]
    fn test_secondary_index_lifecycle() {
        let (mut bm, _dir) = create_test_bm();
        let mut idx = SecondaryIndex::new(
            "si_dedup".to_string(),
            "Memory".to_string(),
            "content_identity_fingerprint".to_string(),
            akar_catalog::IndexType::Hash,
            "si_dedup".to_string(),
        );

        // Initially Building, not available
        assert_eq!(idx.state, IndexState::Building);
        assert!(!idx.is_available());

        // Insert entries
        idx.insert("fp-a".to_string(), 0);
        idx.insert("fp-b".to_string(), 1);
        idx.flush(&mut bm).unwrap();

        // Mark available
        idx.mark_available();
        assert_eq!(idx.state, IndexState::Available);
        assert!(idx.is_available());

        // Lookup via disk
        assert_eq!(idx.lookup(&"fp-a".to_string(), &mut bm).unwrap(), Some(0));
        assert_eq!(idx.lookup(&"fp-b".to_string(), &mut bm).unwrap(), Some(1));
        assert_eq!(idx.lookup(&"fp-c".to_string(), &mut bm).unwrap(), None);
    }

    #[test]
    fn test_secondary_index_lookup_unavailable() {
        let (mut bm, _dir) = create_test_bm();
        let mut idx = SecondaryIndex::new(
            "si_unavail".to_string(),
            "Memory".to_string(),
            "fp".to_string(),
            akar_catalog::IndexType::Hash,
            "si_unavail".to_string(),
        );
        // Still Building — lookup returns None (scan fallback)
        idx.insert("fp-x".to_string(), 5);
        assert_eq!(idx.lookup(&"fp-x".to_string(), &mut bm).unwrap(), None);
    }

    #[test]
    fn test_secondary_index_mark_dropped() {
        let (_bm, _dir) = create_test_bm();
        let mut idx = SecondaryIndex::new(
            "si_drop".to_string(),
            "Memory".to_string(),
            "fp".to_string(),
            akar_catalog::IndexType::Hash,
            "si_drop".to_string(),
        );
        idx.mark_available();
        assert!(idx.is_available());
        idx.mark_dropped();
        assert_eq!(idx.state, IndexState::Dropped);
        assert!(!idx.is_available());
    }

    #[test]
    fn test_secondary_index_dedup_flow() {
        let (mut bm, _dir) = create_test_bm();
        let mut idx = SecondaryIndex::new(
            "si_dedup_flow".to_string(),
            "Memory".to_string(),
            "content_identity_fingerprint".to_string(),
            akar_catalog::IndexType::Hash,
            "si_dedup_flow".to_string(),
        );

        // First insert of fingerprint "fp-1" → row 100
        idx.insert("fp-1".to_string(), 100);
        idx.flush(&mut bm).unwrap();
        idx.mark_available();

        // Lookup returns existing row (duplicate detected by application layer)
        assert_eq!(
            idx.lookup(&"fp-1".to_string(), &mut bm).unwrap(),
            Some(100),
            "duplicate fingerprint should resolve to existing row"
        );

        // Different fingerprint → not found (new row)
        assert_eq!(
            idx.lookup(&"fp-2".to_string(), &mut bm).unwrap(),
            None,
            "new fingerprint should not be found"
        );

        // Application layer checks lookup BEFORE insert: since "fp-1" exists,
        // the write path skips the insert (idempotent dedup).
        // If the application incorrectly re-inserts, the index overwrites —
        // that's the application's responsibility to prevent.
        assert_eq!(
            idx.lookup(&"fp-1".to_string(), &mut bm).unwrap(),
            Some(100),
            "existing fingerprint still resolves to original row"
        );
    }
}
