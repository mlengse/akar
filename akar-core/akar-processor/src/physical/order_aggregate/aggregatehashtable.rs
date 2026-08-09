//! Auto-extracted from physical_operator.rs
use crate::physical::common::{hash_value_into, store_value_in_vector};
use crate::physical::types::OperatorResult;
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};
use akar_function::AggregateFunction;
use akar_function::aggregate::AggValueState;
use akar_parser::ast::Expression;
use arrow::compute;
use arrow::datatypes::{Float64Type, Int32Type, Int64Type};

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
    /// Aggregate function argument expressions (for resolving column indices).
    agg_expressions: Vec<Vec<Expression>>,
}

impl AggregateHashTable {
    pub fn new(funcs: Vec<AggregateFunction>, group_by_cols: Vec<u32>, agg_expressions: Vec<Vec<Expression>>) -> Self {
        Self {
            funcs,
            group_by_cols,
            agg_expressions,
        }
    }

    /// Aggregate all input chunks, optionally in parallel.
    pub fn aggregate(&self, chunks: &[DataChunk]) -> OperatorResult {
        let total_rows: usize = chunks.iter().map(|c| c.size).sum();

        // Resolve aggregate expression args to column indices
        let mut col_indices = resolve_agg_col_indices(
            &self.agg_expressions,
            chunks.first().map(|c| c.field_names.as_slice()).unwrap_or(&[]),
        );

        // PhysicalAggregate (simple path) carries no agg expressions, so each
        // function operates on the first input column. Guard against the empty
        // vec to keep the fast paths below from indexing out of bounds.
        if col_indices.is_empty() && !self.funcs.is_empty() {
            col_indices = vec![Some(0); self.funcs.len()];
        }

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
                store_value_in_vector(&mut v, 0, &result)?;
                fields.push(v);
            }
            return Ok(vec![{
                let arrow_fields = fields
                    .iter()
                    .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
                    .collect::<Vec<_>>();
                let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
                DataChunk::new(arrow_fields, arrow_field_types)
            }]);
        }

        if total_rows == 0 {
            return self.empty_result();
        }

        // Fast path for scalar COUNT aggregates (no GROUP BY)
        if self.group_by_cols.is_empty() {
            let all_count = self
                .funcs
                .iter()
                .all(|f| matches!(f, AggregateFunction::Count | AggregateFunction::CountStar));
            if all_count {
                let mut fields = Vec::new();
                for (i, func) in self.funcs.iter().enumerate() {
                    let mut total = 0u64;
                    for chunk in chunks {
                        match func {
                            AggregateFunction::CountStar => {
                                total += chunk.active_rows() as u64;
                            }
                            AggregateFunction::Count => {
                                if let Some(col_idx) = col_indices[i] {
                                    if let Some(field) = chunk.fields.get(col_idx) {
                                        if chunk.sel_vector.is_some() {
                                            for row in chunk.iter_rows() {
                                                if !field.is_null(row) {
                                                    total += 1;
                                                }
                                            }
                                        } else {
                                            total += (field.len() - field.null_count()) as u64;
                                        }
                                    }
                                } else {
                                    total += chunk.active_rows() as u64;
                                }
                            }
                            _ => {}
                        }
                    }
                    let state = AggValueState::Count(total);
                    let result = state.finalize();
                    let phys_type = result.physical_type();
                    let mut v = ValueVector::new(phys_type, 1);
                    v.resize(1);
                    store_value_in_vector(&mut v, 0, &result)?;
                    fields.push(v);
                }
                return Ok(vec![{
                    let arrow_fields = fields
                        .iter()
                        .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
                        .collect::<Vec<_>>();
                    let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
                    DataChunk::new(arrow_fields, arrow_field_types)
                }]);
            }
        }

        // Fast path for scalar Sum/Min/Max/Avg aggregates (no GROUP BY).
        // Uses Arrow compute kernels directly on ArrayRef — avoids per-row Value dispatch.
        if self.group_by_cols.is_empty() && chunks.iter().all(|c| c.sel_vector.is_none()) {
            let all_scalar_agg = self.funcs.iter().all(|f| {
                matches!(
                    f,
                    AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max | AggregateFunction::Avg
                )
            });
            if all_scalar_agg {
                let mut fields = Vec::new();
                for (i, func) in self.funcs.iter().enumerate() {
                    if let Some(col_idx) = col_indices[i] {
                        let result_val = arrow_scalar_agg(func, chunks, col_idx);
                        let phys_type = result_val.physical_type();
                        let mut v = ValueVector::new(phys_type, 1);
                        v.resize(1);
                        store_value_in_vector(&mut v, 0, &result_val)?;
                        fields.push(v);
                    } else {
                        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
                        v.resize(1);
                        v.set_null(0, true);
                        fields.push(v);
                    }
                }
                return Ok(vec![DataChunk::new(
                    fields
                        .iter()
                        .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
                        .collect(),
                    fields.iter().map(|v| v.physical_type()).collect(),
                )]);
            }
        }

        // Use rayon parallel aggregation for large inputs
        if total_rows > 1000 {
            self.aggregate_parallel(chunks, &col_indices)
        } else {
            self.aggregate_sequential(chunks, &col_indices)
        }
    }

    /// Parallel aggregation: split chunks across threads, aggregate locally, merge.
    fn aggregate_parallel(&self, chunks: &[DataChunk], col_indices: &[Option<usize>]) -> OperatorResult {
        use rayon::prelude::*;
        let total_rows: usize = chunks.iter().map(|c| c.size).sum();
        type LocalTable = hashbrown::HashMap<u64, Vec<(Value, Vec<AggValueState>)>>;

        // Each thread aggregates its portion
        let group_cols = &self.group_by_cols;
        let funcs = &self.funcs;

        let results: Vec<LocalTable> = chunks
            .par_iter()
            .map(|chunk| {
                let mut local: LocalTable = hashbrown::HashMap::with_capacity(chunk.size.max(16));
                for row in chunk.iter_rows() {
                    let hash = hash_group_key(chunk, group_cols, row);
                    let bucket = local.entry(hash).or_default();
                    let entry = bucket.iter_mut().find(|(k, _)| keys_equal(k, chunk, group_cols, row));
                    if let Some((_, states)) = entry {
                        update_states_row(states, chunk, funcs, col_indices, row);
                    } else {
                        let key = build_group_key(chunk, group_cols, row);
                        let mut states = funcs.iter().map(AggValueState::new).collect::<Vec<_>>();
                        update_states_row(&mut states, chunk, funcs, col_indices, row);
                        bucket.push((key, states));
                    }
                }
                local
            })
            .collect();

        // Merge all local tables
        let mut merged: LocalTable = hashbrown::HashMap::with_capacity(total_rows.max(16));
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
    fn aggregate_sequential(&self, chunks: &[DataChunk], col_indices: &[Option<usize>]) -> OperatorResult {
        let total_rows: usize = chunks.iter().map(|c| c.size).sum();
        let mut groups: hashbrown::HashMap<u64, Vec<(Value, Vec<AggValueState>)>> =
            hashbrown::HashMap::with_capacity(total_rows.max(16));
        let group_cols = &self.group_by_cols;
        let funcs = &self.funcs;

        for chunk in chunks {
            for row in chunk.iter_rows() {
                let hash = hash_group_key(chunk, group_cols, row);
                let bucket = groups.entry(hash).or_default();
                let entry = bucket.iter_mut().find(|(k, _)| keys_equal(k, chunk, group_cols, row));
                if let Some((_, states)) = entry {
                    update_states_row(states, chunk, funcs, col_indices, row);
                } else {
                    let key = build_group_key(chunk, group_cols, row);
                    let mut states = funcs.iter().map(AggValueState::new).collect::<Vec<_>>();
                    update_states_row(&mut states, chunk, funcs, col_indices, row);
                    bucket.push((key, states));
                }
            }
        }
        self.build_output(&groups)
    }

    fn empty_result(&self) -> OperatorResult {
        let num_cols = self.group_by_cols.len() + self.funcs.len();
        Ok(vec![DataChunk::new(
            Vec::with_capacity(num_cols),
            Vec::with_capacity(num_cols),
        )])
    }

    /// Scalar aggregate (no GROUP BY) over empty input: emit one row where each
    /// aggregate column holds its initial state's final value (COUNT=0, SUM=NULL,
    /// MIN/MAX/AVG=NULL, COLLECT=[], ...).
    fn empty_scalar_result(&self) -> OperatorResult {
        let mut fields = Vec::new();
        for func in &self.funcs {
            let state = AggValueState::new(func);
            let result = state.finalize();
            let phys_type = result.physical_type();
            let mut v = ValueVector::new(phys_type, 1);
            v.resize(1);
            store_value_in_vector(&mut v, 0, &result)?;
            fields.push(v);
        }
        Ok(vec![{
            let arrow_fields = fields
                .iter()
                .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
                .collect::<Vec<_>>();
            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
            DataChunk::new(arrow_fields, arrow_field_types)
        }])
    }

    pub fn build_output(&self, groups: &hashbrown::HashMap<u64, Vec<(Value, Vec<AggValueState>)>>) -> OperatorResult {
        if groups.is_empty() {
            // Scalar aggregate (no GROUP BY) over empty input must still emit
            // exactly one row with default values (COUNT=0, SUM/MIN/MAX/AVG=NULL).
            if self.group_by_cols.is_empty() && !self.funcs.is_empty() {
                return self.empty_scalar_result();
            }
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
                    store_value_in_vector(&mut v, row, key)?;
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
                        store_value_in_vector(&mut v, row, &val)?;
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
                store_value_in_vector(&mut v, row, val)?;
            }
            output.push(v);
        }

        // Split output into chunks of 2048
        const CHUNK_SIZE: usize = 2048;
        let mut chunks = Vec::new();

        for chunk_start in (0..num_rows).step_by(CHUNK_SIZE) {
            let chunk_end = (chunk_start + CHUNK_SIZE).min(num_rows);
            let chunk_len = chunk_end - chunk_start;

            let mut chunk_fields = Vec::with_capacity(output.len());
            for field in &output {
                let mut new_v = ValueVector::new(field.physical_type(), chunk_len);
                new_v.resize(chunk_len);
                for i in 0..chunk_len {
                    if field.is_null(chunk_start + i) {
                        new_v.set_null(i, true);
                    } else if let Some(val) = field.get_value(chunk_start + i) {
                        store_value_in_vector(&mut new_v, i, &val)?;
                    }
                }
                chunk_fields.push(new_v);
            }
            chunks.push({
                let arrow_fields = chunk_fields
                    .iter()
                    .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
                    .collect::<Vec<_>>();
                let arrow_field_types = chunk_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
                DataChunk::new(arrow_fields, arrow_field_types)
            });
        }

        Ok(chunks)
    }
}

/// Hash group key columns directly from Arrow arrays without creating intermediate `Value` objects.
/// For strings, avoids the `to_string()` allocation that `get_value()` would incur.
/// For primitives, avoids `Value` enum dispatch overhead.
pub fn hash_group_key(chunk: &DataChunk, group_cols: &[u32], row: usize) -> u64 {
    use std::hash::Hash;
    use std::hash::Hasher;
    let mut hasher = ahash::AHasher::default();
    for &gc in group_cols {
        let col = gc as usize;
        if col >= chunk.fields.len() {
            0u8.hash(&mut hasher);
            continue;
        }
        if chunk.is_null(col, row) {
            0u8.hash(&mut hasher);
            continue;
        }
        match chunk.field_types[col] {
            PhysicalTypeID::Int64 => {
                let v = chunk.get_i64(col, row).unwrap_or(0);
                v.hash(&mut hasher);
            }
            PhysicalTypeID::Int32 => {
                let v = chunk.get_i32(col, row).unwrap_or(0);
                v.hash(&mut hasher);
            }
            PhysicalTypeID::Double => {
                let v = chunk.get_f64(col, row).unwrap_or(0.0);
                v.to_bits().hash(&mut hasher);
            }
            PhysicalTypeID::Bool => {
                let v = chunk.get_bool(col, row).unwrap_or(false);
                v.hash(&mut hasher);
            }
            PhysicalTypeID::String => {
                if let Some(s) = chunk.get_string(col, row) {
                    s.hash(&mut hasher);
                }
            }
            _ => {
                // Fallback: create Value for types we don't handle directly
                if let Some(val) = chunk.get_value(col, row) {
                    hash_value_into(&val, &mut hasher);
                }
            }
        }
    }
    hasher.finish()
}

/// Check if a stored group key matches the current row's group column values.
/// Avoids creating Value::List or intermediate Value objects for comparison.
pub fn keys_equal(stored: &Value, chunk: &DataChunk, group_cols: &[u32], row: usize) -> bool {
    if group_cols.is_empty() {
        // Scalar aggregate without GROUP BY: every row belongs to the single
        // global group, so the stored `Value::Null` key always matches (P52.8).
        // Without this, each row became its own bucket entry with an identical
        // Null key, causing O(n^2) bucket scans and N duplicate output groups.
        return true;
    }
    if group_cols.len() == 1 {
        let col = group_cols[0] as usize;
        if col >= chunk.fields.len() {
            return *stored == Value::Null;
        }
        if chunk.is_null(col, row) {
            return *stored == Value::Null;
        }
        return match stored {
            Value::Int64(v) => chunk.get_i64(col, row).is_some_and(|x| x == *v),
            Value::Int32(v) => chunk.get_i32(col, row).is_some_and(|x| x == *v),
            Value::Double(v) => chunk.get_f64(col, row).is_some_and(|x| x == *v),
            Value::Bool(v) => chunk.get_bool(col, row).is_some_and(|x| x == *v),
            Value::String(v) => chunk.get_string(col, row).is_some_and(|x| x == *v),
            _ => {
                let val = chunk.get_value(col, row).unwrap_or(Value::Null);
                *stored == val
            }
        };
    }
    match stored {
        Value::List(vals) if vals.len() == group_cols.len() => vals.iter().enumerate().all(|(i, v)| {
            let col = group_cols[i] as usize;
            if col >= chunk.fields.len() {
                return *v == Value::Null;
            }
            if chunk.is_null(col, row) {
                return *v == Value::Null;
            }
            match v {
                Value::Int64(expected) => chunk.get_i64(col, row).is_some_and(|x| x == *expected),
                Value::Int32(expected) => chunk.get_i32(col, row).is_some_and(|x| x == *expected),
                Value::Double(expected) => chunk.get_f64(col, row).is_some_and(|x| x == *expected),
                Value::Bool(expected) => chunk.get_bool(col, row).is_some_and(|x| x == *expected),
                Value::String(expected) => chunk.get_string(col, row).is_some_and(|x| x == *expected),
                _ => {
                    let val = chunk.get_value(col, row).unwrap_or(Value::Null);
                    *v == val
                }
            }
        }),
        _ => false,
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
            .map(|_| chunk.get_value(group_cols[0] as usize, row))
            .unwrap_or(Some(Value::Null))
            .unwrap_or(Value::Null)
    } else {
        let vals: Vec<Value> = group_cols
            .iter()
            .map(|&gc| {
                chunk
                    .fields
                    .get(gc as usize)
                    .map(|_| chunk.get_value(gc as usize, row))
                    .unwrap_or(Some(Value::Null))
                    .unwrap_or(Value::Null)
            })
            .collect();
        Value::List(vals)
    }
}

/// Resolve an expression name to a column index using flexible matching.
/// Tries: exact match, numeric parse, ends_with(".name").
fn resolve_name_to_col(name: &str, field_names: &[String]) -> Option<usize> {
    if let Some(idx) = field_names.iter().position(|n| n == name) {
        return Some(idx);
    }
    if let Ok(idx) = name.parse::<usize>() {
        if idx < field_names.len() {
            return Some(idx);
        }
    }
    let dot_name = format!(".{}", name);
    if let Some(idx) = field_names.iter().position(|n| n.ends_with(&dot_name)) {
        return Some(idx);
    }
    None
}

/// Resolve aggregate function argument expressions to column indices.
/// Returns one Option per function: None means no column needed (e.g., COUNT(*)).
pub fn resolve_agg_col_indices(agg_expressions: &[Vec<Expression>], field_names: &[String]) -> Vec<Option<usize>> {
    agg_expressions
        .iter()
        .map(|args| {
            for expr in args {
                match expr {
                    Expression::Variable(name) => {
                        if let Some(idx) = resolve_name_to_col(name, field_names) {
                            return Some(idx);
                        }
                    }
                    Expression::PropertyAccess(base, prop) => {
                        if let Expression::Variable(prefix) = base.as_ref() {
                            // Try "prefix.prop" (exact match with Cypher variable name)
                            let qualified = format!("{}.{}", prefix, prop);
                            if let Some(idx) = field_names.iter().position(|n| n == &qualified) {
                                return Some(idx);
                            }
                            // Try ".prop" suffix match (handles table-prefixed names like "Person.age")
                            let dot_prop = format!(".{}", prop);
                            if let Some(idx) = field_names.iter().position(|n| n.ends_with(&dot_prop)) {
                                return Some(idx);
                            }
                        }
                    }
                    Expression::Star => return None,
                    _ => {}
                }
            }
            None
        })
        .collect()
}

/// Resolve GROUP BY expressions to actual column indices using field_names.
pub fn resolve_group_by_indices(group_by: &[Expression], field_names: &[String]) -> Vec<u32> {
    group_by
        .iter()
        .map(|expr| match expr {
            Expression::Variable(name) => resolve_name_to_col(name, field_names).unwrap_or(0) as u32,
            Expression::PropertyAccess(base, prop) => {
                if let Expression::Variable(prefix) = base.as_ref() {
                    let qualified = format!("{}.{}", prefix, prop);
                    if let Some(idx) = field_names.iter().position(|n| n == &qualified) {
                        return idx as u32;
                    }
                    let dot_prop = format!(".{}", prop);
                    if let Some(idx) = field_names.iter().position(|n| n.ends_with(&dot_prop)) {
                        return idx as u32;
                    }
                }
                0
            }
            _ => 0,
        })
        .collect()
}

/// Update aggregate states for a single row.
pub fn update_states_row(
    states: &mut [AggValueState],
    chunk: &DataChunk,
    funcs: &[AggregateFunction],
    col_indices: &[Option<usize>],
    row: usize,
) {
    for (i, state) in states.iter_mut().enumerate() {
        if matches!(funcs[i], AggregateFunction::CountStar) {
            if let AggValueState::Count(n) = state {
                *n += 1;
            }
            continue;
        }
        if col_indices.get(i).copied().flatten().is_none() && matches!(funcs[i], AggregateFunction::Count) {
            if let AggValueState::Count(n) = state {
                *n += 1;
            }
            continue;
        }
        let col_idx = col_indices
            .get(i)
            .copied()
            .flatten()
            .unwrap_or_else(|| i.min(chunk.fields.len().saturating_sub(1)));
        let val = chunk
            .fields
            .get(col_idx)
            .map(|_| chunk.get_value(col_idx, row))
            .unwrap_or(Some(Value::Null))
            .unwrap_or(Value::Null);
        state.update(&val);
    }
}

/// Compute a scalar aggregate (Sum/Min/Max/Avg) over chunks using Arrow compute kernels.
/// Avoids per-row Value dispatch entirely.
fn arrow_scalar_agg(func: &AggregateFunction, chunks: &[DataChunk], col_idx: usize) -> Value {
    // Collect non-null numeric values from all chunks into a single Arrow array
    let all_values: Vec<Value> = chunks
        .iter()
        .flat_map(|c| {
            if col_idx >= c.fields.len() {
                return Vec::new();
            }
            let field = &c.fields[col_idx];
            let rows = if c.sel_vector.is_some() {
                c.iter_rows().collect::<Vec<_>>()
            } else {
                (0..c.size).collect::<Vec<_>>()
            };
            rows.into_iter()
                .filter_map(|row| {
                    if field.is_null(row) {
                        None
                    } else {
                        c.get_value(col_idx, row)
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();

    if all_values.is_empty() {
        return match func {
            AggregateFunction::Sum | AggregateFunction::Avg => Value::Null,
            AggregateFunction::Min | AggregateFunction::Max => Value::Null,
            _ => Value::Null,
        };
    }

    // Try to use Arrow compute kernels on primitive arrays
    if let Some(arr) = values_to_prim_array(&all_values) {
        return match func {
            AggregateFunction::Sum => compute_sum(&arr),
            AggregateFunction::Min => compute_min(&arr),
            AggregateFunction::Max => compute_max(&arr),
            AggregateFunction::Avg => {
                let sum_val = compute_sum_f64(&arr);
                let count = non_null_count(chunks, col_idx);
                if count == 0 {
                    Value::Null
                } else {
                    Value::Double(sum_val / count as f64)
                }
            }
            _ => Value::Null,
        };
    }

    // Fallback: per-row Value dispatch (for non-numeric types)
    let mut state = AggValueState::new(func);
    for val in &all_values {
        state.update(val);
    }
    state.finalize()
}

enum PrimArray {
    I64(arrow::array::PrimitiveArray<Int64Type>),
    I32(arrow::array::PrimitiveArray<Int32Type>),
    F64(arrow::array::PrimitiveArray<Float64Type>),
}

fn values_to_prim_array(vals: &[Value]) -> Option<PrimArray> {
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
            Some(PrimArray::I64(b.finish()))
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
            Some(PrimArray::I32(b.finish()))
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
            Some(PrimArray::F64(b.finish()))
        }
        _ => None,
    }
}

fn compute_sum(arr: &PrimArray) -> Value {
    match arr {
        PrimArray::I64(a) => compute::sum(a).map(Value::Int64).unwrap_or(Value::Null),
        PrimArray::I32(a) => compute::sum(a).map(|v| Value::Int64(v as i64)).unwrap_or(Value::Null),
        PrimArray::F64(a) => compute::sum(a).map(Value::Double).unwrap_or(Value::Null),
    }
}

fn compute_sum_f64(arr: &PrimArray) -> f64 {
    match arr {
        PrimArray::I64(a) => compute::sum::<Int64Type>(a).unwrap_or(0) as f64,
        PrimArray::I32(a) => compute::sum::<Int32Type>(a).unwrap_or(0) as f64,
        PrimArray::F64(a) => compute::sum::<Float64Type>(a).unwrap_or(0.0),
    }
}

fn compute_min(arr: &PrimArray) -> Value {
    match arr {
        PrimArray::I64(a) => compute::min(a).map(Value::Int64).unwrap_or(Value::Null),
        PrimArray::I32(a) => compute::min(a).map(|v| Value::Int64(v as i64)).unwrap_or(Value::Null),
        PrimArray::F64(a) => compute::min(a).map(Value::Double).unwrap_or(Value::Null),
    }
}

fn compute_max(arr: &PrimArray) -> Value {
    match arr {
        PrimArray::I64(a) => compute::max(a).map(Value::Int64).unwrap_or(Value::Null),
        PrimArray::I32(a) => compute::max(a).map(|v| Value::Int64(v as i64)).unwrap_or(Value::Null),
        PrimArray::F64(a) => compute::max(a).map(Value::Double).unwrap_or(Value::Null),
    }
}

fn non_null_count(chunks: &[DataChunk], col_idx: usize) -> u64 {
    let mut count = 0u64;
    for c in chunks {
        if col_idx >= c.fields.len() {
            continue;
        }
        let field = &c.fields[col_idx];
        if c.sel_vector.is_some() {
            for row in c.iter_rows() {
                if !field.is_null(row) {
                    count += 1;
                }
            }
        } else {
            count += (field.len() - field.null_count()) as u64;
        }
    }
    count
}
