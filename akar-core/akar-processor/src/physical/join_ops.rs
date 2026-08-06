//! Auto-extracted from physical_operator.rs
use crate::physical::common::{hash_value_into, store_value_in_vector, value_hash};
use crate::physical::types::{HashJoinTable, OperatorResult};
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Hash a single DataChunk cell directly from its Arrow array, avoiding
/// intermediate `Value` creation (especially the string `to_string()` alloc).
/// Returns `None` for null values so callers can skip them like the old code
/// skipped `Value::Null`.
#[inline]
fn hash_chunk_cell(chunk: &DataChunk, col: usize, row: usize) -> Option<u64> {
    if col >= chunk.fields.len() || chunk.is_null(col, row) {
        return None;
    }
    let mut hasher = ahash::AHasher::default();
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
            if let Some(val) = chunk.get_value(col, row) {
                hash_value_into(&val, &mut hasher);
            }
        }
    }
    Some(hasher.finish())
}

/// Compare two DataChunk cells for join key equality without building Values.
#[inline]
fn chunk_cells_equal(
    left: &DataChunk,
    left_col: usize,
    left_row: usize,
    right: &DataChunk,
    right_col: usize,
    right_row: usize,
) -> bool {
    if left.is_null(left_col, left_row) || right.is_null(right_col, right_row) {
        return left.is_null(left_col, left_row) && right.is_null(right_col, right_row);
    }
    match (left.field_types[left_col], right.field_types[right_col]) {
        (PhysicalTypeID::Int64, PhysicalTypeID::Int64) => {
            left.get_i64(left_col, left_row) == right.get_i64(right_col, right_row)
        }
        (PhysicalTypeID::Int32, PhysicalTypeID::Int32) => {
            left.get_i32(left_col, left_row) == right.get_i32(right_col, right_row)
        }
        (PhysicalTypeID::Int64, PhysicalTypeID::Int32) | (PhysicalTypeID::Int32, PhysicalTypeID::Int64) => {
            let a = left.get_i64(left_col, left_row).or_else(|| left.get_i32(left_col, left_row).map(|v| v as i64));
            let b = right.get_i64(right_col, right_row).or_else(|| right.get_i32(right_col, right_row).map(|v| v as i64));
            a == b
        }
        (PhysicalTypeID::Double, PhysicalTypeID::Double) => {
            left.get_f64(left_col, left_row) == right.get_f64(right_col, right_row)
        }
        (PhysicalTypeID::Bool, PhysicalTypeID::Bool) => {
            left.get_bool(left_col, left_row) == right.get_bool(right_col, right_row)
        }
        (PhysicalTypeID::String, PhysicalTypeID::String) => {
            left.get_string(left_col, left_row) == right.get_string(right_col, right_row)
        }
        _ => left.get_value(left_col, left_row) == right.get_value(right_col, right_row),
    }
}
// ==================== CrossProduct ====================

/// Physical cross product (Cartesian product) operator.
///
/// Combines every row from the left side with every row from the right side.
/// The left side is the first half of input chunks, the right side is the
/// second half.
pub struct PhysicalCrossProduct;

impl PhysicalCrossProduct {
    pub fn execute_binary(&self, left_chunks: &[DataChunk], right_chunks: &[DataChunk]) -> OperatorResult {
        if left_chunks.is_empty() || right_chunks.is_empty() {
            return Ok(vec![]);
        }

        // Count total rows on each side
        let left_rows: usize = left_chunks.iter().map(|c| c.size).sum();
        let right_rows: usize = right_chunks.iter().map(|c| c.size).sum();

        if left_rows == 0 || right_rows == 0 {
            return Ok(vec![]);
        }

        // Collect left and right values into column-major Vec<Vec<Value>>
        let num_left_cols = left_chunks.first().map(|c| c.num_fields()).unwrap_or(0);
        let num_right_cols = right_chunks.first().map(|c| c.num_fields()).unwrap_or(0);
        let total_cols = num_left_cols + num_right_cols;
        let total_rows = left_rows * right_rows;

        let mut left_values: Vec<Vec<Value>> = (0..num_left_cols).map(|_| Vec::with_capacity(left_rows)).collect();
        for chunk in left_chunks {
            for col in 0..num_left_cols {
                if chunk.fields.get(col).is_some() {
                    for row in 0..chunk.size {
                        left_values[col].push(chunk.get_value(col, row).unwrap_or(Value::Null));
                    }
                }
            }
        }

        let mut right_values: Vec<Vec<Value>> = (0..num_right_cols).map(|_| Vec::with_capacity(right_rows)).collect();
        for chunk in right_chunks {
            for col in 0..num_right_cols {
                if chunk.fields.get(col).is_some() {
                    for row in 0..chunk.size {
                        right_values[col].push(chunk.get_value(col, row).unwrap_or(Value::Null));
                    }
                }
            }
        }

        // Build physical types and names for output
        let mut output_types: Vec<PhysicalTypeID> = Vec::with_capacity(total_cols);
        let mut field_names = Vec::with_capacity(total_cols);
        for col in 0..num_left_cols {
            if left_chunks[0].fields.get(col).is_some() {
                output_types.push(left_chunks[0].field_types[col]);
            }
        }
        for col in 0..num_right_cols {
            if right_chunks[0].fields.get(col).is_some() {
                output_types.push(right_chunks[0].field_types[col]);
            }
        }

        if let Some(c) = left_chunks.first() {
            field_names.extend(c.field_names.iter().cloned());
        }
        if let Some(c) = right_chunks.first() {
            field_names.extend(c.field_names.iter().cloned());
        }

        // Build output vectors
        let mut output_fields: Vec<ValueVector> = output_types
            .iter()
            .map(|t| ValueVector::new(*t, total_rows.max(1)))
            .collect();

        let mut out_row = 0usize;
        for lr in 0..left_rows {
            for rr in 0..right_rows {
                for (col, field) in output_fields.iter_mut().enumerate().take(num_left_cols) {
                    let val = &left_values[col][lr];
                    let _ = field.set_value(out_row, val);
                }
                for col in 0..num_right_cols {
                    let val = &right_values[col][rr];
                    let _ = output_fields[num_left_cols + col].set_value(out_row, val);
                }
                out_row += 1;
            }
        }

        for field in &mut output_fields {
            field.resize(total_rows);
        }

        // Propagate field names from left ++ right sides
        let mut output_names: Vec<String> = left_chunks.first().map(|c| c.field_names.clone()).unwrap_or_default();
        output_names.extend(right_chunks.first().map(|c| c.field_names.clone()).unwrap_or_default());
        let arrow_fields = output_fields
            .iter()
            .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
            .collect::<Vec<_>>();
        let arrow_field_types = output_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            size: total_rows,
            field_names: output_names,
            sel_vector: None,
        }])
    }
}

// ==================== SemiJoin ====================

/// Physical semi-join: Returns left rows that have a matching join key in the right side.
/// Only left-side columns are emitted (no right columns in output).
pub struct PhysicalSemiJoin {
    pub build_columns: Vec<u32>,
    pub probe_columns: Vec<u32>,
}

impl PhysicalSemiJoin {
    pub fn execute_binary(&self, build_chunks: &[DataChunk], probe_chunks: &[DataChunk]) -> OperatorResult {
        if build_chunks.is_empty() || probe_chunks.is_empty() {
            return Ok(vec![]);
        }

        let build_col = self.build_columns.first().copied().unwrap_or(0) as usize;
        let probe_col = self.probe_columns.first().copied().unwrap_or(0) as usize;

        // Build hash set of right-side keys
        let mut hash_set: std::collections::HashSet<u64> = std::collections::HashSet::new();
        for chunk in build_chunks {
            for row in 0..chunk.size {
                if chunk.fields.get(build_col).is_some() {
                    let key = chunk.get_value(build_col, row).unwrap_or(Value::Null);
                    if matches!(key, Value::Null) {
                        continue;
                    }
                    hash_set.insert(value_hash(&key));
                }
            }
        }

        // Probe: emit left rows whose key is in hash_set
        let num_probe_fields = probe_chunks.first().map(|c| c.num_fields()).unwrap_or(0);
        let mut probe_types: Vec<PhysicalTypeID> = Vec::with_capacity(num_probe_fields);
        if let Some(first) = probe_chunks.first() {
            for col in 0..first.num_fields() {
                probe_types.push(first.field_types[col]);
            }
        }

        let total_probe_rows: usize = probe_chunks.iter().map(|c| c.size).sum();
        let mut match_rows: Vec<(usize, usize)> = Vec::with_capacity(total_probe_rows);
        for (ci, chunk) in probe_chunks.iter().enumerate() {
            for row in 0..chunk.size {
                if chunk.fields.get(probe_col).is_some() {
                    let key = chunk.get_value(probe_col, row).unwrap_or(Value::Null);
                    if matches!(key, Value::Null) {
                        continue;
                    }
                    if hash_set.contains(&value_hash(&key)) {
                        match_rows.push((ci, row));
                    }
                }
            }
        }

        if match_rows.is_empty() {
            return Ok(vec![]);
        }

        // Build output with only left-side columns
        let num_left_cols = probe_types.len();
        let mut output_fields: Vec<ValueVector> = probe_types
            .iter()
            .map(|t| ValueVector::new(*t, match_rows.len().max(1)))
            .collect();

        for (out_idx, (ci, row)) in match_rows.iter().enumerate() {
            if let Some(chunk) = probe_chunks.get(*ci) {
                for (col, out_field) in output_fields.iter_mut().enumerate().take(num_left_cols) {
                    if chunk.fields.get(col).is_some() {
                        let val = chunk.get_value(col, *row).unwrap_or(Value::Null);
                        let _ = out_field.set_value(out_idx, &val);
                    }
                }
            }
        }
        for field in &mut output_fields {
            field.resize(match_rows.len());
        }
        let arrow_fields = output_fields
            .iter()
            .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
            .collect::<Vec<_>>();
        let arrow_field_types = output_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            size: match_rows.len(),
            field_names: vec![],
            sel_vector: None,
        }])
    }
}

// ==================== AntiJoin ====================

/// Physical anti-join: Returns left rows that have NO matching join key in the right side.
/// Only left-side columns are emitted.
pub struct PhysicalAntiJoin {
    pub build_columns: Vec<u32>,
    pub probe_columns: Vec<u32>,
}

impl PhysicalAntiJoin {
    pub fn execute_binary(&self, build_chunks: &[DataChunk], probe_chunks: &[DataChunk]) -> OperatorResult {
        if probe_chunks.is_empty() {
            return Ok(vec![]);
        }
        if build_chunks.is_empty() {
            // If build is empty, AntiJoin returns all of probe
            return Ok(probe_chunks.to_vec());
        }

        let build_col = self.build_columns.first().copied().unwrap_or(0) as usize;
        let probe_col = self.probe_columns.first().copied().unwrap_or(0) as usize;

        // Build hash set of right-side keys
        let mut hash_set: std::collections::HashSet<u64> = std::collections::HashSet::new();
        for chunk in build_chunks {
            for row in 0..chunk.size {
                if chunk.fields.get(build_col).is_some() {
                    let key = chunk.get_value(build_col, row).unwrap_or(Value::Null);
                    if matches!(key, Value::Null) {
                        continue;
                    }
                    hash_set.insert(value_hash(&key));
                }
            }
        }

        let num_probe_fields = probe_chunks.first().map(|c| c.num_fields()).unwrap_or(0);
        let mut probe_types: Vec<PhysicalTypeID> = Vec::with_capacity(num_probe_fields);
        if let Some(first) = probe_chunks.first() {
            for col in 0..first.num_fields() {
                probe_types.push(first.field_types[col]);
            }
        }

        let total_probe_rows: usize = probe_chunks.iter().map(|c| c.size).sum();
        let mut non_match_rows: Vec<(usize, usize)> = Vec::with_capacity(total_probe_rows);
        for (ci, chunk) in probe_chunks.iter().enumerate() {
            for row in 0..chunk.size {
                if let Some(_field) = chunk.fields.get(probe_col) {
                    let key = chunk.get_value(probe_col, row).unwrap_or(Value::Null);
                    if matches!(key, Value::Null) {
                        continue;
                    }
                    if !hash_set.contains(&value_hash(&key)) {
                        non_match_rows.push((ci, row));
                    }
                }
            }
        }

        if non_match_rows.is_empty() {
            return Ok(vec![]);
        }

        let num_left_cols = probe_types.len();
        let mut output_fields: Vec<ValueVector> = probe_types
            .iter()
            .map(|t| ValueVector::new(*t, non_match_rows.len().max(1)))
            .collect();

        for (out_idx, (ci, row)) in non_match_rows.iter().enumerate() {
            if let Some(chunk) = probe_chunks.get(*ci) {
                for (col, out_field) in output_fields.iter_mut().enumerate().take(num_left_cols) {
                    if let Some(_field) = chunk.fields.get(col) {
                        let val = chunk.get_value(col, *row).unwrap_or(Value::Null);
                        let _ = out_field.set_value(out_idx, &val);
                    }
                }
            }
        }
        for field in &mut output_fields {
            field.resize(non_match_rows.len());
        }
        let arrow_fields = output_fields
            .iter()
            .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
            .collect::<Vec<_>>();
        let arrow_field_types = output_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            size: non_match_rows.len(),
            field_names: vec![],
            sel_vector: None,
        }])
    }
}

// ==================== Intersect ====================

/// Physical intersect operator.
///
/// For multi-pattern matching like `MATCH (a)-[:r1]->(b), (a)-[:r2]->(c)`:
/// - Multiple build sides each produce a hash table keyed by the shared variable `a`
/// - The probe side produces candidate values for `a`
/// - For each probe key, all build hash tables are probed
/// - The matching node ID lists are pairwise intersected (two-way sorted merge)
/// - Only keys that appear in ALL build sides produce output
///
/// Implementation: a simplified version of the C++ `Intersect` (intersect.h).
/// Builds hash tables from build chunks, probes with probe chunks, and does
/// pairwise intersection using sorted node ID comparison.
pub struct PhysicalIntersect {
    /// Number of build hash tables (one per pattern).
    pub num_build_sides: u32,
    /// Column index of the key in the probe side.
    pub probe_key_col: u32,
    /// Column index of the key in each build side.
    pub build_key_col: u32,
}

impl PhysicalIntersect {
    pub fn execute_binary(&self, build_chunks: &[DataChunk], probe_chunks: &[DataChunk]) -> OperatorResult {
        let num_builds = self.num_build_sides.max(1) as usize;
        if build_chunks.is_empty() || probe_chunks.is_empty() {
            return Ok(vec![]);
        }

        // Partition the flat build chunk list into per-side groups. Each build
        // side in the plan produces the same number of chunks, so we split evenly.
        let chunk_group_size = (build_chunks.len() / num_builds).max(1);
        let mut sides: Vec<Vec<DataChunk>> = Vec::with_capacity(num_builds);
        for side in 0..num_builds {
            let start = side * chunk_group_size;
            let end = (start + chunk_group_size).min(build_chunks.len());
            sides.push(build_chunks[start..end].to_vec());
        }

        self.execute_sides(&sides, probe_chunks)
    }

    /// Execute the intersect against independently-produced build sides.
    ///
    /// Each build side is hashed into its own table keyed on the shared node ID.
    /// A probe row passes only when its key is present in EVERY build table, and
    /// the output is the full cross product of the matching build rows (so a
    /// shared node with `k` neighbors per side produces `k1 * k2 * ...` rows).
    ///
    /// Output column layout: `[probe columns] + [build side 1 columns] + ...`.
    pub fn execute_sides(&self, build_sides: &[Vec<DataChunk>], probe_chunks: &[DataChunk]) -> OperatorResult {
        let num_builds = build_sides.len().max(1);
        if probe_chunks.is_empty() {
            return Ok(vec![]);
        }

        let build_col = self.build_key_col as usize;
        let probe_col = self.probe_key_col as usize;

        // Build one hash table per side: key_hash → (key_value, Vec<(ci, row)>)
        let mut build_tables: Vec<HashJoinTable> = Vec::with_capacity(num_builds);
        let mut side_field_names: Vec<Vec<String>> = Vec::with_capacity(num_builds);
        let mut side_field_counts: Vec<usize> = Vec::with_capacity(num_builds);

        for side in build_sides {
            let mut ht: HashJoinTable = HashMap::new();
            let mut names: Vec<String> = Vec::new();
            let mut count = 0usize;

            for (ci, chunk) in side.iter().enumerate() {
                if ci == 0 {
                    names = chunk.field_names.clone();
                    count = chunk.fields.len();
                }
                for row in 0..chunk.size {
                    if chunk.fields.get(build_col).is_none() {
                        continue;
                    }
                    let key = chunk.get_value(build_col, row).unwrap_or(Value::Null);
                    if matches!(key, Value::Null) {
                        continue;
                    }
                    let hash = value_hash(&key);
                    ht.entry(hash).or_default().push((key, vec![(ci, row)]));
                }
            }

            side_field_names.push(names);
            side_field_counts.push(count);
            build_tables.push(ht);
        }

        if build_tables.iter().any(|t| t.is_empty()) {
            // A build side without data → no key can be in all sides → empty result
            return Ok(vec![]);
        }

        let probe_field_names = probe_chunks
            .first()
            .map(|c| c.field_names.clone())
            .unwrap_or_default();
        let probe_field_count = probe_chunks.first().map(|c| c.fields.len()).unwrap_or(0);
        let mut output_rows: Vec<Vec<Value>> = Vec::new();

        for (ci, chunk) in probe_chunks.iter().enumerate() {
            let _ = ci;
            for row in 0..chunk.size {
                let probe_key = chunk.get_value(probe_col, row).unwrap_or(Value::Null);
                if matches!(probe_key, Value::Null) {
                    continue;
                }
                let probe_hash = value_hash(&probe_key);

                // Collect matching (chunk_idx, row_idx) per build side.
                let mut matches_per_side: Vec<Vec<(usize, usize)>> = Vec::with_capacity(num_builds);
                let mut all_match = true;
                for ht in &build_tables {
                    let mut side_matches: Vec<(usize, usize)> = Vec::new();
                    if let Some(bucket) = ht.get(&probe_hash) {
                        for (stored_key, locations) in bucket {
                            if stored_key == &probe_key {
                                side_matches.extend(locations.iter().cloned());
                            }
                        }
                    }
                    if side_matches.is_empty() {
                        all_match = false;
                        break;
                    }
                    matches_per_side.push(side_matches);
                }
                if !all_match {
                    continue;
                }

                // Cross product across build sides.
                let mut combos: Vec<Vec<(usize, usize)>> = vec![vec![]];
                for side_matches in &matches_per_side {
                    let mut next = Vec::with_capacity(combos.len() * side_matches.len());
                    for combo in &combos {
                        for m in side_matches {
                            let mut c = combo.clone();
                            c.push(*m);
                            next.push(c);
                        }
                    }
                    combos = next;
                }

                let per_row_cols = probe_field_count + side_field_counts.iter().sum::<usize>();
                for combo in combos {
                    let mut row_values: Vec<Value> = Vec::with_capacity(per_row_cols);
                    // Probe side values — all columns of the probe row.
                    for col_in_probe in 0..probe_field_count {
                        row_values.push(chunk.get_value(col_in_probe, row).unwrap_or(Value::Null));
                    }
                    // One build side's payload per combo entry.
                    for (side_idx, &(b_ci, b_row)) in combo.iter().enumerate() {
                        if let Some(side_chunk) = build_sides.get(side_idx).and_then(|s| s.get(b_ci)) {
                            for col in 0..side_chunk.fields.len() {
                                row_values.push(side_chunk.get_value(col, b_row).unwrap_or(Value::Null));
                            }
                        }
                    }
                    output_rows.push(row_values);
                }
            }
        }

        if output_rows.is_empty() {
            return Ok(vec![]);
        }

        // Build output DataChunk (one row per field group)
        let output_size = output_rows.len();
        let mut output_fields: Vec<ValueVector> =
            Vec::with_capacity(output_rows.first().map(|r| r.len()).unwrap_or(0));

        if let Some(first_row) = output_rows.first() {
            for val in first_row {
                let ptype = val.physical_type();
                let mut vv = ValueVector::new(ptype, output_size);
                vv.resize(output_size);
                output_fields.push(vv);
            }
        }

        for (out_idx, row_values) in output_rows.iter().enumerate() {
            for (col, val) in row_values.iter().enumerate() {
                if let Some(field) = output_fields.get_mut(col) {
                    let _ = field.set_value(out_idx, val);
                }
            }
        }

        let arrow_fields = output_fields
            .iter()
            .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
            .collect::<Vec<_>>();
        let arrow_field_types = output_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();

        let mut field_names: Vec<String> = probe_field_names;
        for names in &side_field_names {
            field_names.extend(names.iter().cloned());
        }

        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            field_names,
            size: output_size,
            sel_vector: None,
        }])
    }
}

// ==================== JoinHashTable ====================

/// A hash table for hash join operations with parallel build support.
///
/// Optimized with:
/// - `ahash` for fast integer hashing (3-5× faster than SipHash)
/// - Flat bucket design: `HashMap<u64, Vec<(usize, usize)>>` — no Value cloning in buckets
/// - Pre-sized hash map based on total build rows (zero reallocations)
/// - Bulk output construction instead of per-row set_value
pub struct JoinHashTable {
    build_columns: Vec<u32>,
    probe_columns: Vec<u32>,
}

impl JoinHashTable {
    pub fn new(build_columns: Vec<u32>, probe_columns: Vec<u32>) -> Self {
        Self {
            build_columns,
            probe_columns,
        }
    }

    /// Build phase: create a flat hash table mapping key hashes to (chunk_idx, row_idx) pairs.
    /// The hash table is pre-sized and uses ahash for fast integer hashing.
    pub fn build(&self, build_chunks: &[DataChunk]) -> hashbrown::HashMap<u64, Vec<(usize, usize)>> {
        let total_rows: usize = build_chunks.iter().map(|c| c.size).sum();
        let build_col = self.build_columns.first().copied().unwrap_or(0) as usize;

        if total_rows > 1000 {
            self.build_parallel(build_chunks, build_col, total_rows)
        } else {
            self.build_sequential(build_chunks, build_col, total_rows)
        }
    }

    fn build_sequential(
        &self,
        build_chunks: &[DataChunk],
        build_col: usize,
        total_rows: usize,
    ) -> hashbrown::HashMap<u64, Vec<(usize, usize)>> {
        // Pre-size to ~75% load factor to avoid rehashing
        let mut table: hashbrown::HashMap<u64, Vec<(usize, usize)>> =
            hashbrown::HashMap::with_capacity(total_rows * 4 / 3);

        for (ci, chunk) in build_chunks.iter().enumerate() {
            for row in 0..chunk.size {
                let Some(hash) = hash_chunk_cell(chunk, build_col, row) else {
                    continue;
                };
                table
                    .entry(hash)
                    .or_insert_with(|| Vec::with_capacity(4))
                    .push((ci, row));
            }
        }
        table
    }

    fn build_parallel(
        &self,
        build_chunks: &[DataChunk],
        build_col: usize,
        total_rows: usize,
    ) -> hashbrown::HashMap<u64, Vec<(usize, usize)>> {
        use rayon::prelude::*;

        let tables: Vec<hashbrown::HashMap<u64, Vec<(usize, usize)>>> = build_chunks
            .par_iter()
            .enumerate()
            .map(|(ci, chunk)| {
                let mut local: hashbrown::HashMap<u64, Vec<(usize, usize)>> =
                    hashbrown::HashMap::with_capacity(chunk.size * 4 / 3);
                for row in 0..chunk.size {
                    let Some(hash) = hash_chunk_cell(chunk, build_col, row) else {
                        continue;
                    };
                    local
                        .entry(hash)
                        .or_insert_with(|| Vec::with_capacity(4))
                        .push((ci, row));
                }
                local
            })
            .collect();

        // Merge: pre-size the final table
        let mut merged: hashbrown::HashMap<u64, Vec<(usize, usize)>> =
            hashbrown::HashMap::with_capacity(total_rows * 4 / 3);
        for local in tables {
            for (hash, locations) in local {
                merged
                    .entry(hash)
                    .or_insert_with(|| Vec::with_capacity(locations.len()))
                    .extend(locations);
            }
        }
        merged
    }

    /// Probe phase: for each probe row, look up matching build rows by key hash,
    /// then verify key equality. Outputs combined build+probe columns.
    pub fn probe(
        &self,
        hash_table: &hashbrown::HashMap<u64, Vec<(usize, usize)>>,
        build_chunks: &[DataChunk],
        probe_chunks: &[DataChunk],
    ) -> OperatorResult {
        let probe_col = self.probe_columns.first().copied().unwrap_or(0) as usize;
        let build_col = self.build_columns.first().copied().unwrap_or(0) as usize;

        // Determine output schema from build + probe
        let num_build_fields = build_chunks.first().map(|c| c.num_fields()).unwrap_or(0);
        let num_probe_fields = probe_chunks.first().map(|c| c.num_fields()).unwrap_or(0);
        let total_cols = num_build_fields + num_probe_fields;

        if total_cols == 0 {
            return Ok(Vec::new());
        }

        // Collect output types
        let mut output_types: Vec<PhysicalTypeID> = Vec::with_capacity(total_cols);
        if let Some(bc) = build_chunks.first() {
            for col in 0..bc.num_fields() {
                output_types.push(bc.field_types[col]);
            }
        }
        if let Some(pc) = probe_chunks.first() {
            for col in 0..pc.num_fields() {
                output_types.push(pc.field_types[col]);
            }
        }

        let total_probe_rows: usize = probe_chunks.iter().map(|c| c.size).sum();
        let mut matches: Vec<(usize, usize, usize, usize)> = Vec::with_capacity(total_probe_rows);

        for (pci, chunk) in probe_chunks.iter().enumerate() {
            for row in 0..chunk.size {
                let Some(probe_hash) = hash_chunk_cell(chunk, probe_col, row) else {
                    continue;
                };

                if let Some(locations) = hash_table.get(&probe_hash) {
                    // Verify key equality for each candidate
                    for &(bci, brow) in locations {
                        if chunk_cells_equal(&build_chunks[bci], build_col, brow, chunk, probe_col, row) {
                            matches.push((bci, brow, pci, row));
                        }
                    }
                }
            }
        }

        if matches.is_empty() {
            return Ok(Vec::new());
        }

        // Second pass: build output DataChunk from collected matches
        let num_rows = matches.len();
        let mut result_fields: Vec<ValueVector> = output_types
            .iter()
            .map(|t| {
                let mut v = ValueVector::new(*t, num_rows);
                v.resize(num_rows);
                v
            })
            .collect();

        for (out_row, &(bci, brow, pci, prow)) in matches.iter().enumerate() {
            // Copy build-side columns
            for col in 0..num_build_fields {
                if let Some(_field) = build_chunks[bci].fields.get(col) {
                    let val = build_chunks[bci].get_value(col, brow).unwrap_or(Value::Null);
                    if matches!(val, Value::Null) {
                        result_fields[col].set_null(out_row, true);
                    } else {
                        store_value_in_vector(&mut result_fields[col], out_row, &val)?;
                    }
                }
            }
            // Copy probe-side columns
            for col in 0..num_probe_fields {
                if let Some(_field) = probe_chunks[pci].fields.get(col) {
                    let val = probe_chunks[pci].get_value(col, prow).unwrap_or(Value::Null);
                    if matches!(val, Value::Null) {
                        result_fields[num_build_fields + col].set_null(out_row, true);
                    } else {
                        store_value_in_vector(&mut result_fields[num_build_fields + col], out_row, &val)?;
                    }
                }
            }
        }

        let arrow_fields = result_fields
            .iter()
            .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
            .collect::<Vec<_>>();
        let arrow_field_types = result_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            size: num_rows,
            field_names: vec![],
            sel_vector: None,
        }])
    }
}

// ==================== HashJoin ====================

pub struct PhysicalHashJoin {
    pub build_columns: Vec<u32>,
    pub probe_columns: Vec<u32>,
}

impl PhysicalHashJoin {
    pub fn new(build_columns: Vec<u32>, probe_columns: Vec<u32>) -> Self {
        Self {
            build_columns,
            probe_columns,
        }
    }
}

impl PhysicalHashJoin {
    pub fn execute_binary(&self, build_chunks: &[DataChunk], probe_chunks: &[DataChunk]) -> OperatorResult {
        if build_chunks.is_empty() || probe_chunks.is_empty() {
            return Ok(vec![]);
        }

        // Use JoinHashTable for parallel build
        let join_table = JoinHashTable::new(self.build_columns.clone(), self.probe_columns.clone());
        let hash_table = join_table.build(build_chunks);
        let mut result = join_table.probe(&hash_table, build_chunks, probe_chunks)?;

        // Propagate field names
        if !result.is_empty() {
            let mut output_names: Vec<String> = build_chunks.first().map(|c| c.field_names.clone()).unwrap_or_default();
            output_names.extend(probe_chunks.first().map(|c| c.field_names.clone()).unwrap_or_default());
            result[0].field_names = output_names;
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_i64_chunk(values: &[i64]) -> DataChunk {
        let mut v = ValueVector::new(PhysicalTypeID::Int64, values.len().max(1));
        for (i, val) in values.iter().enumerate() {
            v.set_i64(i, *val);
        }
        v.resize(values.len());
        let ptype = v.physical_type();
        let fields = vec![akar_common::arrow_vector::ArrowVector::from_legacy(&v).array];
        DataChunk::new(fields, vec![ptype])
    }

    #[test]
    fn test_intersect_execute_sides_cross_product() {
        let intersect = PhysicalIntersect {
            num_build_sides: 2,
            probe_key_col: 0,
            build_key_col: 0,
        };
        let build1 = make_i64_chunk(&[1, 1, 5]);
        let build2 = make_i64_chunk(&[1, 1, 1, 7]);
        let probe = make_i64_chunk(&[1, 2]);
        let sides = vec![vec![build1], vec![build2]];
        let result = intersect.execute_sides(&sides, &[probe]).unwrap();
        assert!(!result.is_empty(), "expected non-empty result");
        assert_eq!(result[0].size, 6, "expected 2x3 cross product for probe key 1");
        assert_eq!(result[0].fields.len(), 3, "probe + 2 build columns");
    }

    #[test]
    fn test_intersect_execute_sides_key_resolution() {
        let mut probe_v = ValueVector::new(PhysicalTypeID::Int64, 2);
        probe_v.set_i64(0, 10);
        probe_v.set_i64(1, 20);
        let mut probe_id = ValueVector::new(PhysicalTypeID::Int64, 2);
        probe_id.set_i64(0, 1);
        probe_id.set_i64(1, 2);
        let ptype = probe_v.physical_type();
        let probe_fields = vec![
            akar_common::arrow_vector::ArrowVector::from_legacy(&probe_v).array,
            akar_common::arrow_vector::ArrowVector::from_legacy(&probe_id).array,
        ];
        let mut probe = DataChunk::new(probe_fields, vec![ptype, ptype]);
        probe.field_names = vec!["a.other".into(), "a.id".into()];

        let mut build_v = ValueVector::new(PhysicalTypeID::Int64, 2);
        build_v.set_i64(0, 30);
        build_v.set_i64(1, 40);
        let mut build_id = ValueVector::new(PhysicalTypeID::Int64, 2);
        build_id.set_i64(0, 1);
        build_id.set_i64(1, 1);
        let build_fields = vec![
            akar_common::arrow_vector::ArrowVector::from_legacy(&build_v).array,
            akar_common::arrow_vector::ArrowVector::from_legacy(&build_id).array,
        ];
        let mut build = DataChunk::new(build_fields, vec![ptype, ptype]);
        build.field_names = vec!["a.other".into(), "a.id".into()];

        let intersect = PhysicalIntersect {
            num_build_sides: 1,
            probe_key_col: 1,
            build_key_col: 1,
        };
        let result = intersect.execute_sides(&vec![vec![build]], &vec![probe]).unwrap();
        assert!(!result.is_empty(), "expected non-empty result");
        assert_eq!(result[0].size, 2, "probe id 1 matches both build rows; id 2 matches nothing");
        assert_eq!(result[0].field_names, vec!["a.other".to_string(), "a.id".to_string(), "a.other".to_string(), "a.id".to_string()]);
    }
}
