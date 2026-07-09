//! Auto-extracted from physical_operator.rs
use crate::physical::order_aggregate::{AggregateHashTable, build_group_key, update_states_row};
use kuzu_common::types::Value;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_function::AggregateFunction;
use kuzu_function::aggregate::AggValueState;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::common::{store_value_in_vector, value_cmp, value_hash};
use std::collections::BinaryHeap;


// ==================== SplitAggregation ====================

pub struct SharedAggregateState {
    pub funcs: Vec<AggregateFunction>,
    pub group_by_cols: Vec<u32>,
    pub groups: std::sync::Mutex<hashbrown::HashMap<u64, Vec<(Value, Vec<AggValueState>)>>>,
}

impl SharedAggregateState {
    pub fn new(funcs: Vec<AggregateFunction>, group_by_cols: Vec<u32>) -> Self {
        Self {
            funcs,
            group_by_cols,
            groups: std::sync::Mutex::new(hashbrown::HashMap::new()),
        }
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
        let mut groups = self.shared_state.groups.lock().unwrap();
        let funcs = &self.shared_state.funcs;
        let group_cols = &self.shared_state.group_by_cols;

        for chunk in &input {
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
        let groups = self.shared_state.groups.lock().unwrap();
        
        // Mock a dummy AggregateHashTable to reuse its output builder
        let dummy_table = AggregateHashTable::new(
            self.shared_state.funcs.clone(),
            self.shared_state.group_by_cols.clone(),
        );
        
        dummy_table.build_output(&groups)
    }
}
