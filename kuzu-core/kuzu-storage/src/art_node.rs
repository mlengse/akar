//! ART (Adaptive Radix Tree) node types with arena allocation.
//!
//! Four node sizes matching the C++ ART implementation:
//! - `Node4`: Up to 4 children — smallest, best for sparse branches
//! - `Node16`: Up to 16 children — transitions from Node4 at 5 children
//! - `Node48`: Up to 48 children — uses indirect index for compact storage
//! - `Node256`: Up to 256 children — full fan-out, direct array
//!
//! Each node carries:
//! - A `prefix` (compressed path from parent)
//! - A set of `offsets` (row IDs stored at this leaf)
//! - `overflow_offsets` (additional row IDs for duplicate keys)
//!
//! Port of C++ `ArtPrimaryKeyIndex::Node` from `art_index.h` (lines 95–145)
//! and `art_index.cpp`.

use std::fmt;

/// Maximum number of nodes per arena block.
pub const NODE_BLOCK_CAPACITY: usize = 16 * 1024;

/// ART node kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Node4 = 0,
    Node16 = 1,
    Node48 = 2,
    Node256 = 3,
}

/// The growth threshold from one node type to the next.
pub const NODE4_MAX: u16 = 4;
pub const NODE16_MAX: u16 = 16;
pub const NODE48_MAX: u16 = 48;

/// Marker for unused child slots in Node48's indirect index.
pub const EMPTY_MARKER: u8 = u8::MAX;

/// An ART node — stores a prefix, child pointers, and value offsets.
///
/// Uses a flat enum to model the four growth stages. Each variant stores
/// a fixed-size array of children (or indirect index + compact array).
#[derive(Clone)]
pub enum ArtNode {
    Node4 {
        prefix: Vec<u8>,
        keys: [u8; 4],
        children: [Option<Box<ArtNode>>; 4],
        offsets: Vec<u64>,
        overflow_offsets: Vec<u64>,
        count: u16,
    },
    Node16 {
        prefix: Vec<u8>,
        keys: [u8; 16],
        children: [Option<Box<ArtNode>>; 16],
        offsets: Vec<u64>,
        overflow_offsets: Vec<u64>,
        count: u16,
    },
    Node48 {
        prefix: Vec<u8>,
        child_index: [u8; 256],
        children: [Option<Box<ArtNode>>; 48],
        offsets: Vec<u64>,
        overflow_offsets: Vec<u64>,
        count: u16,
    },
    Node256 {
        prefix: Vec<u8>,
        children: [Option<Box<ArtNode>>; 256],
        offsets: Vec<u64>,
        overflow_offsets: Vec<u64>,
        count: u16,
    },
}

impl ArtNode {
    /// Create a new empty Node4.
    pub fn new_node4() -> Self {
        ArtNode::Node4 {
            prefix: Vec::new(),
            keys: [0u8; 4],
            children: Default::default(),
            offsets: Vec::new(),
            overflow_offsets: Vec::new(),
            count: 0,
        }
    }

    /// Get the node kind.
    pub fn kind(&self) -> NodeKind {
        match self {
            ArtNode::Node4 { .. } => NodeKind::Node4,
            ArtNode::Node16 { .. } => NodeKind::Node16,
            ArtNode::Node48 { .. } => NodeKind::Node48,
            ArtNode::Node256 { .. } => NodeKind::Node256,
        }
    }

    /// Get the number of children.
    pub fn count(&self) -> u16 {
        match self {
            ArtNode::Node4 { count, .. }
            | ArtNode::Node16 { count, .. }
            | ArtNode::Node48 { count, .. }
            | ArtNode::Node256 { count, .. } => *count,
        }
    }

    /// Get the prefix bytes.
    pub fn prefix(&self) -> &[u8] {
        match self {
            ArtNode::Node4 { prefix, .. }
            | ArtNode::Node16 { prefix, .. }
            | ArtNode::Node48 { prefix, .. }
            | ArtNode::Node256 { prefix, .. } => prefix,
        }
    }

    /// Get the prefix bytes (mutable).
    pub fn prefix_mut(&mut self) -> &mut Vec<u8> {
        match self {
            ArtNode::Node4 { prefix, .. }
            | ArtNode::Node16 { prefix, .. }
            | ArtNode::Node48 { prefix, .. }
            | ArtNode::Node256 { prefix, .. } => prefix,
        }
    }

    /// Returns `true` if this node has any offsets (i.e., it's a leaf or contains leaves).
    pub fn has_offsets(&self) -> bool {
        let (offsets, overflow) = match self {
            ArtNode::Node4 { offsets, overflow_offsets, .. } => (offsets, overflow_offsets),
            ArtNode::Node16 { offsets, overflow_offsets, .. } => (offsets, overflow_offsets),
            ArtNode::Node48 { offsets, overflow_offsets, .. } => (offsets, overflow_offsets),
            ArtNode::Node256 { offsets, overflow_offsets, .. } => (offsets, overflow_offsets),
        };
        !offsets.is_empty() || !overflow.is_empty()
    }

    /// Get all offsets (primary + overflow) as a slice.
    pub fn all_offsets(&self) -> Vec<u64> {
        let (offsets, overflow) = match self {
            ArtNode::Node4 { offsets, overflow_offsets, .. } => (offsets, overflow_offsets),
            ArtNode::Node16 { offsets, overflow_offsets, .. } => (offsets, overflow_offsets),
            ArtNode::Node48 { offsets, overflow_offsets, .. } => (offsets, overflow_offsets),
            ArtNode::Node256 { offsets, overflow_offsets, .. } => (offsets, overflow_offsets),
        };
        let mut result = offsets.clone();
        result.extend_from_slice(overflow);
        result
    }

    /// Insert a child at the given byte.
    /// Grows the node if necessary.
    pub fn insert_child(&mut self, byte: u8, child: Box<ArtNode>) {
        match self {
            ArtNode::Node4 { prefix, keys, children, offsets, overflow_offsets, count } => {
                if *count < NODE4_MAX {
                    keys[*count as usize] = byte;
                    children[*count as usize] = Some(child);
                    *count += 1;
                } else {
                    let old_prefix = std::mem::take(prefix);
                    let old_offsets = std::mem::take(offsets);
                    let old_overflow = std::mem::take(overflow_offsets);
                    let old_keys = std::mem::take(keys);
                    let old_children = std::mem::take(children);
                    let old_count = *count;
                    *self = ArtNode::grow_node4_to_node16(
                        old_prefix, old_offsets, old_overflow,
                        old_keys, old_children, old_count,
                    );
                    self.insert_child(byte, child);
                }
            }
            ArtNode::Node16 { prefix, keys, children, offsets, overflow_offsets, count } => {
                if *count < NODE16_MAX {
                    keys[*count as usize] = byte;
                    children[*count as usize] = Some(child);
                    *count += 1;
                } else {
                    let old_prefix = std::mem::take(prefix);
                    let old_offsets = std::mem::take(offsets);
                    let old_overflow = std::mem::take(overflow_offsets);
                    let old_keys = std::mem::take(keys);
                    let old_children = std::mem::take(children);
                    let old_count = *count;
                    *self = ArtNode::grow_node16_to_node48(
                        old_prefix, old_offsets, old_overflow,
                        old_keys, old_children, old_count,
                    );
                    self.insert_child(byte, child);
                }
            }
            ArtNode::Node48 { prefix, child_index, children, offsets, overflow_offsets, count } => {
                if *count < NODE48_MAX {
                    child_index[byte as usize] = *count as u8;
                    children[*count as usize] = Some(child);
                    *count += 1;
                } else {
                    let old_prefix = std::mem::take(prefix);
                    let old_offsets = std::mem::take(offsets);
                    let old_overflow = std::mem::take(overflow_offsets);
                    let old_child_index = std::mem::replace(child_index, [EMPTY_MARKER; 256]);
                    let old_children = std::mem::replace(children, std::array::from_fn(|_| None));
                    let old_count = *count;
                    *self = ArtNode::grow_node48_to_node256(
                        old_prefix, old_offsets, old_overflow,
                        old_child_index, old_children, old_count,
                    );
                    self.insert_child(byte, child);
                }
            }
            ArtNode::Node256 { children, count, .. } => {
                if children[byte as usize].is_none() {
                    children[byte as usize] = Some(child);
                    *count += 1;
                }
            }
        }
    }

    /// Get a child by byte, if it exists.
    pub fn get_child(&self, byte: u8) -> Option<&Box<ArtNode>> {
        match self {
            ArtNode::Node4 { keys, children, count, .. } => {
                for i in 0..*count as usize {
                    if keys[i] == byte {
                        return children[i].as_ref();
                    }
                }
                None
            }
            ArtNode::Node16 { keys, children, count, .. } => {
                for i in 0..*count as usize {
                    if keys[i] == byte {
                        return children[i].as_ref();
                    }
                }
                None
            }
            ArtNode::Node48 { child_index, children, .. } => {
                let idx = child_index[byte as usize];
                if idx == EMPTY_MARKER {
                    None
                } else {
                    children[idx as usize].as_ref()
                }
            }
            ArtNode::Node256 { children, .. } => children[byte as usize].as_ref(),
        }
    }

    /// Get a mutable child by byte, if it exists.
    pub fn get_child_mut(&mut self, byte: u8) -> Option<&mut Box<ArtNode>> {
        match self {
            ArtNode::Node4 { keys, children, count, .. } => {
                for i in 0..*count as usize {
                    if keys[i] == byte {
                        return children[i].as_mut();
                    }
                }
                None
            }
            ArtNode::Node16 { keys, children, count, .. } => {
                for i in 0..*count as usize {
                    if keys[i] == byte {
                        return children[i].as_mut();
                    }
                }
                None
            }
            ArtNode::Node48 { child_index, children, .. } => {
                let idx = child_index[byte as usize];
                if idx == EMPTY_MARKER {
                    None
                } else {
                    children[idx as usize].as_mut()
                }
            }
            ArtNode::Node256 { children, .. } => children[byte as usize].as_mut(),
        }
    }

    /// Get or insert a child node at the given byte.
    /// Returns a mutable reference to the child, creating a new empty Node4 if needed.
    pub fn get_or_insert_child(&mut self, byte: u8) -> &mut Box<ArtNode> {
        if self.get_child(byte).is_some() {
            return self.get_child_mut(byte).unwrap();
        }
        let new_child = Box::new(ArtNode::new_node4());
        self.insert_child(byte, new_child);
        self.get_child_mut(byte).unwrap()
    }

    /// Remove a child by byte. Does not shrink the node.
    pub fn remove_child(&mut self, byte: u8) {
        match self {
            ArtNode::Node4 { keys, children, count, .. } => {
                for i in 0..*count as usize {
                    if keys[i] == byte {
                        // Shift remaining keys/children left
                        for j in i..(*count as usize - 1) {
                            keys[j] = keys[j + 1];
                            children[j] = children[j + 1].take();
                        }
                        children[*count as usize - 1] = None;
                        *count -= 1;
                        return;
                    }
                }
            }
            ArtNode::Node16 { keys, children, count, .. } => {
                for i in 0..*count as usize {
                    if keys[i] == byte {
                        for j in i..(*count as usize - 1) {
                            keys[j] = keys[j + 1];
                            children[j] = children[j + 1].take();
                        }
                        children[*count as usize - 1] = None;
                        *count -= 1;
                        // Could shrink to Node4 if count <= 4
                        return;
                    }
                }
            }
            ArtNode::Node48 { child_index, children, count, .. } => {
                let idx = child_index[byte as usize];
                if idx != EMPTY_MARKER {
                    children[idx as usize] = None;
                    child_index[byte as usize] = EMPTY_MARKER;
                    *count -= 1;
                }
            }
            ArtNode::Node256 { children, count, .. } => {
                if children[byte as usize].take().is_some() {
                    *count -= 1;
                }
            }
        }
    }

    /// Add a value offset to this node.
    pub fn add_offset(&mut self, offset: u64) {
        let (offsets, overflow) = match self {
            ArtNode::Node4 { offsets, overflow_offsets, .. }
            | ArtNode::Node16 { offsets, overflow_offsets, .. }
            | ArtNode::Node48 { offsets, overflow_offsets, .. }
            | ArtNode::Node256 { offsets, overflow_offsets, .. } => (offsets, overflow_offsets),
        };
        if offsets.is_empty() {
            offsets.push(offset);
        } else {
            overflow.push(offset);
        }
    }

    /// Remove a specific offset from this node's offsets or overflow.
    /// Returns `true` if the offset was found and removed.
    pub fn remove_offset(&mut self, offset: u64) -> bool {
        let (offsets, overflow) = match self {
            ArtNode::Node4 { offsets, overflow_offsets, .. }
            | ArtNode::Node16 { offsets, overflow_offsets, .. }
            | ArtNode::Node48 { offsets, overflow_offsets, .. }
            | ArtNode::Node256 { offsets, overflow_offsets, .. } => (offsets, overflow_offsets),
        };
        if let Some(pos) = offsets.iter().position(|&o| o == offset) {
            offsets.remove(pos);
            // If overflow exists, promote one
            if !overflow.is_empty() {
                offsets.push(overflow.remove(0));
            }
            return true;
        }
        if let Some(pos) = overflow.iter().position(|&o| o == offset) {
            overflow.remove(pos);
            return true;
        }
        false
    }

    /// Clear all offsets from this node.
    pub fn clear_offsets(&mut self) {
        match self {
            ArtNode::Node4 { offsets, overflow_offsets, .. }
            | ArtNode::Node16 { offsets, overflow_offsets, .. }
            | ArtNode::Node48 { offsets, overflow_offsets, .. }
            | ArtNode::Node256 { offsets, overflow_offsets, .. } => {
                offsets.clear();
                overflow_offsets.clear();
            }
        }
    }

    /// Returns `true` if this node is empty (no children, no offsets, no prefix).
    pub fn is_empty(&self) -> bool {
        !self.has_offsets() && self.count() == 0 && self.prefix().is_empty()
    }

    // ---- Growth helpers ----

    fn grow_node4_to_node16(
        old_prefix: Vec<u8>,
        old_offsets: Vec<u64>,
        old_overflow: Vec<u64>,
        old_keys: [u8; 4],
        mut old_children: [Option<Box<ArtNode>>; 4],
        old_count: u16,
    ) -> Self {
        let mut keys = [0u8; 16];
        let mut children: [Option<Box<ArtNode>>; 16] = Default::default();
        for i in 0..old_count as usize {
            keys[i] = old_keys[i];
            children[i] = old_children[i].take();
        }
        ArtNode::Node16 {
            prefix: old_prefix,
            keys,
            children,
            offsets: old_offsets,
            overflow_offsets: old_overflow,
            count: old_count,
        }
    }

    fn grow_node16_to_node48(
        old_prefix: Vec<u8>,
        old_offsets: Vec<u64>,
        old_overflow: Vec<u64>,
        old_keys: [u8; 16],
        mut old_children: [Option<Box<ArtNode>>; 16],
        old_count: u16,
    ) -> Self {
        let mut child_index = [EMPTY_MARKER; 256];
        let mut children: [Option<Box<ArtNode>>; 48] = std::array::from_fn(|_| None);
        for i in 0..old_count as usize {
            let byte = old_keys[i];
            child_index[byte as usize] = i as u8;
            children[i] = old_children[i].take();
        }
        ArtNode::Node48 {
            prefix: old_prefix,
            child_index,
            children,
            offsets: old_offsets,
            overflow_offsets: old_overflow,
            count: old_count,
        }
    }

    fn grow_node48_to_node256(
        old_prefix: Vec<u8>,
        old_offsets: Vec<u64>,
        old_overflow: Vec<u64>,
        old_child_index: [u8; 256],
        mut old_children: [Option<Box<ArtNode>>; 48],
        old_count: u16,
    ) -> Self {
        let mut children: [Option<Box<ArtNode>>; 256] = std::array::from_fn(|_| None);
        for byte in 0..256u16 {
            let idx = old_child_index[byte as usize];
            if idx != EMPTY_MARKER {
                children[byte as usize] = old_children[idx as usize].take();
            }
        }
        ArtNode::Node256 {
            prefix: old_prefix,
            children,
            offsets: old_offsets,
            overflow_offsets: old_overflow,
            count: old_count,
        }
    }
}

impl fmt::Debug for ArtNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArtNode::Node4 { prefix, offsets, overflow_offsets, count, .. } => {
                f.debug_struct("Node4")
                    .field("prefix", &prefix)
                    .field("count", count)
                    .field("offsets", offsets)
                    .field("overflow", overflow_offsets)
                    .finish()
            }
            ArtNode::Node16 { prefix, offsets, overflow_offsets, count, .. } => {
                f.debug_struct("Node16")
                    .field("prefix", &prefix)
                    .field("count", count)
                    .field("offsets", offsets)
                    .field("overflow", overflow_offsets)
                    .finish()
            }
            ArtNode::Node48 { prefix, offsets, overflow_offsets, count, .. } => {
                f.debug_struct("Node48")
                    .field("prefix", &prefix)
                    .field("count", count)
                    .field("offsets", offsets)
                    .field("overflow", overflow_offsets)
                    .finish()
            }
            ArtNode::Node256 { prefix, offsets, overflow_offsets, count, .. } => {
                f.debug_struct("Node256")
                    .field("prefix", &prefix)
                    .field("count", count)
                    .field("offsets", offsets)
                    .field("overflow", overflow_offsets)
                    .finish()
            }
        }
    }
}

// ---- Arena block for fixed-size node storage ----

/// A block of nodes in contiguous memory, used for arena allocation.
///
/// Each block holds up to `NODE_BLOCK_CAPACITY` nodes allocated in
/// a `Vec<ArtNode>`. New nodes are appended; existing nodes are never
/// individually freed (the entire block is freed on clear).
///
/// Port of C++ `NodeBlock` from `art_index.h` (lines 148–163).
#[derive(Clone)]
pub struct NodeBlock {
    nodes: Vec<ArtNode>,
    used: usize,
}

impl NodeBlock {
    pub fn new() -> Self {
        Self {
            nodes: Vec::with_capacity(NODE_BLOCK_CAPACITY),
            used: 0,
        }
    }

    /// Allocate a new node in this block.
    /// Returns the index within the block.
    pub fn allocate(&mut self, node: ArtNode) -> usize {
        let idx = self.used;
        if idx < NODE_BLOCK_CAPACITY {
            self.nodes.push(node);
            self.used += 1;
            idx
        } else {
            panic!("NodeBlock full (capacity={NODE_BLOCK_CAPACITY})");
        }
    }

    /// Get a reference to a node by index.
    pub fn get(&self, idx: usize) -> Option<&ArtNode> {
        self.nodes.get(idx)
    }

    /// Get a mutable reference to a node by index.
    pub fn get_mut(&mut self, idx: usize) -> Option<&mut ArtNode> {
        self.nodes.get_mut(idx)
    }

    /// Number of used slots.
    pub fn len(&self) -> usize {
        self.used
    }

    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    /// Capacity of this block.
    pub fn capacity(&self) -> usize {
        NODE_BLOCK_CAPACITY
    }

    /// How many slots are remaining.
    pub fn remaining(&self) -> usize {
        NODE_BLOCK_CAPACITY - self.used
    }

    /// Clear all nodes (drops them).
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.used = 0;
    }
}

impl Default for NodeBlock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node4_insert_and_get() {
        let mut node = ArtNode::new_node4();
        let child = Box::new(ArtNode::new_node4());
        node.insert_child(0x42, child);

        assert_eq!(node.count(), 1);
        assert!(node.get_child(0x42).is_some());
        assert!(node.get_child(0x00).is_none());
    }

    #[test]
    fn test_node4_grows_to_node16() {
        let mut node = ArtNode::new_node4();
        for b in 0..5u8 {
            node.insert_child(b, Box::new(ArtNode::new_node4()));
        }
        assert_eq!(node.kind(), NodeKind::Node16);
        assert_eq!(node.count(), 5);
        for b in 0..5u8 {
            assert!(node.get_child(b).is_some(), "child {b} should exist");
        }
    }

    #[test]
    fn test_node16_grows_to_node48() {
        let mut node = ArtNode::new_node4();
        for b in 0..18u8 {
            node.insert_child(b, Box::new(ArtNode::new_node4()));
        }
        assert_eq!(node.kind(), NodeKind::Node48);
        assert_eq!(node.count(), 18);
    }

    #[test]
    fn test_node48_grows_to_node256() {
        let mut node = ArtNode::new_node4();
        for b in 0..50u8 {
            node.insert_child(b, Box::new(ArtNode::new_node4()));
        }
        assert_eq!(node.kind(), NodeKind::Node256);
        assert_eq!(node.count(), 50);
    }

    #[test]
    fn test_remove_child() {
        let mut node = ArtNode::new_node4();
        node.insert_child(0x10, Box::new(ArtNode::new_node4()));
        node.insert_child(0x20, Box::new(ArtNode::new_node4()));
        assert_eq!(node.count(), 2);

        node.remove_child(0x10);
        assert_eq!(node.count(), 1);
        assert!(node.get_child(0x10).is_none());
        assert!(node.get_child(0x20).is_some());
    }

    #[test]
    fn test_add_and_remove_offset() {
        let mut node = ArtNode::new_node4();
        assert!(!node.has_offsets());
        node.add_offset(42);
        assert!(node.has_offsets());
        assert_eq!(node.all_offsets(), vec![42]);

        node.add_offset(99);
        assert_eq!(node.all_offsets(), vec![42, 99]);

        assert!(node.remove_offset(42));
        assert!(!node.remove_offset(999));
        assert_eq!(node.all_offsets(), vec![99]);
    }

    #[test]
    fn test_node_block_allocate() {
        let mut block = NodeBlock::new();
        let idx = block.allocate(ArtNode::new_node4());
        assert_eq!(idx, 0);
        assert_eq!(block.len(), 1);
        assert!(block.get(0).is_some());
    }

    #[test]
    fn test_get_or_insert_child() {
        let mut node = ArtNode::new_node4();
        let _child = node.get_or_insert_child(0xAB);
        assert_eq!(node.count(), 1);

        // Getting again should not create a new child
        let _same = node.get_or_insert_child(0xAB);
        assert_eq!(node.count(), 1);
    }
}
