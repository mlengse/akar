//! Auto-extracted from physical_operator.rs
use crate::physical::order_aggregate::{AggregateHashTable, build_group_key, update_states_row};
use kuzu_common::types::Value;
use kuzu_common::vector::DataChunk;
use kuzu_function::AggregateFunction;
use kuzu_function::aggregate::AggValueState;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::common::value_hash_fast;

const NUM_SHARDS: usize = 64;

type ShardMap = hashbrown::HashMap<u64, Vec<(Value, Vec<AggValueState>)>>;

// ==================== SplitAggregation ====================

/// Thread-local sharded aggregate state.
///
/// Replaces a single global `Mutex<HashMap>` with an array of `NUM_SHARDS`
/// shards, each protected by its own `Mutex`. The current thread's ID is
/// used to select a shard, reducing lock contention to only that shard.
pub struct SharedAggregateState {
    pub funcs: Vec<AggregateFunction>,
    pub group_by_cols: Vec<u32>,
    shards: Vec<std::sync::Mutex<ShardMap>>,
}

impl SharedAggregateState {
    pub fn new(funcs: Vec<AggregateFunction>, group_by_cols: Vec<u32>) -> Self {
        let mut shards = Vec::with_capacity(NUM_SHARDS);
        for _ in 0..NUM_SHARDS {
            shards.push(std::sync::Mutex::new(hashbrown::HashMap::new()));
        }
        Self {
            funcs,
            group_by_cols,
            shards,
        }
    }

    /// Pick the shard for the current thread based on its thread ID hash.
    fn current_shard(&self) -> &std::sync::Mutex<ShardMap> {
        let tid = std::thread::current().id();
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            tid.hash(&mut hasher);
            hasher.finish()
        };
        &self.shards[(hash as usize) % NUM_SHARDS]
    }

    /// Merge all shards into a single map for finalization.
    fn merge_all_shards(&self) -> ShardMap {
        let mut merged: ShardMap = hashbrown::HashMap::new();

        for shard in &self.shards {
            let guard = shard.lock().unwrap();
            for (hash, bucket) in guard.iter() {
                let mbucket = merged.entry(*hash).or_default();
                for (key, states) in bucket {
                    let entry = mbucket.iter_mut().find(|(k, _)| *k == *key);
                    if let Some((_, existing)) = entry {
                        for (i, s) in states.iter().enumerate() {
                            existing[i].merge(s);
                        }
                    } else {
                        mbucket.push((key.clone(), states.clone()));
                    }
                }
            }
        }

        merged
    }
}

pub struct PhysicalAggregateScan {
    pub shared_state: std::sync::Arc<SharedAggregateState>,
}

impl PhysicalOperatorExec for PhysicalAggregateScan {
    fn operator_type(&self) -> &str {
        "aggregate_scan"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Lock only the current thread's shard — not the global map
        let mut shard = self.shared_state.current_shard().lock().unwrap();
        let funcs = &self.shared_state.funcs;
        let group_cols = &self.shared_state.group_by_cols;

        for chunk in &input {
            for row in 0..chunk.size {
                let key = build_group_key(chunk, group_cols, row);
                let hash = value_hash(&key);
                let bucket = shard.entry(hash).or_default();
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
        // Sink operator returns empty chunks because it accumulates into shared state
        Ok(vec![])
    }
}

pub struct PhysicalAggregateFinalize {
    pub shared_state: std::sync::Arc<SharedAggregateState>,
}

impl PhysicalOperatorExec for PhysicalAggregateFinalize {
    fn operator_type(&self) -> &str {
        "aggregate_finalize"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // Merge all thread-local shards into one map
        let merged = self.shared_state.merge_all_shards();

        // Reuse AggregateHashTable's output builder for the merged result
        let table = AggregateHashTable::new(
            self.shared_state.funcs.clone(),
            self.shared_state.group_by_cols.clone(),
        );

        table.build_output(&merged)
    }
}
