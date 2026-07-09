//! Auto-extracted from physical_operator.rs
use kuzu_common::types::Value;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_function::AggregateFunction;
use kuzu_function::aggregate::AggValueState;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::common::{store_value_in_vector, value_cmp, value_hash};
use std::collections::BinaryHeap;


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

    pub fn build_output(&self, groups: &hashbrown::HashMap<u64, Vec<(Value, Vec<AggValueState>)>>) -> OperatorResult {
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
pub fn build_group_key(chunk: &DataChunk, group_cols: &[u32], row: usize) -> Value {
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
pub fn update_states_row(states: &mut [AggValueState], chunk: &DataChunk, funcs: &[AggregateFunction], row: usize) {
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

