//! Roaring Bitmap — compressed bitset for node/edge ID sets.
//!
//! Uses the standard Roaring Bitmap layout:
//! - High 16 bits → container key (u16)
//! - Low 16 bits → index within the container
//! - **Array container**: sorted `Vec<u16>` for sparse chunks (≤ `ARRAY_MAX_SIZE`)
//! - **Bitmap container**: `[u64; 1024]` fixed bitset for dense chunks
//! - Auto-upgrades Array → Bitmap when size exceeds `ARRAY_MAX_SIZE`
//!
//! Supports `u32` values (covering 4B IDs, sufficient for most graphs).

use std::collections::BTreeMap;

/// Maximum number of elements in an Array container before upgrading to Bitmap.
const ARRAY_MAX_SIZE: usize = 4096;

/// Bits per bitmap container (1024 × 64).


// ---------------------------------------------------------------------------
// Container enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Container {
    Array(Vec<u16>),
    Bitmap(Box<[u64; 1024]>),
}

impl Container {
    fn new_array() -> Self {
        Container::Array(Vec::new())
    }

    fn len(&self) -> usize {
        match self {
            Container::Array(v) => v.len(),
            Container::Bitmap(bm) => bm.iter().map(|w| w.count_ones() as usize).sum(),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Container::Array(v) => v.is_empty(),
            Container::Bitmap(bm) => bm.iter().all(|w| *w == 0),
        }
    }

    fn contains(&self, idx: u16) -> bool {
        match self {
            Container::Array(v) => v.binary_search(&idx).is_ok(),
            Container::Bitmap(bm) => bm[idx as usize / 64] & (1u64 << (idx as u64 % 64)) != 0,
        }
    }

    fn add(&mut self, idx: u16) -> bool {
        match self {
            Container::Array(v) => {
                if let Err(pos) = v.binary_search(&idx) {
                    v.insert(pos, idx);
                    if v.len() > ARRAY_MAX_SIZE {
                        *self = Container::Bitmap(Self::from_array_to_bitmap(std::mem::take(v)));
                    }
                    true
                } else {
                    false
                }
            }
            Container::Bitmap(bm) => {
                let word = &mut bm[idx as usize / 64];
                let bit = 1u64 << (idx as u64 % 64);
                if *word & bit == 0 {
                    *word |= bit;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn remove(&mut self, idx: u16) -> bool {
        match self {
            Container::Array(v) => {
                if let Ok(pos) = v.binary_search(&idx) {
                    v.remove(pos);
                    true
                } else {
                    false
                }
            }
            Container::Bitmap(bm) => {
                let word = &mut bm[idx as usize / 64];
                let bit = 1u64 << (idx as u64 % 64);
                if *word & bit != 0 {
                    *word &= !bit;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn insert_all_from(&mut self, other: &Container) {
        match self {
            Container::Array(v) => match other {
                Container::Array(other_v) => {
                    let mut merged = Vec::with_capacity(v.len() + other_v.len());
                    let mut i = 0;
                    let mut j = 0;
                    while i < v.len() && j < other_v.len() {
                        if v[i] < other_v[j] {
                            merged.push(v[i]);
                            i += 1;
                        } else if v[i] > other_v[j] {
                            merged.push(other_v[j]);
                            j += 1;
                        } else {
                            merged.push(v[i]);
                            i += 1;
                            j += 1;
                        }
                    }
                    merged.extend_from_slice(&v[i..]);
                    merged.extend_from_slice(&other_v[j..]);

                    if merged.len() > ARRAY_MAX_SIZE {
                        *self = Container::Bitmap(Self::from_array_to_bitmap(merged));
                    } else {
                        *v = merged;
                    }
                }
                Container::Bitmap(other_bm) => {
                    let other_count = other_bm.iter().map(|w| w.count_ones() as usize).sum::<usize>();
                    if v.len() + other_count > ARRAY_MAX_SIZE {
                        let mut bm = Self::from_array_to_bitmap(std::mem::take(v));
                        for (i, w) in other_bm.iter().enumerate() {
                            bm[i] |= w;
                        }
                        *self = Container::Bitmap(bm);
                    } else {
                        for (word_idx, word) in other_bm.iter().enumerate() {
                            if *word == 0 { continue; }
                            let base = (word_idx * 64) as u16;
                            for bit in 0..64 {
                                if word & (1u64 << bit) != 0 {
                                    let idx = base + bit as u16;
                                    if let Err(pos) = v.binary_search(&idx) {
                                        v.insert(pos, idx);
                                    }
                                }
                            }
                        }
                        if v.len() > ARRAY_MAX_SIZE {
                            *self = Container::Bitmap(Self::from_array_to_bitmap(std::mem::take(v)));
                        }
                    }
                }
            },
            Container::Bitmap(bm) => match other {
                Container::Array(other_v) => {
                    for &idx in other_v {
                        bm[idx as usize / 64] |= 1u64 << (idx as u64 % 64);
                    }
                }
                Container::Bitmap(other_bm) => {
                    for (i, w) in other_bm.iter().enumerate() {
                        bm[i] |= w;
                    }
                }
            },
        }
    }

    fn retain_intersection(&mut self, other: &Container) {
        match (self, other) {
            (Container::Array(v), Container::Array(other_v)) => {
                v.retain(|x| other_v.binary_search(x).is_ok());
            }
            (Container::Array(v), Container::Bitmap(other_bm)) => {
                v.retain(|x| other_bm[*x as usize / 64] & (1u64 << (*x as u64 % 64)) != 0);
            }
            (Container::Bitmap(bm), Container::Array(other_v)) => {
                let mut bit_set = [0u64; 1024];
                for &idx in other_v {
                    bit_set[idx as usize / 64] |= 1u64 << (idx as u64 % 64);
                }
                for (i, w) in bm.iter_mut().enumerate() {
                    *w &= bit_set[i];
                }
            }
            (Container::Bitmap(bm), Container::Bitmap(other_bm)) => {
                for (i, w) in bm.iter_mut().enumerate() {
                    *w &= other_bm[i];
                }
            }
        }
    }

    fn subtract(&mut self, other: &Container) {
        match (self, other) {
            (Container::Array(v), Container::Array(other_v)) => {
                v.retain(|x| other_v.binary_search(x).is_err());
            }
            (Container::Array(v), Container::Bitmap(other_bm)) => {
                v.retain(|x| other_bm[*x as usize / 64] & (1u64 << (*x as u64 % 64)) == 0);
            }
            (Container::Bitmap(bm), Container::Array(other_v)) => {
                for &idx in other_v {
                    bm[idx as usize / 64] &= !(1u64 << (idx as u64 % 64));
                }
            }
            (Container::Bitmap(bm), Container::Bitmap(other_bm)) => {
                for (i, w) in bm.iter_mut().enumerate() {
                    *w &= !other_bm[i];
                }
            }
        }
    }

    fn iter(&self) -> ContainerIter<'_> {
        match self {
            Container::Array(v) => ContainerIter::Array { data: v, pos: 0 },
            Container::Bitmap(bm) => ContainerIter::Bitmap { data: bm, word_idx: 0, bit: 0 },
        }
    }

    fn from_array_to_bitmap(v: Vec<u16>) -> Box<[u64; 1024]> {
        let mut bm = Box::new([0u64; 1024]);
        for &idx in &v {
            bm[idx as usize / 64] |= 1u64 << (idx as u64 % 64);
        }
        bm
    }
}

// ---------------------------------------------------------------------------
// Container iterator
// ---------------------------------------------------------------------------

enum ContainerIter<'a> {
    Array { data: &'a Vec<u16>, pos: usize },
    Bitmap { data: &'a [u64; 1024], word_idx: usize, bit: u32 },
}

impl<'a> Iterator for ContainerIter<'a> {
    type Item = u16;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ContainerIter::Array { data, pos } => {
                let v = data.get(*pos)?.clone();
                *pos += 1;
                Some(v)
            }
            ContainerIter::Bitmap { data, word_idx, bit } => {
                while *word_idx < 1024 {
                    let word = data[*word_idx];
                    if word == 0 {
                        *word_idx += 1;
                        *bit = 0;
                        continue;
                    }
                    while *bit < 64 {
                        if word & (1u64 << *bit) != 0 {
                            let result = (*word_idx * 64 + *bit as usize) as u16;
                            *bit += 1;
                            return Some(result);
                        }
                        *bit += 1;
                    }
                    *word_idx += 1;
                    *bit = 0;
                }
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// RoaringBitmap
// ---------------------------------------------------------------------------

/// A compressed bitset for `u32` values using the Roaring Bitmap format.
///
/// # Example
///
/// ```
/// use kuzu_storage::roaring_bitmap::RoaringBitmap;
///
/// let mut rb = RoaringBitmap::new();
/// rb.add(42);
/// rb.add(100000);
/// assert!(rb.contains(42));
/// assert!(!rb.contains(0));
/// assert_eq!(rb.len(), 2);
/// ```
#[derive(Debug, Clone)]
pub struct RoaringBitmap {
    containers: BTreeMap<u16, Container>,
    len: usize,
}

impl Default for RoaringBitmap {
    fn default() -> Self {
        Self::new()
    }
}

impl RoaringBitmap {
    /// Create an empty bitmap.
    pub fn new() -> Self {
        Self {
            containers: BTreeMap::new(),
            len: 0,
        }
    }

    /// Create a bitmap from a sorted `Vec<u32>`.
    pub fn from_sorted(values: &[u32]) -> Self {
        let mut rb = Self::new();
        if values.is_empty() {
            return rb;
        }
        // Greedily build containers
        let mut i = 0;
        while i < values.len() {
            let key = (values[i] >> 16) as u16;
            let low = (values[i] & 0xFFFF) as u16;
            let container = rb.containers.entry(key).or_insert_with(Container::new_array);
            let _ = container.add(low);
            rb.len += 1;
            i += 1;
            // Batch contiguous values under the same key
            while i < values.len() && (values[i] >> 16) as u16 == key {
                let low = (values[i] & 0xFFFF) as u16;
                let _ = container.add(low);
                rb.len += 1;
                i += 1;
            }
        }
        rb
    }

    /// Insert a value. Returns `true` if the value was newly added.
    pub fn add(&mut self, value: u32) -> bool {
        let key = (value >> 16) as u16;
        let low = (value & 0xFFFF) as u16;
        let container = self.containers.entry(key).or_insert_with(Container::new_array);
        if container.add(low) {
            self.len += 1;
            true
        } else {
            false
        }
    }

    /// Remove a value. Returns `true` if the value was present.
    pub fn remove(&mut self, value: u32) -> bool {
        let key = (value >> 16) as u16;
        let low = (value & 0xFFFF) as u16;
        if let Some(container) = self.containers.get_mut(&key) {
            if container.remove(low) {
                self.len = self.len.saturating_sub(1);
                if container.is_empty() {
                    self.containers.remove(&key);
                }
                return true;
            }
        }
        false
    }

    /// Check if a value is present.
    pub fn contains(&self, value: u32) -> bool {
        let key = (value >> 16) as u16;
        let low = (value & 0xFFFF) as u16;
        self.containers.get(&key).is_some_and(|c| c.contains(low))
    }

    /// Number of elements in the bitmap.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the bitmap is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Union with another bitmap in-place.
    pub fn union_with(&mut self, other: &RoaringBitmap) {
        for (&key, other_c) in &other.containers {
            let container = self.containers.entry(key).or_insert_with(Container::new_array);
            let before = container.len();
            container.insert_all_from(other_c);
            self.len += container.len() - before;
        }
    }

    /// Intersection with another bitmap in-place.
    pub fn intersect_with(&mut self, other: &RoaringBitmap) {
        let mut keys_to_remove = Vec::new();
        for (&key, container) in &mut self.containers {
            if let Some(other_c) = other.containers.get(&key) {
                let before = container.len();
                container.retain_intersection(other_c);
                self.len -= before - container.len();
                if container.is_empty() {
                    keys_to_remove.push(key);
                }
            } else {
                self.len -= container.len();
                keys_to_remove.push(key);
            }
        }
        for key in keys_to_remove {
            self.containers.remove(&key);
        }
    }

    /// Difference with another bitmap in-place (self = self \\ other).
    pub fn difference_with(&mut self, other: &RoaringBitmap) {
        let mut keys_to_remove = Vec::new();
        for (&key, container) in &mut self.containers {
            if let Some(other_c) = other.containers.get(&key) {
                let before = container.len();
                container.subtract(other_c);
                self.len -= before - container.len();
                if container.is_empty() {
                    keys_to_remove.push(key);
                }
            }
        }
        for key in keys_to_remove {
            self.containers.remove(&key);
        }
    }

    /// Return a new bitmap as the union of `self` and `other`.
    pub fn union(&self, other: &RoaringBitmap) -> RoaringBitmap {
        let mut result = self.clone();
        result.union_with(other);
        result
    }

    /// Return a new bitmap as the intersection of `self` and `other`.
    pub fn intersection(&self, other: &RoaringBitmap) -> RoaringBitmap {
        let mut result = self.clone();
        result.intersect_with(other);
        result
    }

    /// Return a new bitmap as the difference of `self` and `other`.
    pub fn difference(&self, other: &RoaringBitmap) -> RoaringBitmap {
        let mut result = self.clone();
        result.difference_with(other);
        result
    }

    /// Iterator over all values in sorted order.
    pub fn iter(&self) -> RoaringIter<'_> {
        RoaringIter {
            containers: self.containers.iter(),
            current: None,
        }
    }

    /// Collect all values into a sorted `Vec<u32>`.
    pub fn to_vec(&self) -> Vec<u32> {
        self.iter().collect()
    }
}

// ---------------------------------------------------------------------------
// Top-level iterator
// ---------------------------------------------------------------------------

pub struct RoaringIter<'a> {
    containers: std::collections::btree_map::Iter<'a, u16, Container>,
    current: Option<(u16, ContainerIter<'a>)>,
}

impl<'a> Iterator for RoaringIter<'a> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((key, ref mut inner)) = self.current {
                if let Some(low) = inner.next() {
                    return Some(((key as u32) << 16) | low as u32);
                }
            }
            // Advance to next container
            let (key, container) = self.containers.next()?;
            self.current = Some((*key, container.iter()));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let rb = RoaringBitmap::new();
        assert!(rb.is_empty());
        assert_eq!(rb.len(), 0);
        assert!(!rb.contains(0));
    }

    #[test]
    fn test_add_and_contains() {
        let mut rb = RoaringBitmap::new();
        assert!(rb.add(42));
        assert!(rb.contains(42));
        assert!(!rb.contains(0));
        assert_eq!(rb.len(), 1);
    }

    #[test]
    fn test_add_duplicate() {
        let mut rb = RoaringBitmap::new();
        assert!(rb.add(100));
        assert!(!rb.add(100)); // duplicate
        assert_eq!(rb.len(), 1);
    }

    #[test]
    fn test_remove() {
        let mut rb = RoaringBitmap::new();
        rb.add(42);
        assert!(rb.remove(42));
        assert!(!rb.contains(42));
        assert!(rb.is_empty());
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut rb = RoaringBitmap::new();
        assert!(!rb.remove(999));
    }

    #[test]
    fn test_multi_containers() {
        let mut rb = RoaringBitmap::new();
        // Values across different high 16-bit keys
        rb.add(0);
        rb.add(70000); // key 1
        rb.add(200000); // key 3
        assert_eq!(rb.len(), 3);
        assert!(rb.contains(0));
        assert!(rb.contains(70000));
        assert!(rb.contains(200000));
    }

    #[test]
    fn test_auto_upgrade_to_bitmap() {
        let mut rb = RoaringBitmap::new();
        // Add values in a single container to force Array→Bitmap upgrade
        for i in 0..ARRAY_MAX_SIZE as u32 + 100 {
            rb.add(i);
        }
        assert_eq!(rb.len(), ARRAY_MAX_SIZE + 100);
        assert!(rb.contains(0));
        assert!(rb.contains(4096));
        assert!(rb.contains(4195));
    }

    #[test]
    fn test_union() {
        let mut a = RoaringBitmap::new();
        a.add(1);
        a.add(2);
        a.add(3);

        let mut b = RoaringBitmap::new();
        b.add(3);
        b.add(4);
        b.add(5);

        let c = a.union(&b);
        assert_eq!(c.len(), 5);
        assert!(c.contains(1));
        assert!(c.contains(5));
    }

    #[test]
    fn test_union_empty() {
        let a = RoaringBitmap::new();
        let mut b = RoaringBitmap::new();
        b.add(10);

        let c = a.union(&b);
        assert_eq!(c.len(), 1);
        assert!(c.contains(10));
    }

    #[test]
    fn test_intersection() {
        let mut a = RoaringBitmap::new();
        a.add(1);
        a.add(2);
        a.add(3);

        let mut b = RoaringBitmap::new();
        b.add(2);
        b.add(3);
        b.add(4);

        let c = a.intersection(&b);
        assert_eq!(c.len(), 2);
        assert!(c.contains(2));
        assert!(c.contains(3));
        assert!(!c.contains(1));
        assert!(!c.contains(4));
    }

    #[test]
    fn test_intersection_disjoint() {
        let mut a = RoaringBitmap::new();
        a.add(1);
        a.add(2);
        let mut b = RoaringBitmap::new();
        b.add(3);
        b.add(4);
        let c = a.intersection(&b);
        assert!(c.is_empty());
    }

    #[test]
    fn test_difference() {
        let mut a = RoaringBitmap::new();
        a.add(1);
        a.add(2);
        a.add(3);
        a.add(4);

        let mut b = RoaringBitmap::new();
        b.add(2);
        b.add(4);

        let c = a.difference(&b);
        assert_eq!(c.len(), 2);
        assert!(c.contains(1));
        assert!(c.contains(3));
    }

    #[test]
    fn test_difference_all() {
        let mut a = RoaringBitmap::new();
        a.add(5);
        a.add(6);
        let mut b = RoaringBitmap::new();
        b.add(5);
        b.add(6);
        let c = a.difference(&b);
        assert!(c.is_empty());
    }

    #[test]
    fn test_iter() {
        let mut rb = RoaringBitmap::new();
        rb.add(3);
        rb.add(1);
        rb.add(2);
        let vals: Vec<u32> = rb.iter().collect();
        assert_eq!(vals, vec![1, 2, 3]);
    }

    #[test]
    fn test_iter_multi_container() {
        let mut rb = RoaringBitmap::new();
        rb.add(0);
        rb.add(100000);
        rb.add(50000);
        let vals: Vec<u32> = rb.iter().collect();
        // Keys: 0, 0, 1 (sorted by key, then by low index)
        assert_eq!(vals, vec![0, 50000, 100000]);
    }

    #[test]
    fn test_from_sorted() {
        let values = vec![10u32, 20, 30, 100000, 200000];
        let rb = RoaringBitmap::from_sorted(&values);
        assert_eq!(rb.len(), 5);
        for v in &values {
            assert!(rb.contains(*v));
        }
        let collected: Vec<u32> = rb.iter().collect();
        assert_eq!(collected, values);
    }

    #[test]
    fn test_from_sorted_empty() {
        let rb = RoaringBitmap::from_sorted(&[]);
        assert!(rb.is_empty());
    }

    #[test]
    fn test_to_vec() {
        let mut rb = RoaringBitmap::new();
        rb.add(3);
        rb.add(1);
        rb.add(2);
        assert_eq!(rb.to_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn test_union_dense_chunks() {
        let mut a = RoaringBitmap::new();
        let mut b = RoaringBitmap::new();
        // Fill container 0 densely in both
        for i in 0..5000u32 {
            a.add(i);
        }
        for i in 3000..8000u32 {
            b.add(i);
        }
        let c = a.union(&b);
        assert_eq!(c.len(), 8000);
        assert!(c.contains(0));
        assert!(c.contains(7999));
    }

    #[test]
    fn test_intersection_dense_chunks() {
        let mut a = RoaringBitmap::new();
        let mut b = RoaringBitmap::new();
        for i in 0..5000u32 {
            a.add(i);
        }
        for i in 3000..8000u32 {
            b.add(i);
        }
        let c = a.intersection(&b);
        assert_eq!(c.len(), 2000); // 3000..5000
        assert!(c.contains(3000));
        assert!(c.contains(4999));
        assert!(!c.contains(2999));
        assert!(!c.contains(5000));
    }

    #[test]
    fn test_remove_after_upgrade() {
        let mut rb = RoaringBitmap::new();
        for i in 0..5000u32 {
            rb.add(i);
        }
        // Remove some elements from the bitmap container
        for i in 0..1000u32 {
            rb.remove(i);
        }
        assert_eq!(rb.len(), 4000);
        assert!(!rb.contains(0));
        assert!(rb.contains(1000));
    }

    #[test]
    fn test_union_with() {
        let mut a = RoaringBitmap::new();
        a.add(1);
        a.add(2);
        let mut b = RoaringBitmap::new();
        b.add(2);
        b.add(3);
        a.union_with(&b);
        assert_eq!(a.len(), 3);
        assert!(a.contains(1));
        assert!(a.contains(2));
        assert!(a.contains(3));
    }

    #[test]
    fn test_intersect_with() {
        let mut a = RoaringBitmap::new();
        a.add(1);
        a.add(2);
        a.add(3);
        let mut b = RoaringBitmap::new();
        b.add(2);
        b.add(3);
        b.add(4);
        a.intersect_with(&b);
        assert_eq!(a.len(), 2);
        assert!(a.contains(2));
        assert!(a.contains(3));
        assert!(!a.contains(1));
    }

    #[test]
    fn test_intersect_with_disjoint_containers() {
        let mut a = RoaringBitmap::new();
        a.add(1);
        a.add(100000); // key 1
        let mut b = RoaringBitmap::new();
        b.add(2);
        b.add(3);
        a.intersect_with(&b);
        assert!(a.is_empty());
    }

    #[test]
    fn test_difference_with() {
        let mut a = RoaringBitmap::new();
        a.add(1);
        a.add(2);
        a.add(3);
        let mut b = RoaringBitmap::new();
        b.add(2);
        a.difference_with(&b);
        assert_eq!(a.len(), 2);
        assert!(a.contains(1));
        assert!(a.contains(3));
        assert!(!a.contains(2));
    }
}
