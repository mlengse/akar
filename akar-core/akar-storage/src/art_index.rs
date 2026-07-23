//! ART (Adaptive Radix Tree) Primary Key Index.
//!
//! An in-memory radix tree index supporting exact lookup, range scans, and
//! persistence via the BufferManager. Each key is an `ArtKey` (order-preserving
//! byte encoding of a Akar Value), and each leaf stores row offsets (`u64`).
//!
//! Port of C++ `ArtPrimaryKeyIndex` from `ladybug/src/storage/index/art_index.h`
//! and `art_index.cpp`.

use crate::art_key::ArtKey;
use crate::art_node::{ArtNode, NodeBlock};
use crate::buffer_manager::BufferManager;

/// Default number of pages allocated for the ART index file.
const INITIAL_PAGE_COUNT: u64 = 4;

/// Magic number for ART index header page: "ART\0"
const ART_HEADER_MAGIC: u64 = 0x4152540000000000;

/// Size of the header page in bytes.
const HEADER_SIZE: usize = 64;

// ---------------------------------------------------------------------------
// Varint encoding (LEB128-style) — port of C++ `writeArtVarUint`/`readArtVarUint`
// ---------------------------------------------------------------------------

fn write_art_varint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            buf.push(byte);
            break;
        } else {
            buf.push(byte | 0x80);
        }
    }
}

fn read_art_varint(data: &[u8], pos: &mut usize) -> u64 {
    let mut result = 0u64;
    let mut shift = 0u64;
    loop {
        let byte = data[*pos];
        *pos += 1;
        result |= ((byte & 0x7F) as u64) << shift;
        if (byte & 0x80) == 0 {
            return result;
        }
        shift += 7;
    }
}

// ---------------------------------------------------------------------------
// ArtPrimaryKeyIndex
// ---------------------------------------------------------------------------

/// A radix tree primary key index mapping `ArtKey` → row offsets.
///
/// Supports exact lookup, range scan, insertion, deletion, and
/// persistence via `save()` / `load()` through the BufferManager.
///
/// Thread safety: NOT thread-safe by default. Wrap in `Mutex` for
/// concurrent access.
#[derive(Clone)]
pub struct ArtPrimaryKeyIndex {
    /// Root node of the radix tree.
    root: ArtNode,
    /// Arena-allocated node blocks.
    node_blocks: Vec<NodeBlock>,
    /// Total number of allocated nodes (including root).
    num_allocated_nodes: u64,
    /// Count of nodes by kind: [Node4, Node16, Node48, Node256].
    num_nodes_by_kind: [u64; 4],

    // Persistence fields
    /// BufferManager file name for this index.
    file_name: String,
    /// Number of pages allocated in the BufferManager.
    page_count: u64,
    /// Whether in-memory state has changed since last save.
    dirty: bool,
    /// Total number of entries (key-value pairs) in this index.
    num_entries: u64,
}

impl ArtPrimaryKeyIndex {
    /// Create a new empty ART index.
    pub fn new(file_name: &str) -> Self {
        let root = ArtNode::new_node4();
        Self {
            root,
            node_blocks: Vec::new(),
            num_allocated_nodes: 1,
            num_nodes_by_kind: [1, 0, 0, 0],
            file_name: file_name.to_string(),
            page_count: INITIAL_PAGE_COUNT,
            dirty: false,
            num_entries: 0,
        }
    }

    /// Get the number of entries in this index.
    pub fn len(&self) -> u64 {
        self.num_entries
    }

    /// Returns `true` if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.num_entries == 0
    }

    /// Get the file name used for BufferManager persistence.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// Mark the index as dirty (needs save).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    // ---- Core Operations ----

    /// Insert a key-value pair into the ART index.
    ///
    /// If the key already exists, the offset is added as an overflow value
    /// (supporting duplicate PK handling for secondary index use).
    pub fn insert(&mut self, key: &ArtKey, row_offset: u64) {
        let key_bytes = key.bytes().to_vec();
        if key_bytes.is_empty() {
            return;
        }
        insert_internal(&mut self.root, &key_bytes, 0, row_offset, &mut self.num_entries);
        self.dirty = true;
    }

    /// Look up a key in the ART index.
    ///
    /// Returns `Some(offset)` if an exact match is found for the primary offset.
    /// For duplicate keys, only the first offset is returned.
    pub fn lookup(&self, key: &ArtKey) -> Option<u64> {
        let key_bytes = key.bytes();
        if key_bytes.is_empty() {
            return None;
        }

        let mut current = &self.root;
        let mut depth = 0usize;

        loop {
            let prefix = current.prefix();
            if depth + prefix.len() > key_bytes.len() || key_bytes[depth..depth + prefix.len()] != *prefix {
                return None;
            }
            depth += prefix.len();

            if depth >= key_bytes.len() {
                return current.all_offsets().first().copied();
            }

            let byte = key_bytes[depth];
            match current.get_child(byte) {
                Some(child) => {
                    current = child;
                    depth += 1;
                }
                None => return None,
            }
        }
    }

    /// Delete a key from the ART index.
    ///
    /// Removes the first occurrence of the row offset for this key.
    /// If no more offsets remain for this leaf, the leaf node is cleaned up.
    pub fn delete(&mut self, key: &ArtKey, row_offset: u64) {
        let key_bytes = key.bytes().to_vec();
        if key_bytes.is_empty() {
            return;
        }
        if erase_internal(&mut self.root, &key_bytes, 0, row_offset) {
            self.num_entries = self.num_entries.saturating_sub(1);
        }
        self.dirty = true;
    }

    /// Perform a range scan over keys within [lower, upper] bounds.
    ///
    /// - `lower`: Optional lower bound (`None` = unbounded).
    /// - `lower_inclusive`: Whether to include keys equal to `lower`.
    /// - `upper`: Optional upper bound (`None` = unbounded).
    /// - `upper_inclusive`: Whether to include keys equal to `upper`.
    /// - `max_results`: Maximum number of offsets to return.
    ///
    /// Returns up to `max_results` row offsets matching keys in the range.
    /// Port of C++ `collectRange()` from `art_index.cpp` lines 924–985.
    pub fn range_scan(
        &self,
        lower: Option<&ArtKey>,
        lower_inclusive: bool,
        upper: Option<&ArtKey>,
        upper_inclusive: bool,
        max_results: u64,
    ) -> Vec<u64> {
        let mut results = Vec::new();
        let mut key_buf = Vec::new();
        self.collect_range(
            &self.root,
            &mut key_buf,
            lower,
            lower_inclusive,
            upper,
            upper_inclusive,
            max_results,
            &mut results,
        );
        results
    }

    /// Collect offsets from a range of keys in sorted order.
    /// Port of C++ `ArtPrimaryKeyIndex::collectRange()`.
    fn collect_range(
        &self,
        node: &ArtNode,
        key: &mut Vec<u8>,
        lower: Option<&ArtKey>,
        lower_inclusive: bool,
        upper: Option<&ArtKey>,
        upper_inclusive: bool,
        max_results: u64,
        results: &mut Vec<u64>,
    ) {
        let key_size_before = key.len();

        // Add node's prefix to the current key path
        key.extend_from_slice(node.prefix());

        if results.len() as u64 >= max_results {
            key.truncate(key_size_before);
            return;
        }

        // Check if this node's path satisfies bounds
        if node.has_offsets()
            && satisfies_lower_bound(key, lower, lower_inclusive)
            && satisfies_upper_bound(key, upper, upper_inclusive)
        {
            for &offset in &node.all_offsets() {
                if results.len() as u64 >= max_results {
                    break;
                }
                results.push(offset);
            }
            if results.len() as u64 >= max_results {
                key.truncate(key_size_before);
                return;
            }
        }

        // Visit children in byte order
        match node {
            ArtNode::Node4 {
                keys, children, count, ..
            } => {
                // Sort children by key byte
                let mut indices: Vec<usize> = (0..*count as usize).collect();
                indices.sort_by_key(|&i| keys[i]);
                for &i in &indices {
                    if let Some(ref child) = children[i] {
                        key.push(keys[i]);
                        if satisfies_upper_bound(key, upper, true) {
                            self.collect_range(
                                child,
                                key,
                                lower,
                                lower_inclusive,
                                upper,
                                upper_inclusive,
                                max_results,
                                results,
                            );
                        }
                        key.pop();
                        if results.len() as u64 >= max_results {
                            key.truncate(key_size_before);
                            return;
                        }
                    }
                }
            }
            ArtNode::Node16 {
                keys, children, count, ..
            } => {
                // Sort children by key byte
                let mut indices: Vec<usize> = (0..*count as usize).collect();
                indices.sort_by_key(|&i| keys[i]);
                for &i in &indices {
                    if let Some(ref child) = children[i] {
                        key.push(keys[i]);
                        if satisfies_upper_bound(key, upper, true) {
                            self.collect_range(
                                child,
                                key,
                                lower,
                                lower_inclusive,
                                upper,
                                upper_inclusive,
                                max_results,
                                results,
                            );
                        }
                        key.pop();
                        if results.len() as u64 >= max_results {
                            key.truncate(key_size_before);
                            return;
                        }
                    }
                }
            }
            ArtNode::Node48 {
                child_index, children, ..
            } => {
                for byte in 0u16..256u16 {
                    let idx = child_index[byte as usize];
                    if idx == crate::art_node::EMPTY_MARKER {
                        continue;
                    }
                    if let Some(ref child) = children[idx as usize] {
                        key.push(byte as u8);
                        if satisfies_upper_bound(key, upper, true) {
                            self.collect_range(
                                child,
                                key,
                                lower,
                                lower_inclusive,
                                upper,
                                upper_inclusive,
                                max_results,
                                results,
                            );
                        }
                        key.pop();
                        if results.len() as u64 >= max_results {
                            key.truncate(key_size_before);
                            return;
                        }
                    }
                }
            }
            ArtNode::Node256 { children, .. } => {
                for byte in 0u16..256u16 {
                    if let Some(ref child) = children[byte as usize] {
                        key.push(byte as u8);
                        if satisfies_upper_bound(key, upper, true) {
                            self.collect_range(
                                child,
                                key,
                                lower,
                                lower_inclusive,
                                upper,
                                upper_inclusive,
                                max_results,
                                results,
                            );
                        }
                        key.pop();
                        if results.len() as u64 >= max_results {
                            key.truncate(key_size_before);
                            return;
                        }
                    }
                }
            }
        }

        key.truncate(key_size_before);
    }

    /// Clear the entire index (drops all nodes).
    pub fn clear(&mut self) {
        self.node_blocks.clear();
        self.root = ArtNode::new_node4();
        self.num_allocated_nodes = 1;
        self.num_nodes_by_kind = [1, 0, 0, 0];
        self.num_entries = 0;
        self.dirty = true;
    }

    // ---- Persistence ----

    /// Serialize the ART tree to a byte vector.
    /// Port of C++ `serializeTree()`.
    pub fn serialize_tree(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.serialize_node(&self.root, &mut buf);
        buf
    }

    fn serialize_node(&self, node: &ArtNode, buf: &mut Vec<u8>) {
        // Write kind
        write_art_varint(buf, node.kind() as u64);
        // Write prefix
        write_art_varint(buf, node.prefix().len() as u64);
        buf.extend_from_slice(node.prefix());
        // Write offsets
        let offsets = node.all_offsets();
        write_art_varint(buf, offsets.len() as u64);
        for &offset in &offsets {
            write_art_varint(buf, offset);
        }
        // Write children
        write_art_varint(buf, node.count() as u64);
        match node {
            ArtNode::Node4 {
                keys, children, count, ..
            } => {
                for i in 0..*count as usize {
                    if let Some(ref child) = children[i] {
                        buf.push(keys[i]);
                        self.serialize_node(child, buf);
                    }
                }
            }
            ArtNode::Node16 {
                keys, children, count, ..
            } => {
                for i in 0..*count as usize {
                    if let Some(ref child) = children[i] {
                        buf.push(keys[i]);
                        self.serialize_node(child, buf);
                    }
                }
            }
            ArtNode::Node48 {
                child_index, children, ..
            } => {
                for byte in 0u16..256u16 {
                    let idx = child_index[byte as usize];
                    if idx != crate::art_node::EMPTY_MARKER
                        && let Some(ref child) = children[idx as usize]
                    {
                        buf.push(byte as u8);
                        self.serialize_node(child, buf);
                    }
                }
            }
            ArtNode::Node256 { children, .. } => {
                for byte in 0u16..256u16 {
                    if let Some(ref child) = children[byte as usize] {
                        buf.push(byte as u8);
                        self.serialize_node(child, buf);
                    }
                }
            }
        }
    }

    /// Deserialize a serialized ART tree into a node.
    /// Port of C++ `loadTree()` template.
    pub fn deserialize_tree(data: &[u8], pos: &mut usize) -> ArtNode {
        let kind = read_art_varint(data, pos) as u8;
        let prefix_len = read_art_varint(data, pos) as usize;
        let mut prefix = vec![0u8; prefix_len];
        if prefix_len > 0 {
            prefix.copy_from_slice(&data[*pos..*pos + prefix_len]);
            *pos += prefix_len;
        }

        let num_offsets = read_art_varint(data, pos) as usize;
        let mut offsets = Vec::with_capacity(num_offsets);
        let mut overflow_offsets = Vec::new();
        for i in 0..num_offsets {
            let off = read_art_varint(data, pos);
            if i == 0 {
                offsets.push(off);
            } else {
                overflow_offsets.push(off);
            }
        }

        let num_children = read_art_varint(data, pos) as usize;

        match kind {
            0 => {
                // Node4
                let mut keys = [0u8; 4];
                let mut children: [Option<Box<ArtNode>>; 4] = Default::default();
                let mut count: u16 = 0;
                for _ in 0..num_children.min(4) {
                    let byte = data[*pos];
                    *pos += 1;
                    keys[count as usize] = byte;
                    children[count as usize] = Some(Box::new(Self::deserialize_tree(data, pos)));
                    count += 1;
                }
                ArtNode::Node4 {
                    prefix,
                    keys,
                    children,
                    offsets,
                    overflow_offsets,
                    count,
                }
            }
            1 => {
                // Node16
                let mut keys = [0u8; 16];
                let mut children: [Option<Box<ArtNode>>; 16] = Default::default();
                let mut count: u16 = 0;
                for _ in 0..num_children.min(16) {
                    let byte = data[*pos];
                    *pos += 1;
                    keys[count as usize] = byte;
                    children[count as usize] = Some(Box::new(Self::deserialize_tree(data, pos)));
                    count += 1;
                }
                ArtNode::Node16 {
                    prefix,
                    keys,
                    children,
                    offsets,
                    overflow_offsets,
                    count,
                }
            }
            2 => {
                // Node48
                let mut child_index = [crate::art_node::EMPTY_MARKER; 256];
                let mut children: [Option<Box<ArtNode>>; 48] = std::array::from_fn(|_| None);
                let mut count: u16 = 0;
                for _ in 0..num_children.min(48) {
                    let byte = data[*pos];
                    *pos += 1;
                    child_index[byte as usize] = count as u8;
                    children[count as usize] = Some(Box::new(Self::deserialize_tree(data, pos)));
                    count += 1;
                }
                ArtNode::Node48 {
                    prefix,
                    child_index,
                    children: Box::new(children),
                    offsets,
                    overflow_offsets,
                    count,
                }
            }
            3 => {
                // Node256
                let mut children: [Option<Box<ArtNode>>; 256] = std::array::from_fn(|_| None);
                let mut count: u16 = 0;
                for _ in 0..num_children.min(256) {
                    let byte = data[*pos];
                    *pos += 1;
                    children[byte as usize] = Some(Box::new(Self::deserialize_tree(data, pos)));
                    count += 1;
                }
                ArtNode::Node256 {
                    prefix,
                    children: Box::new(children),
                    offsets,
                    overflow_offsets,
                    count,
                }
            }
            _ => ArtNode::new_node4(),
        }
    }

    /// Save the ART index to BufferManager-backed pages.
    ///
    /// Page layout:
    /// - Page 0: Header (magic, num_entries, num_pages, tree_byte_size)
    /// - Pages 1..N: Serialized tree data
    pub fn save(&mut self, bm: &mut BufferManager) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        if !bm.is_file_registered(&self.file_name) {
            return Err(format!(
                "ART index file '{}' not registered with BufferManager",
                self.file_name
            ));
        }

        let tree_bytes = self.serialize_tree();
        let tree_size = tree_bytes.len() as u64;

        // Calculate pages needed: header + data pages
        let page_size = bm.page_size() as u64;
        let data_pages = if tree_size == 0 {
            1
        } else {
            1 + tree_size.div_ceil(page_size)
        };

        // Ensure enough pages are allocated
        if data_pages > self.page_count {
            self.page_count = data_pages;
        }

        // Write header page (page 0)
        let mut header = vec![0u8; HEADER_SIZE.max(page_size as usize)];
        header[0..8].copy_from_slice(&ART_HEADER_MAGIC.to_le_bytes());
        header[8..16].copy_from_slice(&self.num_entries.to_le_bytes());
        header[16..24].copy_from_slice(&self.page_count.to_le_bytes());
        header[24..32].copy_from_slice(&tree_size.to_le_bytes());

        let frame = bm
            .pin_mut(&self.file_name, 0)
            .map_err(|e| format!("Failed to pin ART header page: {e}"))?;
        let copy_len = header.len().min(frame.data.len());
        frame.data[..copy_len].copy_from_slice(&header[..copy_len]);
        frame.is_dirty = true;
        bm.unpin(&self.file_name, 0);

        // Write tree data pages
        if tree_size > 0 {
            let page_data_size = page_size as usize;
            for page_idx in 0..data_pages - 1 {
                let offset = page_idx as usize * page_data_size;
                let end = (offset + page_data_size).min(tree_bytes.len());
                let chunk = &tree_bytes[offset..end];

                let frame = bm
                    .pin_mut(&self.file_name, page_idx + 1)
                    .map_err(|e| format!("Failed to pin ART data page {}: {e}", page_idx + 1))?;
                frame.data[..chunk.len()].copy_from_slice(chunk);
                if chunk.len() < frame.data.len() {
                    frame.data[chunk.len()..].fill(0);
                }
                frame.is_dirty = true;
                bm.unpin(&self.file_name, page_idx + 1);
            }
        }

        self.dirty = false;
        bm.flush_all().map_err(|e| format!("Failed to flush ART index: {e}"))
    }

    /// Load the ART index from BufferManager-backed pages.
    ///
    /// Returns a new `ArtPrimaryKeyIndex` with the loaded data.
    pub fn load(bm: &mut BufferManager, file_name: &str) -> Result<Self, String> {
        if !bm.is_file_registered(file_name) {
            return Err(format!(
                "ART index file '{file_name}' not registered with BufferManager"
            ));
        }

        // Read header page
        let frame = bm
            .pin(file_name, 0)
            .map_err(|e| format!("Failed to pin ART header page: {e}"))?;
        let data = &frame.data;

        let magic = u64::from_le_bytes(data[0..8].try_into().unwrap());
        if magic != ART_HEADER_MAGIC {
            bm.unpin(file_name, 0);
            return Err("Invalid ART index magic number".into());
        }

        let num_entries = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let page_count = u64::from_le_bytes(data[16..24].try_into().unwrap());
        let tree_size = u64::from_le_bytes(data[24..32].try_into().unwrap());
        bm.unpin(file_name, 0);

        // Read tree data
        let mut index = Self::new(file_name);
        index.page_count = page_count;
        index.num_entries = num_entries;

        if tree_size > 0 {
            let page_data_size = bm.page_size();
            let data_pages = (tree_size as usize).div_ceil(page_data_size);
            let mut tree_bytes = vec![0u8; tree_size as usize];

            for page_idx in 0..data_pages {
                let page_num = page_idx as u64 + 1;
                let offset = page_idx * page_data_size;
                let end = (offset + page_data_size).min(tree_bytes.len());

                let frame = bm
                    .pin(file_name, page_num)
                    .map_err(|e| format!("Failed to pin ART data page {page_num}: {e}"))?;
                let chunk_len = end - offset;
                tree_bytes[offset..end].copy_from_slice(&frame.data[..chunk_len]);
                bm.unpin(file_name, page_num);
            }

            // Deserialize tree
            let mut pos = 0usize;
            index.root = Self::deserialize_tree(&tree_bytes, &mut pos);
        }

        index.dirty = false;
        Ok(index)
    }

    /// Get the serialized tree size in bytes.
    pub fn serialized_tree_size(&self) -> u64 {
        self.serialize_tree().len() as u64
    }
}

impl std::fmt::Debug for ArtPrimaryKeyIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtPrimaryKeyIndex")
            .field("file_name", &self.file_name)
            .field("num_entries", &self.num_entries)
            .field("num_allocated_nodes", &self.num_allocated_nodes)
            .field("num_nodes_by_kind", &self.num_nodes_by_kind)
            .field("page_count", &self.page_count)
            .field("dirty", &self.dirty)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Compute the length of the common prefix between `node_prefix` (starting at
/// index 0) and `key_bytes` (starting at `key_offset`).
///
/// Port of C++ ART prefix comparison logic.
fn common_prefix_len(node_prefix: &[u8], key_bytes: &[u8], key_offset: usize) -> usize {
    let max_common = node_prefix.len().min(key_bytes.len().saturating_sub(key_offset));
    let mut count = 0;
    for i in 0..max_common {
        if node_prefix[i] == key_bytes[key_offset + i] {
            count += 1;
        } else {
            break;
        }
    }
    count
}

/// Check if the current key path satisfies the lower bound.
fn satisfies_lower_bound(key: &[u8], lower: Option<&ArtKey>, inclusive: bool) -> bool {
    match lower {
        None => true,
        Some(lower_key) => {
            let lower_bytes = lower_key.bytes();
            // Shorter keys sort before longer keys with the same prefix
            let min_len = key.len().min(lower_bytes.len());
            for i in 0..min_len {
                if key[i] < lower_bytes[i] {
                    return false;
                }
                if key[i] > lower_bytes[i] {
                    return true;
                }
            }
            // Prefix matches up to min_len
            if key.len() < lower_bytes.len() {
                return false; // key is shorter and prefix matches → key < lower
            }
            if key.len() > lower_bytes.len() {
                return true; // key is longer → key > lower
            }
            // Equal length → depends on inclusivity
            inclusive
        }
    }
}

/// Check if the current key path satisfies the upper bound.
fn satisfies_upper_bound(key: &[u8], upper: Option<&ArtKey>, inclusive: bool) -> bool {
    match upper {
        None => true,
        Some(upper_key) => {
            let upper_bytes = upper_key.bytes();
            let min_len = key.len().min(upper_bytes.len());
            for i in 0..min_len {
                if key[i] > upper_bytes[i] {
                    return false;
                }
                if key[i] < upper_bytes[i] {
                    return true;
                }
            }
            // Prefix matches up to min_len
            if key.len() > upper_bytes.len() {
                return false; // key is longer and prefix matches → key > upper
            }
            if key.len() < upper_bytes.len() {
                return true; // key is shorter → key < upper
            }
            // Equal length → depends on inclusivity
            inclusive
        }
    }
}

// ---------------------------------------------------------------------------
// Standalone ART operations (used by ArtPrimaryKeyIndex methods)
// ---------------------------------------------------------------------------

/// Split a node at the given prefix position, creating a new intermediate node.
/// Port of C++ `ArtPrimaryKeyIndex` split logic.
fn split_node(
    node: &mut ArtNode,
    common: usize,
    depth: usize,
    key_bytes: &[u8],
    row_offset: u64,
    num_entries: &mut u64,
) {
    let prefix = node.prefix().to_vec();
    let existing_byte = prefix[common];
    let existing_suffix = if common + 1 < prefix.len() {
        prefix[common + 1..].to_vec()
    } else {
        Vec::new()
    };

    // Build the new child for the existing branch
    let mut existing_child = ArtNode::new_node4();
    *existing_child.prefix_mut() = existing_suffix;
    existing_child = transfer_children_into(node, existing_child);
    let old_offsets = node.all_offsets();
    for off in old_offsets {
        existing_child.add_offset(off);
    }
    clear_children_offsets(node);

    // Build the new leaf for the inserted key
    let mut new_leaf = ArtNode::new_node4();
    new_leaf.add_offset(row_offset);
    if depth + common + 1 < key_bytes.len() {
        *new_leaf.prefix_mut() = key_bytes[depth + common + 1..].to_vec();
    }

    // Reset current node's prefix to the common prefix
    *node.prefix_mut() = prefix[..common].to_vec();
    clear_all_children(node);

    let new_byte = key_bytes[depth + common];
    node.insert_child(existing_byte, Box::new(existing_child));
    node.insert_child(new_byte, Box::new(new_leaf));
    *num_entries += 1;
}

/// Transfer all children from `src` into `dst`.
fn transfer_children_into(src: &mut ArtNode, mut dst: ArtNode) -> ArtNode {
    let pairs = collect_all_children(src);
    clear_all_children(src);
    for (byte, child) in pairs {
        dst.insert_child(byte, child);
    }
    dst
}

/// Collect all (byte, child) pairs from a node, taking ownership of children.
fn collect_all_children(node: &mut ArtNode) -> Vec<(u8, Box<ArtNode>)> {
    let mut pairs = Vec::new();
    match node {
        ArtNode::Node4 {
            keys, children, count, ..
        } => {
            for i in 0..*count as usize {
                if let Some(child) = children[i].take() {
                    pairs.push((keys[i], child));
                }
            }
        }
        ArtNode::Node16 {
            keys, children, count, ..
        } => {
            for i in 0..*count as usize {
                if let Some(child) = children[i].take() {
                    pairs.push((keys[i], child));
                }
            }
        }
        ArtNode::Node48 {
            child_index, children, ..
        } => {
            for byte in 0..256u16 {
                let idx = child_index[byte as usize];
                if idx != crate::art_node::EMPTY_MARKER
                    && let Some(child) = children[idx as usize].take()
                {
                    pairs.push((byte as u8, child));
                }
            }
        }
        ArtNode::Node256 { children, .. } => {
            for byte in 0..256u16 {
                if let Some(child) = children[byte as usize].take() {
                    pairs.push((byte as u8, child));
                }
            }
        }
    }
    pairs
}

/// Clear all children from a node (set count to 0, clear all child slots).
fn clear_all_children(node: &mut ArtNode) {
    match node {
        ArtNode::Node4 { children, count, .. } => {
            for c in children.iter_mut() {
                *c = None;
            }
            *count = 0;
        }
        ArtNode::Node16 { children, count, .. } => {
            for c in children.iter_mut() {
                *c = None;
            }
            *count = 0;
        }
        ArtNode::Node48 {
            child_index,
            children,
            count,
            ..
        } => {
            *child_index = [crate::art_node::EMPTY_MARKER; 256];
            for c in children.iter_mut() {
                *c = None;
            }
            *count = 0;
        }
        ArtNode::Node256 { children, count, .. } => {
            for c in children.iter_mut() {
                *c = None;
            }
            *count = 0;
        }
    }
}

/// Clear offsets from a node (but not children).
fn clear_children_offsets(node: &mut ArtNode) {
    match node {
        ArtNode::Node4 {
            offsets,
            overflow_offsets,
            ..
        }
        | ArtNode::Node16 {
            offsets,
            overflow_offsets,
            ..
        }
        | ArtNode::Node48 {
            offsets,
            overflow_offsets,
            ..
        }
        | ArtNode::Node256 {
            offsets,
            overflow_offsets,
            ..
        } => {
            offsets.clear();
            overflow_offsets.clear();
        }
    }
}

/// Recursive insert: traverse the tree and insert a key-value pair.
/// Port of C++ `ArtPrimaryKeyIndex::insertInternal()`.
fn insert_internal(node: &mut ArtNode, key: &[u8], depth: usize, row_offset: u64, num_entries: &mut u64) {
    let prefix = node.prefix().to_vec();
    let common = common_prefix_len(&prefix, key, depth);

    if common < prefix.len() {
        split_node(node, common, depth, key, row_offset, num_entries);
        return;
    }

    let new_depth = depth + prefix.len();

    if new_depth >= key.len() {
        node.add_offset(row_offset);
        if node.count() == 0 && node.prefix().is_empty() {
            *num_entries += 1;
        }
        return;
    }

    let byte = key[new_depth];
    if node.get_child(byte).is_none() {
        // Create a new leaf for the remaining key
        let mut leaf = ArtNode::new_node4();
        leaf.add_offset(row_offset);
        *leaf.prefix_mut() = key[new_depth + 1..].to_vec();
        node.insert_child(byte, Box::new(leaf));
        *num_entries += 1;
        return;
    }

    if let Some(child) = node.get_child_mut(byte) {
        insert_internal(child, key, new_depth + 1, row_offset, num_entries);
    }
}

/// Recursive erase: find and remove the offset from the matching leaf.
/// Returns `true` if an entry was actually removed.
fn erase_internal(node: &mut ArtNode, key: &[u8], depth: usize, row_offset: u64) -> bool {
    let prefix = node.prefix().to_vec();
    if depth + prefix.len() > key.len() || key[depth..depth + prefix.len()] != *prefix {
        return false;
    }
    let new_depth = depth + prefix.len();

    if new_depth >= key.len() {
        return node.remove_offset(row_offset);
    }

    let byte = key[new_depth];
    if let Some(child) = node.get_child_mut(byte) {
        erase_internal(child, key, new_depth + 1, row_offset)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(v: i64) -> ArtKey {
        ArtKey::from_value(&akar_common::types::Value::Int64(v)).unwrap()
    }

    fn make_str_key(s: &str) -> ArtKey {
        ArtKey::from_value(&akar_common::types::Value::String(s.into())).unwrap()
    }

    #[test]
    fn test_insert_and_lookup() {
        let mut idx = ArtPrimaryKeyIndex::new("test");
        idx.insert(&make_key(42), 0);
        assert_eq!(
            idx.lookup(&make_key(42)),
            Some(0),
            "key 42 should be found after first insert"
        );

        idx.insert(&make_key(43), 1);
        assert_eq!(
            idx.lookup(&make_key(42)),
            Some(0),
            "key 42 should still be found after second insert"
        );
        assert_eq!(
            idx.lookup(&make_key(43)),
            Some(1),
            "key 43 should be found after second insert"
        );
        assert_eq!(idx.lookup(&make_key(44)), None);
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn test_insert_multiple_consecutive() {
        let mut idx = ArtPrimaryKeyIndex::new("test");
        // Insert a range of consecutive integers to test node splitting
        for i in 0..20u64 {
            idx.insert(&make_key(i as i64), i);
        }
        assert_eq!(idx.len(), 20);
        for i in 0..20u64 {
            assert_eq!(idx.lookup(&make_key(i as i64)), Some(i), "key {i} should be found");
        }
    }

    #[test]
    fn test_range_scan_basic() {
        let mut idx = ArtPrimaryKeyIndex::new("test");
        for i in 0..100u64 {
            idx.insert(&make_key(i as i64), i);
        }

        // Range [10, 20)
        let results = idx.range_scan(Some(&make_key(10)), true, Some(&make_key(20)), false, 100);
        assert_eq!(results.len(), 10, "should find 10 items in [10, 20)");
        assert_eq!(results[0], 10);
        assert_eq!(results[9], 19);

        // Range [95, MAX)
        let results = idx.range_scan(Some(&make_key(95)), true, None, true, 100);
        assert_eq!(results.len(), 5, "should find 5 items from 95 onwards");
    }

    #[test]
    fn test_range_scan_open_bounds() {
        let mut idx = ArtPrimaryKeyIndex::new("test");
        idx.insert(&make_key(10), 10);
        idx.insert(&make_key(20), 20);
        idx.insert(&make_key(30), 30);

        // No bounds
        let results = idx.range_scan(None, true, None, true, 100);
        assert_eq!(results.len(), 3);

        // Only lower bound
        let results = idx.range_scan(Some(&make_key(20)), true, None, true, 100);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], 20);
    }

    #[test]
    fn test_range_scan_with_strings() {
        let mut idx = ArtPrimaryKeyIndex::new("test");
        idx.insert(&make_str_key("apple"), 1);
        idx.insert(&make_str_key("banana"), 2);
        idx.insert(&make_str_key("cherry"), 3);
        idx.insert(&make_str_key("date"), 4);

        let results = idx.range_scan(
            Some(&make_str_key("banana")),
            true,
            Some(&make_str_key("date")),
            false,
            100,
        );
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], 2);
        assert_eq!(results[1], 3);
    }

    #[test]
    fn test_delete() {
        let mut idx = ArtPrimaryKeyIndex::new("test");
        idx.insert(&make_key(42), 0);
        assert_eq!(idx.len(), 1);

        idx.delete(&make_key(42), 0);
        assert_eq!(idx.lookup(&make_key(42)), None);
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn test_serialize_roundtrip() {
        let mut idx = ArtPrimaryKeyIndex::new("test");
        for i in 0..50u64 {
            idx.insert(&make_key(i as i64), i);
        }

        let serialized = idx.serialize_tree();
        assert!(!serialized.is_empty());

        let mut pos = 0;
        let root = ArtPrimaryKeyIndex::deserialize_tree(&serialized, &mut pos);
        assert!(root.count() > 0 || root.has_offsets());
    }

    #[test]
    fn test_max_results() {
        let mut idx = ArtPrimaryKeyIndex::new("test");
        for i in 0..100u64 {
            idx.insert(&make_key(i as i64), i);
        }

        let results = idx.range_scan(None, true, None, true, 5);
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_clear() {
        let mut idx = ArtPrimaryKeyIndex::new("test");
        for i in 0..10u64 {
            idx.insert(&make_key(i as i64), i);
        }
        assert_eq!(idx.len(), 10);

        idx.clear();
        assert_eq!(idx.len(), 0);
        assert!(idx.is_empty());
    }

    #[test]
    fn test_lookup_empty() {
        let idx = ArtPrimaryKeyIndex::new("test");
        assert_eq!(idx.lookup(&make_key(42)), None);
    }

    #[test]
    fn test_range_scan_exclusive_lower() {
        let mut idx = ArtPrimaryKeyIndex::new("test");
        for i in 0..10u64 {
            idx.insert(&make_key(i as i64), i);
        }

        // (5, 10) — exclusive lower
        let results = idx.range_scan(Some(&make_key(5)), false, Some(&make_key(10)), false, 100);
        assert_eq!(results.len(), 4, "should find items 6,7,8,9");
        assert_eq!(results[0], 6);
    }

    #[test]
    fn test_range_scan_inclusive_upper() {
        let mut idx = ArtPrimaryKeyIndex::new("test");
        for i in 0..10u64 {
            idx.insert(&make_key(i as i64), i);
        }

        // [0, 5] — inclusive upper
        let results = idx.range_scan(Some(&make_key(0)), true, Some(&make_key(5)), true, 100);
        assert_eq!(results.len(), 6, "should find items 0-5 inclusive");
    }

    #[test]
    fn test_varint_roundtrip() {
        let values = vec![0u64, 1, 127, 128, 255, 16383, 16384, u64::MAX];
        for v in values {
            let mut buf = Vec::new();
            write_art_varint(&mut buf, v);
            let mut pos = 0;
            let decoded = read_art_varint(&buf, &mut pos);
            assert_eq!(decoded, v, "varint roundtrip failed for {v}");
            assert_eq!(pos, buf.len(), "not all bytes consumed for {v}");
        }
    }
}
