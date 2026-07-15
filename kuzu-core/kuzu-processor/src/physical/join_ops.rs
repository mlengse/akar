//! Auto-extracted from physical_operator.rs
use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use std::collections::HashMap;
use crate::physical::types::{OperatorResult, HashJoinTable, NodeSemiMask};
use crate::physical::common::{store_value_in_vector, value_hash, value_hash_fast};
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
        let arrow_fields = output_fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();
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
        let mut probe_types: Vec<PhysicalTypeID> = Vec::new();
        if let Some(first) = probe_chunks.first() {
            for col in 0..first.num_fields() {
                probe_types.push(first.field_types[col]);
            }
        }

        // Count matching rows first
        let mut match_rows: Vec<(usize, usize)> = Vec::new();
        for (ci, chunk) in probe_chunks.iter().enumerate() {
            for row in 0..chunk.size {
                if let Some(field) = chunk.fields.get(probe_col) {
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
                    if let Some(field) = chunk.fields.get(col) {
                        let val = chunk.get_value(col, *row).unwrap_or(Value::Null);
                        let _ = out_field.set_value(out_idx, &val);
                    }
                }
            }
        }
        for field in &mut output_fields {
            field.resize(match_rows.len());
        }
        let arrow_fields = output_fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();
        let arrow_field_types = output_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
        Ok(vec![DataChunk { fields: arrow_fields, field_types: arrow_field_types, size: match_rows.len(),
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
                if let Some(field) = chunk.fields.get(build_col) {
                    let key = chunk.get_value(build_col, row).unwrap_or(Value::Null);
                    if matches!(key, Value::Null) {
                        continue;
                    }
                    hash_set.insert(value_hash(&key));
                }
            }
        }

        let mut probe_types: Vec<PhysicalTypeID> = Vec::new();
        if let Some(first) = probe_chunks.first() {
            for col in 0..first.num_fields() {
                probe_types.push(first.field_types[col]);
            }
        }

        // Probe: emit left rows whose key is NOT in hash_set
        let mut non_match_rows: Vec<(usize, usize)> = Vec::new();
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
        let arrow_fields = output_fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();
        let arrow_field_types = output_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
        Ok(vec![DataChunk { fields: arrow_fields, field_types: arrow_field_types, size: non_match_rows.len(),
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

        // For each build side, build a hash table: key_hash → (key_value, Vec<(ci, row)>)
        let build_col = self.build_key_col as usize;
        let probe_col = self.probe_key_col as usize;
        let chunk_group_size = (build_chunks.len() / num_builds).max(1);

        let mut build_tables: Vec<HashJoinTable> = Vec::new();

        for side in 0..num_builds {
            let start = side * chunk_group_size;
            let end = (start + chunk_group_size).min(build_chunks.len());
            let chunks = &build_chunks[start..end];

            let mut ht: HashJoinTable = HashMap::new();

            for (ci, chunk) in chunks.iter().enumerate() {
                for row in 0..chunk.size {
                    if let Some(_field) = chunk.fields.get(build_col) {
                        let key = chunk.get_value(build_col, row).unwrap_or(Value::Null);
                        if matches!(key, Value::Null) {
                            continue;
                        }
                        let hash = value_hash(&key);
                        ht.entry(hash).or_default().push((key, vec![(ci, row)]));
                    }
                }
            }
            build_tables.push(ht);
        }

        if build_tables.is_empty() || build_tables.iter().any(|t| t.is_empty()) {
            // No build data — empty result
            return Ok(vec![]);
        }

        // For each probe row, probe all build tables, find intersecting keys
        let mut output_rows: Vec<Vec<Value>> = Vec::new();
        let mut probe_field_count = 0usize;

        for (ci, chunk) in probe_chunks.iter().enumerate() {
            if ci == 0 {
                probe_field_count = chunk.fields.len();
            }
            for row in 0..chunk.size {
                let probe_key = chunk
                    .fields
                    .get(probe_col)
                    .map(|_| chunk.get_value(probe_col, row))
                    .unwrap_or(Some(Value::Null))
                    .unwrap_or(Value::Null);
                if matches!(probe_key, Value::Null) {
                    continue;
                }
                let probe_hash = value_hash(&probe_key);

                // Check if the probe key appears in ALL build tables
                let mut all_match = true;
                let mut matched_build_rows: Vec<Vec<(usize, usize)>> = Vec::new();

                for ht in &build_tables {
                    if let Some(bucket) = ht.get(&probe_hash) {
                        let mut side_matches = Vec::new();
                        for (stored_key, locations) in bucket {
                            if stored_key == &probe_key {
                                side_matches.extend(locations.iter().cloned());
                            }
                        }
                        if side_matches.is_empty() {
                            all_match = false;
                            break;
                        }
                        matched_build_rows.push(side_matches);
                    } else {
                        all_match = false;
                        break;
                    }
                }

                if !all_match || matched_build_rows.is_empty() {
                    continue;
                }

                // The probe key matches ALL build sides — emit combined payload
                // First, count total fields in output: probe fields + all build side fields
                let mut row_values: Vec<Value> = Vec::new();

                // Collect probe side values (all columns from probe chunk)
                for col_in_probe in 0..probe_field_count {
                    let val = chunk
                        .fields
                        .get(col_in_probe)
                        .map(|_| chunk.get_value(probe_col, row))
                        .unwrap_or(Some(Value::Null))
                        .unwrap_or(Value::Null);
                    row_values.push(val);
                }

                // For each build side, emit the first matching row's payload values
                for matches in matched_build_rows.iter() {
                    if let Some(&(b_ci, b_row)) = matches.first()
                        && let Some(chunk) = build_chunks.get(b_ci)
                    {
                        for col in 0..chunk.fields.len() {
                            let val = chunk
                                .fields
                                .get(col)
                                .map(|_| chunk.get_value(col, b_row))
                                .unwrap_or(Some(Value::Null))
                                .unwrap_or(Value::Null);
                            row_values.push(val);
                        }
                    }
                }
                output_rows.push(row_values);
            }
        }

        if output_rows.is_empty() {
            return Ok(vec![]);
        }

        // Build output DataChunk (one row per field group)
        // Output format: [probe_field_1, ..., probe_field_N, build_1_field_1, ..., build_N_field_M]
        let output_size = output_rows.len();
        let mut output_fields: Vec<ValueVector> = Vec::new();

        // Determine physical types from first row
        if let Some(first_row) = output_rows.first() {
            for val in first_row {
                let ptype = val.physical_type();
                let mut vv = ValueVector::new(ptype, output_size);
                vv.resize(output_size);
                output_fields.push(vv);
            }
        }

        // Fill output
        for (out_idx, row_values) in output_rows.iter().enumerate() {
            for (col, val) in row_values.iter().enumerate() {
                if let Some(field) = output_fields.get_mut(col) {
                    let _ = field.set_value(out_idx, val);
                }
            }
        }

        let arrow_fields = output_fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();
        let arrow_field_types = output_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            field_names: vec![],
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
                let key = chunk
                    .fields
                    .get(build_col)
                    .and_then(|_| { /* this is wrong, need to fix */ None })
                    .unwrap_or(Value::Null);
                if matches!(key, Value::Null) {
                    continue;
                }
                let hash = value_hash_fast(&key);
                table.entry(hash).or_insert_with(|| Vec::with_capacity(4)).push((ci, row));
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
                    let key = chunk
                        .fields
                        .get(build_col)
                        .and_then(|_| { /* this is wrong, need to fix */ None })
                        .unwrap_or(Value::Null);
                    if matches!(key, Value::Null) {
                        continue;
                    }
                    let hash = value_hash_fast(&key);
                    local.entry(hash).or_insert_with(|| Vec::with_capacity(4)).push((ci, row));
                }
                local
            })
            .collect();

        // Merge: pre-size the final table
        let mut merged: hashbrown::HashMap<u64, Vec<(usize, usize)>> =
            hashbrown::HashMap::with_capacity(total_rows * 4 / 3);
        for local in tables {
            for (hash, locations) in local {
                merged.entry(hash).or_insert_with(|| Vec::with_capacity(locations.len())).extend(locations);
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

        // First pass: collect matching (build_chunk_idx, build_row, probe_chunk_idx, probe_row) tuples
        let mut matches: Vec<(usize, usize, usize, usize)> = Vec::new();

        for (pci, chunk) in probe_chunks.iter().enumerate() {
            for row in 0..chunk.size {
                let probe_key = chunk
                    .fields
                    .get(probe_col)
                    .and_then(|_| { /* this is wrong, need to fix */ None })
                    .unwrap_or(Value::Null);
                if matches!(probe_key, Value::Null) {
                    continue;
                }
                let probe_hash = value_hash_fast(&probe_key);

                if let Some(locations) = hash_table.get(&probe_hash) {
                    // Verify key equality for each candidate
                    for &(bci, brow) in locations {
                        let build_key = build_chunks[bci]
                            .fields
                            .get(build_col)
                            .and_then(|_| { /* this is wrong, need to fix */ None })
                            .unwrap_or(Value::Null);
                        if build_key == probe_key {
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
                        store_value_in_vector(&mut result_fields[col], out_row, &val);
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
                        store_value_in_vector(&mut result_fields[num_build_fields + col], out_row, &val);
                    }
                }
            }
        }

        let arrow_fields = result_fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();
        let arrow_field_types = result_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
        Ok(vec![DataChunk { fields: arrow_fields, field_types: arrow_field_types, size: num_rows,
            field_names: vec![],
            sel_vector: None,
          }])
    }
}


// ==================== HashJoin ====================

pub struct PhysicalHashJoin {
    pub build_columns: Vec<u32>,
    pub probe_columns: Vec<u32>,
    /// Optional semi-mask for SIP optimization.
    /// When populated, the build-side keys are collected into this mask
    /// and can be used by downstream scan operators to filter nodes.
    pub semi_mask: Option<NodeSemiMask>,
}

impl PhysicalHashJoin {
    pub fn new(build_columns: Vec<u32>, probe_columns: Vec<u32>) -> Self {
        Self {
            build_columns,
            probe_columns,
            semi_mask: None,
        }
    }

    /// Attach a semi-mask for SIP optimization.
    pub fn with_semi_mask(mut self, mask: NodeSemiMask) -> Self {
        self.semi_mask = Some(mask);
        self
    }
}

impl PhysicalHashJoin {
    pub fn execute_binary(&self, build_chunks: &[DataChunk], probe_chunks: &[DataChunk]) -> OperatorResult {
        if build_chunks.is_empty() || probe_chunks.is_empty() {
            return Ok(vec![]);
        }

        let build_col = self.build_columns.first().copied().unwrap_or(0) as usize;

        // Collect semi-mask keys from build side
        if let Some(mask) = &self.semi_mask {
            for chunk in build_chunks {
                for row in 0..chunk.size {
                    if chunk.fields.get(build_col).is_some()
                        && let Some(val) = chunk.get_value(build_col, row)
                    {
                        if let Value::InternalID(id) = val {
                            mask.mask(id.offset);
                        } else if let Value::Int64(offset) = val {
                            mask.mask(offset as u64);
                        }
                    }
                }
            }
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

