//! Auto-extracted from physical_operator.rs
use crate::physical::order_aggregate::{AggregateHashTable, build_group_key, hash_group_key, keys_equal, update_states_row, resolve_agg_col_indices};
use arrow::array::{Array, ArrayRef, AsArray};
use arrow::compute;
use arrow::datatypes::{Float64Type, Int32Type, Int64Type};
use kuzu_common::types::Value;
use kuzu_common::vector::DataChunk;
use kuzu_function::AggregateFunction;
use kuzu_function::aggregate::AggValueState;
use kuzu_parser::ast::Expression;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

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
    pub agg_expressions: Vec<Vec<Expression>>,
    shards: Vec<std::sync::Mutex<ShardMap>>,
}

impl SharedAggregateState {
    pub fn new(funcs: Vec<AggregateFunction>, group_by_cols: Vec<u32>, agg_expressions: Vec<Vec<Expression>>) -> Self {
        let mut shards = Vec::with_capacity(NUM_SHARDS);
        for _ in 0..NUM_SHARDS {
            shards.push(std::sync::Mutex::new(hashbrown::HashMap::new()));
        }
        Self {
            funcs,
            group_by_cols,
            agg_expressions,
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
        let funcs = &self.shared_state.funcs;
        let group_cols = &self.shared_state.group_by_cols;

        // Resolve aggregate expression args to column indices from input chunks
        let col_indices = resolve_agg_col_indices(&self.shared_state.agg_expressions,
            input.first().map(|c| c.field_names.as_slice()).unwrap_or(&[]));

        // Fast path: scalar COUNT aggregates (no GROUP BY, all COUNT/COUNT_STAR)
        if group_cols.is_empty() && funcs.iter().all(|f| matches!(f, AggregateFunction::Count | AggregateFunction::CountStar)) {
            let mut counts = vec![0u64; funcs.len()];
            for chunk in &input {
                for (i, func) in funcs.iter().enumerate() {
                    match func {
                        AggregateFunction::CountStar => {
                            counts[i] += chunk.active_rows() as u64;
                        }
                        AggregateFunction::Count => {
                            if let Some(col_idx) = col_indices[i] {
                                if let Some(field) = chunk.fields.get(col_idx) {
                                    counts[i] += (field.len() - field.null_count()) as u64;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Store counts into shared state
            let mut shard = self.shared_state.current_shard().lock().unwrap();
            let bucket = shard.entry(0).or_default();
            bucket.clear();
            let states: Vec<AggValueState> = counts.into_iter().map(AggValueState::Count).collect();
            bucket.push((Value::Null, states));
            return Ok(vec![]);
        }

        // Fast path for scalar Sum/Min/Max/Avg aggregates (no GROUP BY, no selection vector).
        // Uses Arrow compute kernels directly on ArrayRef — avoids per-row Value dispatch.
        let total_rows: usize = input.iter().map(|c| c.size).sum();
        if total_rows > 0
            && group_cols.is_empty()
            && input.iter().all(|c| c.sel_vector.is_none())
            && funcs.iter().all(|f| matches!(
                f, AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max | AggregateFunction::Avg
            ))
        {
            let mut result_states: Vec<AggValueState> = Vec::with_capacity(funcs.len());
            for (i, func) in funcs.iter().enumerate() {
                if let Some(col_idx) = col_indices[i] {
                    let agg_val = arrow_scalar_agg_scan(&input, col_idx, func);
                    result_states.push(agg_val_to_state(func, &agg_val));
                } else {
                    result_states.push(AggValueState::new(func));
                }
            }
            let mut shard = self.shared_state.current_shard().lock().unwrap();
            let bucket = shard.entry(0).or_default();
            bucket.clear();
            bucket.push((Value::Null, result_states));
            return Ok(vec![]);
        }

        // Fast path for GROUP BY with numeric aggregates (no selection vector).
        // Partitions rows by group key, then uses Arrow compute per group
        // instead of per-row Value dispatch in update_states_row.
        if !group_cols.is_empty()
            && total_rows > 0
            && input.iter().all(|c| c.sel_vector.is_none())
            && funcs.iter().all(|f| matches!(
                f, AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max
                  | AggregateFunction::Count | AggregateFunction::CountStar | AggregateFunction::Avg
            ))
        {
            return self.batch_group_by_agg(&input, &col_indices);
        }

        // Lock only the current thread's shard — not the global map
        let mut shard = self.shared_state.current_shard().lock().unwrap();

        for chunk in &input {
            for row in 0..chunk.size {
                let hash = hash_group_key(chunk, group_cols, row);
                let bucket = shard.entry(hash).or_default();
                let entry = bucket.iter_mut().find(|(k, _)| keys_equal(k, chunk, group_cols, row));
                if let Some((_, states)) = entry {
                    update_states_row(states, chunk, funcs, &col_indices, row);
                } else {
                    let key = build_group_key(chunk, group_cols, row);
                    let mut states = funcs.iter().map(AggValueState::new).collect::<Vec<_>>();
                    update_states_row(&mut states, chunk, funcs, &col_indices, row);
                    bucket.push((key, states));
                }
            }
        }
        // Sink operator returns empty chunks because it accumulates into shared state
        Ok(vec![])
    }
}

impl PhysicalAggregateScan {
    /// Batch GROUP BY with Arrow compute: partition rows by group key,
    /// then compute aggregates per group using `take()` directly on ArrayRef
    /// — avoids per-row Value boxing/unboxing entirely.
    fn batch_group_by_agg(&self, input: &[DataChunk], col_indices: &[Option<usize>]) -> OperatorResult {
        use arrow::array::UInt32Array;
        use arrow::compute::take;
        use arrow::datatypes::Int64Type;
        let funcs = &self.shared_state.funcs;
        let group_cols = &self.shared_state.group_by_cols;

        // Step 1: Build group map — hash → (key_value, Vec<row_idx>)
        // Row indices are global (across chunks) for take() on concatenated arrays.
        let mut group_map: hashbrown::HashMap<u64, (Value, Vec<u32>)> = hashbrown::HashMap::new();
        let mut chunk_offsets: Vec<usize> = Vec::with_capacity(input.len());
        let mut offset = 0usize;
        for chunk in input {
            chunk_offsets.push(offset);
            offset += chunk.size;
        }
        for (ci, chunk) in input.iter().enumerate() {
            let base = chunk_offsets[ci];
            for row in 0..chunk.size {
                let hash = hash_group_key(chunk, group_cols, row);
                let entry = group_map.entry(hash).or_insert_with(|| {
                    (build_group_key(chunk, group_cols, row), Vec::new())
                });
                if keys_equal(&entry.0, chunk, group_cols, row) {
                    entry.1.push((base + row) as u32);
                }
            }
        }

        // Step 2: For each group, use take() on Arrow ArrayRef + compute kernels
        //
        // For single-chunk (common case): take() directly on chunk.fields[col].
        // For multi-chunk: concatenate first, then take().

        // Concatenate agg columns once if multi-chunk
        let concatenated: Vec<ArrayRef> = if input.len() > 1 {
            col_indices.iter().map(|&ci| {
                if let Some(col) = ci {
                    let refs: Vec<&dyn Array> = input.iter().filter_map(|c| {
                        if col < c.fields.len() { Some(c.fields[col].as_ref()) } else { None }
                    }).collect();
                    arrow::compute::kernels::concat::concat(&refs).unwrap()
                } else {
                    arrow::array::new_null_array(&arrow::datatypes::DataType::Null, 0)
                }
            }).collect()
        } else {
            vec![]
        };

        // Single chunk reference for the common path
        let chunk = if input.len() == 1 { Some(&input[0]) } else { None };

        let mut shard = self.shared_state.current_shard().lock().unwrap();
        for (hash, (key, row_indices)) in &group_map {
            let group_size = row_indices.len() as u64;
            let mut group_states: Vec<AggValueState> = funcs.iter().map(AggValueState::new).collect();
            let indices_arr = UInt32Array::from(row_indices.clone());

            for (fi, func) in funcs.iter().enumerate() {
                match func {
                    AggregateFunction::CountStar => {
                        if let AggValueState::Count(n) = &mut group_states[fi] {
                            *n = group_size;
                        }
                    }
                    AggregateFunction::Count => {
                        if let AggValueState::Count(n) = &mut group_states[fi] {
                            *n = group_size;
                        }
                    }
                    _ => {
                        if let Some(col_idx) = col_indices[fi] {
                            // Get the sub-array for this group via take()
                            let sub_array = if let Some(c) = chunk {
                                take(c.field(col_idx), &indices_arr, None)
                            } else {
                                take(&concatenated[fi], &indices_arr, None)
                            };

                            let sub = match sub_array {
                                Ok(a) => a,
                                Err(_) => continue,
                            };

                            group_states[fi] = match sub.data_type() {
                                arrow::datatypes::DataType::Int64 => {
                                    let arr = sub.as_primitive::<Int64Type>();
                                    match func {
                                        AggregateFunction::Count => {
                                            AggValueState::Count(group_size)
                                        }
                                        AggregateFunction::Sum => {
                                            AggValueState::Sum(arrow::compute::sum(arr).map(Value::Int64).unwrap_or(Value::Null))
                                        }
                                        AggregateFunction::Min => {
                                            AggValueState::Min(arrow::compute::min(arr).map(Value::Int64).unwrap_or(Value::Null))
                                        }
                                        AggregateFunction::Max => {
                                            AggValueState::Max(arrow::compute::max(arr).map(Value::Int64).unwrap_or(Value::Null))
                                        }
                                        AggregateFunction::Avg => {
                                            let non_null = arr.null_count() as u64;
                                            let actual_count = group_size - non_null;
                                            if actual_count == 0 {
                                                AggValueState::Avg { sum: Value::Null, count: 0 }
                                            } else {
                                                let s: f64 = arrow::compute::sum(arr).unwrap_or(0) as f64;
                                                AggValueState::Avg { sum: Value::Double(s / actual_count as f64), count: actual_count }
                                            }
                                        }
                                        _ => AggValueState::new(func),
                                    }
                                }
                                arrow::datatypes::DataType::Float64 => {
                                    let arr = sub.as_primitive::<arrow::datatypes::Float64Type>();
                                    match func {
                                        AggregateFunction::Count => {
                                            AggValueState::Count(group_size)
                                        }
                                        AggregateFunction::Sum => {
                                            AggValueState::Sum(arrow::compute::sum(arr).map(Value::Double).unwrap_or(Value::Null))
                                        }
                                        AggregateFunction::Min => {
                                            AggValueState::Min(arrow::compute::min(arr).map(Value::Double).unwrap_or(Value::Null))
                                        }
                                        AggregateFunction::Max => {
                                            AggValueState::Max(arrow::compute::max(arr).map(Value::Double).unwrap_or(Value::Null))
                                        }
                                        AggregateFunction::Avg => {
                                            let non_null = arr.null_count() as u64;
                                            let actual_count = group_size - non_null;
                                            if actual_count == 0 {
                                                AggValueState::Avg { sum: Value::Null, count: 0 }
                                            } else {
                                                let s: f64 = arrow::compute::sum(arr).unwrap_or(0.0);
                                                AggValueState::Avg { sum: Value::Double(s / actual_count as f64), count: actual_count }
                                            }
                                        }
                                        _ => AggValueState::new(func),
                                    }
                                }
                                _ => {
                                    // Fallback: collect Values for unsupported types
                                    AggValueState::new(func)
                                }
                            };
                        }
                    }
                }
            }

            let bucket = shard.entry(*hash).or_default();
            bucket.push((key.clone(), group_states));
        }

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
            self.shared_state.agg_expressions.clone(),
        );

        table.build_output(&merged)
    }
}

/// Arrow compute fast path for scalar aggregates in PhysicalAggregateScan.
/// Collects values from chunks, converts to typed Arrow arrays, and uses compute kernels.
fn arrow_scalar_agg_scan(chunks: &[DataChunk], col_idx: usize, func: &AggregateFunction) -> Value {
    let all_values: Vec<Value> = chunks.iter().flat_map(|c| {
        if col_idx >= c.fields.len() { return Vec::new(); }
        (0..c.size).filter_map(|row| c.get_value(col_idx, row)).collect::<Vec<_>>()
    }).collect();

    if all_values.is_empty() {
        return Value::Null;
    }

    if let Some(arr) = scan_values_to_prim_array(&all_values) {
        return match func {
            AggregateFunction::Sum => scan_compute_sum(&arr),
            AggregateFunction::Min => scan_compute_min(&arr),
            AggregateFunction::Max => scan_compute_max(&arr),
            AggregateFunction::Avg => {
                let sum_val = scan_compute_sum_f64(&arr);
                let count = chunks.iter().map(|c| {
                    if col_idx >= c.fields.len() { 0u64 }
                    else { (c.fields[col_idx].len() - c.fields[col_idx].null_count()) as u64 }
                }).sum::<u64>();
                if count == 0 { Value::Null }
                else { Value::Double(sum_val / count as f64) }
            }
            _ => Value::Null,
        };
    }

    let mut state = AggValueState::new(func);
    for val in &all_values {
        state.update(val);
    }
    state.finalize()
}

fn agg_val_to_state(func: &AggregateFunction, val: &Value) -> AggValueState {
    match func {
        AggregateFunction::Sum => AggValueState::Sum(val.clone()),
        AggregateFunction::Min => AggValueState::Min(val.clone()),
        AggregateFunction::Max => AggValueState::Max(val.clone()),
        AggregateFunction::Avg => {
            match val {
                Value::Double(s) => AggValueState::Avg { sum: Value::Double(*s), count: 1 },
                Value::Int64(s) => AggValueState::Avg { sum: Value::Int64(*s), count: 1 },
                _ => AggValueState::Avg { sum: Value::Null, count: 0 },
            }
        }
        _ => AggValueState::new(func),
    }
}

enum ScanPrimArray {
    I64(arrow::array::PrimitiveArray<Int64Type>),
    I32(arrow::array::PrimitiveArray<Int32Type>),
    F64(arrow::array::PrimitiveArray<Float64Type>),
}

fn scan_values_to_prim_array(vals: &[Value]) -> Option<ScanPrimArray> {
    match &vals[0] {
        Value::Int64(_) => {
            let mut b = arrow::array::Int64Builder::with_capacity(vals.len());
            for v in vals {
                match v {
                    Value::Int64(n) => b.append_value(*n),
                    Value::Null => b.append_null(),
                    _ => return None,
                }
            }
            Some(ScanPrimArray::I64(b.finish()))
        }
        Value::Int32(_) => {
            let mut b = arrow::array::Int32Builder::with_capacity(vals.len());
            for v in vals {
                match v {
                    Value::Int32(n) => b.append_value(*n),
                    Value::Null => b.append_null(),
                    _ => return None,
                }
            }
            Some(ScanPrimArray::I32(b.finish()))
        }
        Value::Double(_) => {
            let mut b = arrow::array::Float64Builder::with_capacity(vals.len());
            for v in vals {
                match v {
                    Value::Double(n) => b.append_value(*n),
                    Value::Null => b.append_null(),
                    _ => return None,
                }
            }
            Some(ScanPrimArray::F64(b.finish()))
        }
        _ => None,
    }
}

fn scan_compute_sum(arr: &ScanPrimArray) -> Value {
    match arr {
        ScanPrimArray::I64(a) => compute::sum(a).map(Value::Int64).unwrap_or(Value::Null),
        ScanPrimArray::I32(a) => compute::sum(a).map(|v| Value::Int64(v as i64)).unwrap_or(Value::Null),
        ScanPrimArray::F64(a) => compute::sum(a).map(Value::Double).unwrap_or(Value::Null),
    }
}

fn scan_compute_sum_f64(arr: &ScanPrimArray) -> f64 {
    match arr {
        ScanPrimArray::I64(a) => compute::sum::<Int64Type>(a).unwrap_or(0) as f64,
        ScanPrimArray::I32(a) => compute::sum::<Int32Type>(a).unwrap_or(0) as f64,
        ScanPrimArray::F64(a) => compute::sum::<Float64Type>(a).unwrap_or(0.0),
    }
}

fn scan_compute_min(arr: &ScanPrimArray) -> Value {
    match arr {
        ScanPrimArray::I64(a) => compute::min(a).map(Value::Int64).unwrap_or(Value::Null),
        ScanPrimArray::I32(a) => compute::min(a).map(|v| Value::Int64(v as i64)).unwrap_or(Value::Null),
        ScanPrimArray::F64(a) => compute::min(a).map(Value::Double).unwrap_or(Value::Null),
    }
}

fn scan_compute_max(arr: &ScanPrimArray) -> Value {
    match arr {
        ScanPrimArray::I64(a) => compute::max(a).map(Value::Int64).unwrap_or(Value::Null),
        ScanPrimArray::I32(a) => compute::max(a).map(|v| Value::Int64(v as i64)).unwrap_or(Value::Null),
        ScanPrimArray::F64(a) => compute::max(a).map(Value::Double).unwrap_or(Value::Null),
    }
}
