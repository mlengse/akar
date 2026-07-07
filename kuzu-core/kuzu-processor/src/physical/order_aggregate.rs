//! Auto-extracted from physical_operator.rs
use kuzu_common::types::Value;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_function::AggregateFunction;
use kuzu_function::aggregate::AggValueState;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::common::{store_value_in_vector, value_cmp, value_hash};
use std::collections::BinaryHeap;

// ==================== OrderBy ====================

pub struct PhysicalOrderBy {
    pub sort_keys: Vec<(u32, bool)>,
}

impl PhysicalOperatorExec for PhysicalOrderBy {
    fn operator_type(&self) -> &str {
        "order_by"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() {
            return Ok(Vec::new());
        }

        let total_rows: usize = input.iter().map(|c| c.size).sum();
        if total_rows == 0 {
            return Ok(input);
        }

        // Collect all values per column as Value (supports all types)
        let num_fields = input[0].num_fields();
        let mut all_values: Vec<Vec<(Value, bool)>> = (0..num_fields).map(|_| Vec::with_capacity(total_rows)).collect();

        for chunk in &input {
            for row in 0..chunk.size {
                for col in 0..num_fields {
                    if let Some(field) = chunk.fields.get(col) {
                        let val = field.get_value(row).unwrap_or(Value::Null);
                        let is_null = field.is_null(row);
                        all_values[col].push((val, is_null));
                    }
                }
            }
        }

        // Use BlockMergeSorter for large data, simple sort for small
        let block_size = 10000usize;
        let indices = if total_rows > block_size && !self.sort_keys.is_empty() {
            let sorter = BlockMergeSorter::new(block_size, self.sort_keys.clone());
            sorter.sort(&all_values, num_fields)
        } else {
            let mut indices: Vec<usize> = (0..total_rows).collect();
            if !self.sort_keys.is_empty() {
                indices.sort_by(|a, b| {
                    for &(col, ascending) in &self.sort_keys {
                        let col = col as usize;
                        if col >= num_fields {
                            continue;
                        }
                        let cmp = value_cmp(&all_values[col][*a].0, &all_values[col][*b].0);
                        if cmp != std::cmp::Ordering::Equal {
                            return if ascending { cmp } else { cmp.reverse() };
                        }
                    }
                    std::cmp::Ordering::Equal
                });
            }
            indices
        };

        // Build sorted output chunks (up to 100 rows per chunk)
        let chunk_size = 100usize;
        let mut output = Vec::new();
        for chunk_start in (0..total_rows).step_by(chunk_size) {
            let chunk_end = (chunk_start + chunk_size).min(total_rows);
            let size = chunk_end - chunk_start;
            let mut fields = Vec::new();
            for col in 0..num_fields {
                let first_val = &all_values[col][indices[chunk_start]].0;
                let phys_type = first_val.physical_type();
                let mut v = ValueVector::new(phys_type, size);
                v.resize(size);
                for (out_idx, &src_idx) in indices[chunk_start..chunk_end].iter().enumerate() {
                    let (ref val, is_null) = all_values[col][src_idx];
                    if is_null || matches!(val, Value::Null) {
                        v.set_null(out_idx, true);
                    } else {
                        store_value_in_vector(&mut v, out_idx, val);
                    }
                }
                fields.push(v);
            }
            output.push(DataChunk::new(fields));
        }
        Ok(output)
    }
}

/// Compare two Values for sorting. NULLs sort last.



// ==================== TopK ====================

/// Fused ORDER BY + LIMIT using a BinaryHeap (O(n log k) vs O(n log n)).
///
/// Maintains a max-heap of size (limit + offset). Pops the worst entry
/// when capacity is exceeded. Uses `DirectedSortKey` to encode sort
/// direction into the comparison, so the BinaryHeap's natural max-heap
/// behavior correctly retains the best entries.
pub struct PhysicalTopK {
    pub sort_keys: Vec<(u32, bool)>,
    pub limit: u64,
    pub offset: u64,
}

/// Wrapper for a sort-key value that embeds sort direction.
#[derive(Debug, Clone)]
enum DirectedSortKey {
    Asc(Value),
    Desc(Value),
}

impl Eq for DirectedSortKey {}
impl PartialEq for DirectedSortKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}
impl PartialOrd for DirectedSortKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for DirectedSortKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (DirectedSortKey::Asc(a), DirectedSortKey::Asc(b)) => value_cmp(a, b),
            (DirectedSortKey::Desc(a), DirectedSortKey::Desc(b)) => value_cmp(b, a),
            _ => std::cmp::Ordering::Equal,
        }
    }
}

#[derive(Debug, Clone)]
struct TopKHeapEntry {
    sort_key: Vec<DirectedSortKey>,
    row_idx: usize,
}

impl Eq for TopKHeapEntry {}
impl PartialEq for TopKHeapEntry {
    fn eq(&self, other: &Self) -> bool { self.sort_key == other.sort_key }
}
impl PartialOrd for TopKHeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) }
}
impl Ord for TopKHeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        for (a, b) in self.sort_key.iter().zip(other.sort_key.iter()) {
            let cmp = a.cmp(b);
            if cmp != std::cmp::Ordering::Equal { return cmp; }
        }
        std::cmp::Ordering::Equal
    }
}

impl PhysicalOperatorExec for PhysicalTopK {
    fn operator_type(&self) -> &str { "top_k" }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let capacity = (self.limit + self.offset) as usize;
        if capacity == 0 || input.is_empty() {
            return Ok(Vec::new());
        }

        let total_rows: usize = input.iter().map(|c| c.size).sum();
        if total_rows == 0 {
            return Ok(Vec::new());
        }

        let num_fields = input[0].num_fields();

        // Collect all values for random access
        let mut all_values: Vec<Vec<(Value, bool)>> = (0..num_fields)
            .map(|_| Vec::with_capacity(total_rows))
            .collect();
        for chunk in &input {
            for row in 0..chunk.size {
                for col in 0..num_fields {
                    if let Some(field) = chunk.fields.get(col) {
                        let val = field.get_value(row).unwrap_or(Value::Null);
                        let is_null = field.is_null(row);
                        all_values[col].push((val, is_null));
                    }
                }
            }
        }

        // BinaryHeap (max-heap): worst entry at top, popped when > capacity.
        let mut heap: BinaryHeap<TopKHeapEntry> =
            BinaryHeap::with_capacity(capacity.min(total_rows) + 1);

        for row_idx in 0..total_rows {
            let sort_key: Vec<DirectedSortKey> = self
                .sort_keys
                .iter()
                .map(|&(col, asc)| {
                    let val = if col as usize >= num_fields {
                        Value::Null
                    } else {
                        all_values[col as usize][row_idx].0.clone()
                    };
                    if asc { DirectedSortKey::Asc(val) } else { DirectedSortKey::Desc(val) }
                })
                .collect();

            heap.push(TopKHeapEntry { sort_key, row_idx });
            if heap.len() > capacity {
                heap.pop();
            }
        }

        // into_sorted_vec returns ascending DirectedSortKey order = best-first
        let sorted: Vec<TopKHeapEntry> = heap.into_sorted_vec();

        // Apply offset + limit
        let start = (self.offset as usize).min(sorted.len());
        let end = (start + self.limit as usize).min(sorted.len());
        let entries = &sorted[start..end];

        if entries.is_empty() {
            return Ok(Vec::new());
        }

        // Build output chunks (up to 100 rows each)
        let chunk_size = 100usize;
        let mut output = Vec::new();
        for chunk_start in (0..entries.len()).step_by(chunk_size) {
            let chunk_end = (chunk_start + chunk_size).min(entries.len());
            let size = chunk_end - chunk_start;
            let mut fields = Vec::new();
            for col in 0..num_fields {
                let first_row = entries[chunk_start].row_idx;
                let first_val = &all_values[col][first_row].0;
                let phys_type = first_val.physical_type();
                let mut v = ValueVector::new(phys_type, size);
                v.resize(size);
                for (out_idx, entry) in entries[chunk_start..chunk_end].iter().enumerate() {
                    let (ref val, is_null) = all_values[col][entry.row_idx];
                    if is_null || matches!(val, Value::Null) {
                        v.set_null(out_idx, true);
                    } else {
                        store_value_in_vector(&mut v, out_idx, val);
                    }
                }
                fields.push(v);
            }
            output.push(DataChunk::new(fields));
        }

        Ok(output)
    }
}

// ==================== RadixSort ====================

const RADIX_BITS: u32 = 8;
const RADIX_BUCKETS: usize = 1 << RADIX_BITS; // 256

/// LSD radix sort for Int64 indices. Sorts `indices` by the values in `keys`
/// (converted to u64 with sign flip so negative values sort before positive).
fn radix_sort_indices(indices: &mut [usize], keys: &[i64]) {
    let len = indices.len();
    if len < 2 {
        return;
    }

    let mut tmp_indices = vec![0usize; len];
    let mut tmp_keys = vec![0u64; len];

    // Flip sign bit so ordering is correct: smallest → largest
    for (i, &k) in keys.iter().enumerate() {
        tmp_keys[i] = (k as u64) ^ (1u64 << 63);
    }

    let mut counts = [0u32; RADIX_BUCKETS];

    for pass in 0..8u32 {
        // Count
        counts.fill(0);
        for &k in &tmp_keys {
            let bucket = ((k >> (pass * RADIX_BITS)) & 0xFF) as usize;
            counts[bucket] += 1;
        }

        // Prefix sum
        let mut total = 0u32;
        for c in counts.iter_mut() {
            let prev = *c;
            *c = total;
            total += prev;
        }

        // Scatter
        for (i, &k) in tmp_keys.iter().enumerate() {
            let bucket = ((k >> (pass * RADIX_BITS)) & 0xFF) as usize;
            let pos = counts[bucket] as usize;
            tmp_indices[pos] = indices[i];
            counts[bucket] += 1;
        }
        indices.copy_from_slice(&tmp_indices);

        // Rebuild keys in sorted order
        for (i, &idx) in indices.iter().enumerate() {
            tmp_keys[i] = (keys[idx] as u64) ^ (1u64 << 63);
        }
    }
}

/// Check if a sort key column contains only Int64 values (eligible for radix sort).
fn is_radix_eligible(values: &[(Value, bool)]) -> bool {
    values.iter().all(|(v, _)| matches!(v, Value::Int64(_) | Value::Null))
}


// ==================== BlockMergeSort ====================

/// Block-based parallel sort with k-way merge.
/// Splits data into blocks, sorts each block in parallel, then merges.
pub struct BlockMergeSorter {
    block_size: usize,
    sort_keys: Vec<(u32, bool)>,
}

impl BlockMergeSorter {
    pub fn new(block_size: usize, sort_keys: Vec<(u32, bool)>) -> Self {
        Self { block_size, sort_keys }
    }

    /// Sort data using block-based parallel sort + k-way merge.
    /// `all_values` is a per-column vector of (value, is_null).
    pub fn sort(&self, all_values: &[Vec<(Value, bool)>], num_fields: usize) -> Vec<usize> {
        let total_rows = all_values[0].len();
        if total_rows == 0 {
            return Vec::new();
        }

        let num_blocks = total_rows.div_ceil(self.block_size);

        if num_blocks <= 1 {
            // Single block: sort directly (possibly with radix)
            let mut indices: Vec<usize> = (0..total_rows).collect();
            self.sort_block(&mut indices, all_values, num_fields, 0, total_rows);
            return indices;
        }

        // Sort each block
        let sort_keys = self.sort_keys.clone();
        let block_size = self.block_size;
        let blocks: Vec<Vec<usize>> = (0..num_blocks)
            .map(|bi| {
                let start = bi * block_size;
                let end = (start + block_size).min(total_rows);
                let mut block_indices: Vec<usize> = (start..end).collect();
                Self::sort_block_static(&mut block_indices, all_values, num_fields, &sort_keys);
                block_indices
            })
            .collect();

        // K-way merge
        self.k_way_merge(&blocks, all_values, num_fields, total_rows)
    }

    fn sort_block(
        &self,
        indices: &mut [usize],
        all_values: &[Vec<(Value, bool)>],
        num_fields: usize,
        _start: usize,
        _end: usize,
    ) {
        Self::sort_block_static(indices, all_values, num_fields, &self.sort_keys);
    }

    fn sort_block_static(
        indices: &mut [usize],
        all_values: &[Vec<(Value, bool)>],
        num_fields: usize,
        sort_keys: &[(u32, bool)],
    ) {
        if sort_keys.is_empty() {
            return;
        }
        let (col, ascending) = sort_keys[0];
        let col = col as usize;
        if col >= num_fields {
            return;
        }

        // Try radix sort for Int64 keys
        if is_radix_eligible(&all_values[col]) {
            let keys: Vec<i64> = indices
                .iter()
                .map(|&i| match &all_values[col][i].0 {
                    Value::Int64(v) => *v,
                    _ => i64::MAX, // NULLs sort last
                })
                .collect();
            radix_sort_indices(indices, &keys);
            if !ascending {
                indices.reverse();
            }
            // Tie-break with remaining keys
            if sort_keys.len() > 1 {
                indices.sort_by(|a, b| {
                    for &(k, asc) in sort_keys {
                        let k = k as usize;
                        if k >= num_fields {
                            continue;
                        }
                        let cmp = value_cmp(&all_values[k][*a].0, &all_values[k][*b].0);
                        if cmp != std::cmp::Ordering::Equal {
                            return if asc { cmp } else { cmp.reverse() };
                        }
                    }
                    std::cmp::Ordering::Equal
                });
            }
        } else {
            // Comparison sort
            indices.sort_by(|a, b| {
                for &(k, ascending) in sort_keys {
                    let k = k as usize;
                    if k >= num_fields {
                        continue;
                    }
                    let cmp = value_cmp(&all_values[k][*a].0, &all_values[k][*b].0);
                    if cmp != std::cmp::Ordering::Equal {
                        return if ascending { cmp } else { cmp.reverse() };
                    }
                }
                std::cmp::Ordering::Equal
            });
        }
    }

    /// K-way merge of sorted blocks using linear scan for minimum.
    fn k_way_merge(
        &self,
        blocks: &[Vec<usize>],
        all_values: &[Vec<(Value, bool)>],
        num_fields: usize,
        total_rows: usize,
    ) -> Vec<usize> {
        let mut result = Vec::with_capacity(total_rows);
        let mut positions: Vec<usize> = vec![0usize; blocks.len()];

        for _ in 0..total_rows {
            // Find the block with the minimum head value
            let mut best_block: Option<usize> = None;
            for bi in 0..blocks.len() {
                if positions[bi] >= blocks[bi].len() {
                    continue; // Block exhausted
                }
                match best_block {
                    None => best_block = Some(bi),
                    Some(bb) => {
                        let cmp = self.compare_rows(
                            blocks[bb][positions[bb]],
                            blocks[bi][positions[bi]],
                            all_values,
                            num_fields,
                        );
                        if cmp == std::cmp::Ordering::Greater {
                            best_block = Some(bi);
                        }
                    }
                }
            }

            if let Some(bi) = best_block {
                result.push(blocks[bi][positions[bi]]);
                positions[bi] += 1;
            } else {
                break;
            }
        }

        result
    }

    fn compare_rows(
        &self,
        a: usize,
        b: usize,
        all_values: &[Vec<(Value, bool)>],
        num_fields: usize,
    ) -> std::cmp::Ordering {
        for &(k, ascending) in &self.sort_keys {
            let k = k as usize;
            if k >= num_fields {
                continue;
            }
            let cmp = value_cmp(&all_values[k][a].0, &all_values[k][b].0);
            if cmp != std::cmp::Ordering::Equal {
                return if ascending { cmp } else { cmp.reverse() };
            }
        }
        std::cmp::Ordering::Equal
    }
}


// ==================== Aggregate ====================

/// Helper: parse an aggregate function name string into an AggregateFunction enum.
fn parse_aggregate_function(name: &str) -> AggregateFunction {
    match name.to_uppercase().as_str() {
        "COUNT" => AggregateFunction::Count,
        "COUNT(*)" => AggregateFunction::CountStar,
        "SUM" => AggregateFunction::Sum,
        "AVG" => AggregateFunction::Avg,
        "MIN" => AggregateFunction::Min,
        "MAX" => AggregateFunction::Max,
        "COLLECT" => AggregateFunction::Collect,
        "STDDEV" => AggregateFunction::StdDev,
        "VARIANCE" => AggregateFunction::Variance,
        "PERCENTILE_DISC" => AggregateFunction::PercentileDisc { percentile: 0.5 },
        "PERCENTILE_CONT" => AggregateFunction::PercentileCont { percentile: 0.5 },
        _ => AggregateFunction::Count,
    }
}

pub struct PhysicalAggregate {
    pub group_by_cols: Vec<u32>,
    pub aggregate_functions: Vec<String>,
}

impl PhysicalOperatorExec for PhysicalAggregate {
    fn operator_type(&self) -> &str {
        "aggregate"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let funcs: Vec<AggregateFunction> = self
            .aggregate_functions
            .iter()
            .map(|name| parse_aggregate_function(name))
            .collect();

        let table = AggregateHashTable::new(funcs, self.group_by_cols.clone());
        table.aggregate(&input)
    }
}

/// Store a Value into a ValueVector at the given row index.


// ==================== AggregateHashTable ====================

/// A parallel hash table for aggregation with GROUP BY keys.
///
/// Uses `rayon` thread-local aggregation: each thread builds its own
/// local hash table, then all local tables are merged.
pub struct AggregateHashTable {
    /// Pre-parsed aggregate functions.
    funcs: Vec<AggregateFunction>,
    /// Group-by key column indices.
    group_by_cols: Vec<u32>,
}

impl AggregateHashTable {
    pub fn new(funcs: Vec<AggregateFunction>, group_by_cols: Vec<u32>) -> Self {
        Self { funcs, group_by_cols }
    }

    /// Aggregate all input chunks, optionally in parallel.
    pub fn aggregate(&self, chunks: &[DataChunk]) -> OperatorResult {
        let total_rows: usize = chunks.iter().map(|c| c.size).sum();

        if total_rows == 0 && self.group_by_cols.is_empty() {
            // Scalar aggregates on empty input: produce one row with default values
            // (COUNT=0, SUM=Null, etc.)
            let mut fields = Vec::new();
            for func in &self.funcs {
                let state = AggValueState::new(func);
                let result = state.finalize();
                let phys_type = result.physical_type();
                let mut v = ValueVector::new(phys_type, 1);
                v.resize(1);
                store_value_in_vector(&mut v, 0, &result);
                fields.push(v);
            }
            return Ok(vec![DataChunk::new(fields)]);
        }

        if total_rows == 0 {
            return self.empty_result();
        }

        // Use rayon parallel aggregation for large inputs
        if total_rows > 1000 {
            self.aggregate_parallel(chunks)
        } else {
            self.aggregate_sequential(chunks)
        }
    }

    /// Parallel aggregation: split chunks across threads, aggregate locally, merge.
    fn aggregate_parallel(&self, chunks: &[DataChunk]) -> OperatorResult {
        use rayon::prelude::*;
        type LocalTable = hashbrown::HashMap<u64, Vec<(Value, Vec<AggValueState>)>>;

        // Each thread aggregates its portion
        let group_cols = &self.group_by_cols;
        let funcs = &self.funcs;

        let results: Vec<LocalTable> = chunks
            .par_iter()
            .map(|chunk| {
                let mut local: LocalTable = hashbrown::HashMap::new();
                for row in 0..chunk.size {
                    let key = build_group_key(chunk, group_cols, row);
                    let hash = value_hash(&key);
                    let bucket = local.entry(hash).or_default();
                    let entry = bucket.iter_mut().find(|(k, _)| *k == key);
                    if let Some((_, states)) = entry {
                        update_states_row(states, chunk, funcs, row);
                    } else {
                        let mut states = funcs.iter().map(AggValueState::new).collect::<Vec<_>>();
                        update_states_row(&mut states, chunk, funcs, row);
                        bucket.push((key, states));
                    }
                }
                local
            })
            .collect();

        // Merge all local tables
        let mut merged: LocalTable = hashbrown::HashMap::new();
        for local in results {
            for (hash, bucket) in local {
                let mbucket = merged.entry(hash).or_default();
                for (key, states) in bucket {
                    let entry = mbucket.iter_mut().find(|(k, _)| *k == key);
                    if let Some((_, existing)) = entry {
                        for (i, s) in states.iter().enumerate() {
                            existing[i].merge(s);
                        }
                    } else {
                        mbucket.push((key, states));
                    }
                }
            }
        }
        self.build_output(&merged)
    }

    /// Sequential aggregation (small inputs).
    fn aggregate_sequential(&self, chunks: &[DataChunk]) -> OperatorResult {
        let mut groups: hashbrown::HashMap<u64, Vec<(Value, Vec<AggValueState>)>> = hashbrown::HashMap::new();
        let group_cols = &self.group_by_cols;
        let funcs = &self.funcs;

        for chunk in chunks {
            for row in 0..chunk.size {
                let key = build_group_key(chunk, group_cols, row);
                let hash = value_hash(&key);
                let bucket = groups.entry(hash).or_default();
                let entry = bucket.iter_mut().find(|(k, _)| *k == key);
                if let Some((_, states)) = entry {
                    update_states_row(states, chunk, funcs, row);
                } else {
                    let mut states = funcs.iter().map(AggValueState::new).collect::<Vec<_>>();
                    update_states_row(&mut states, chunk, funcs, row);
                    bucket.push((key, states));
                }
            }
        }
        self.build_output(&groups)
    }

    fn empty_result(&self) -> OperatorResult {
        let num_cols = self.group_by_cols.len() + self.funcs.len();
        Ok(vec![DataChunk::new(Vec::with_capacity(num_cols))])
    }

    fn build_output(&self, groups: &hashbrown::HashMap<u64, Vec<(Value, Vec<AggValueState>)>>) -> OperatorResult {
        if groups.is_empty() {
            return self.empty_result();
        }

        // Flatten buckets into parallel arrays
        let mut group_keys: Vec<Value> = Vec::new();
        let mut agg_results: Vec<Vec<Value>> = (0..self.funcs.len()).map(|_| Vec::new()).collect();

        for bucket in groups.values() {
            for (key, states) in bucket {
                group_keys.push(key.clone());
                for (i, state) in states.iter().enumerate() {
                    agg_results[i].push(state.finalize());
                }
            }
        }

        let num_group_cols = self.group_by_cols.len();
        let num_rows = group_keys.len();
        let mut output = Vec::with_capacity(num_group_cols + self.funcs.len());

        // Group key columns
        if num_group_cols == 1 {
            let first_val = &group_keys[0];
            let phys_type = first_val.physical_type();
            let mut v = ValueVector::new(phys_type, num_rows);
            v.resize(num_rows);
            for (row, key) in group_keys.iter().enumerate() {
                if matches!(key, Value::Null) {
                    v.set_null(row, true);
                } else {
                    store_value_in_vector(&mut v, row, key);
                }
            }
            output.push(v);
        } else {
            for gc_idx in 0..num_group_cols {
                let first_key = &group_keys[0];
                let inner_val = match first_key {
                    Value::List(vals) => vals.get(gc_idx).cloned().unwrap_or(Value::Null),
                    _ => Value::Null,
                };
                let phys_type = inner_val.physical_type();
                let mut v = ValueVector::new(phys_type, num_rows);
                v.resize(num_rows);
                for (row, key) in group_keys.iter().enumerate() {
                    let val = match key {
                        Value::List(vals) => vals.get(gc_idx).cloned().unwrap_or(Value::Null),
                        _ => Value::Null,
                    };
                    if matches!(val, Value::Null) {
                        v.set_null(row, true);
                    } else {
                        store_value_in_vector(&mut v, row, &val);
                    }
                }
                output.push(v);
            }
        }

        // Aggregate result columns
        for i in 0..self.funcs.len() {
            let first_val = &agg_results[i][0];
            let phys_type = first_val.physical_type();
            let mut v = ValueVector::new(phys_type, num_rows);
            v.resize(num_rows);
            for (row, val) in agg_results[i].iter().enumerate() {
                store_value_in_vector(&mut v, row, val);
            }
            output.push(v);
        }

        Ok(vec![DataChunk::new(output)])
    }
}

/// Build a composite group key from chunk columns.
fn build_group_key(chunk: &DataChunk, group_cols: &[u32], row: usize) -> Value {
    if group_cols.is_empty() {
        return Value::Null;
    }
    if group_cols.len() == 1 {
        chunk
            .fields
            .get(group_cols[0] as usize)
            .and_then(|f| f.get_value(row))
            .unwrap_or(Value::Null)
    } else {
        let vals: Vec<Value> = group_cols
            .iter()
            .map(|&gc| {
                chunk
                    .fields
                    .get(gc as usize)
                    .and_then(|f| f.get_value(row))
                    .unwrap_or(Value::Null)
            })
            .collect();
        Value::List(vals)
    }
}

/// Update aggregate states for a single row.
fn update_states_row(states: &mut [AggValueState], chunk: &DataChunk, funcs: &[AggregateFunction], row: usize) {
    for (i, state) in states.iter_mut().enumerate() {
        if matches!(funcs[i], AggregateFunction::CountStar) {
            if let AggValueState::Count(n) = state {
                *n += 1;
            }
            continue;
        }
        let col_idx = i.min(chunk.fields.len().saturating_sub(1));
        let val = chunk
            .fields
            .get(col_idx)
            .and_then(|f| f.get_value(row))
            .unwrap_or(Value::Null);
        state.update(&val);
    }
}

