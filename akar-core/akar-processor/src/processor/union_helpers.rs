use crate::physical::common::value_hash;
use crate::processor::chunk_helpers::{extract_all_rows_from_chunks, rows_to_columns};
use akar_common::error::ProcessorError;
use akar_common::types::Value;
use akar_common::vector::DataChunk;
use akar_planner::logical_operator::LogicalOperator;
use std::collections::HashMap;

pub fn flatten_union_child(op: &LogicalOperator) -> Vec<LogicalOperator> {
    match op {
        LogicalOperator::Projection(p) if p.expressions.is_empty() => p.children.clone(),
        other => vec![other.clone()],
    }
}

pub fn merge_union_chunks(
    left: Vec<DataChunk>,
    right: Vec<DataChunk>,
    all: bool,
) -> Result<Vec<DataChunk>, ProcessorError> {
    if left.is_empty() {
        return Ok(right);
    }
    if right.is_empty() {
        return Ok(left);
    }

    let num_fields = left[0].num_fields();
    for chunk in &right {
        if chunk.num_fields() != num_fields {
            return Err(format!(
                "UNION column count mismatch: left has {num_fields} columns, right has {} columns",
                chunk.num_fields()
            )
            .into());
        }
    }

    let mut left_rows = extract_all_rows_from_chunks(&left);
    let right_rows = extract_all_rows_from_chunks(&right);
    left_rows.extend(right_rows);

    let mut deduped: Vec<Vec<Value>> = Vec::with_capacity(left_rows.len());
    if !all {
        for row in &left_rows {
            if !deduped.contains(row) {
                deduped.push(row.clone());
            }
        }
    } else {
        deduped = left_rows;
    }

    if deduped.is_empty() {
        return Ok(vec![DataChunk::new(vec![], vec![])]);
    }

    let (fields, field_types) = rows_to_columns(&deduped);
    let final_size = deduped.len();
    let field_names = left.first().map(|c| c.field_names.clone()).unwrap_or_default();

    Ok(vec![DataChunk {
        fields,
        field_types,
        size: final_size,
        field_names,
        sel_vector: None,
    }])
}

pub fn merge_optional_chunks(left: Vec<DataChunk>, right: Vec<DataChunk>) -> Result<Vec<DataChunk>, ProcessorError> {
    if left.is_empty() {
        return Ok(left);
    }
    if right.is_empty() {
        return Ok(left);
    }

    let left_rows = extract_all_rows_from_chunks(&left);
    let right_rows = extract_all_rows_from_chunks(&right);
    if left_rows.is_empty() {
        return Ok(left);
    }

    let left_names: Vec<String> = left.first().map(|c| c.field_names.clone()).unwrap_or_default();
    let right_names: Vec<String> = right.first().map(|c| c.field_names.clone()).unwrap_or_default();
    let num_right_cols = right_rows.first().map(|r| r.len()).unwrap_or(right_names.len());

    if right_rows.is_empty() {
        // Left-outer join with no right matches: keep every left row and pad
        // the right columns with NULLs, preserving the right side's schema so
        // that RETURN can still resolve the optional columns (e.g. `m.id`).
        if num_right_cols == 0 {
            return Ok(left);
        }
        let mut combined: Vec<Vec<Value>> = Vec::with_capacity(left_rows.len());
        for lrow in &left_rows {
            let mut row = Vec::with_capacity(left_names.len() + num_right_cols);
            row.extend_from_slice(lrow);
            row.extend(std::iter::repeat_n(Value::Null, num_right_cols));
            combined.push(row);
        }
        let (fields, field_types) = rows_to_columns(&combined);
        let mut field_names = left_names;
        field_names.extend(right_names);
        return Ok(vec![DataChunk {
            fields,
            field_types,
            size: combined.len(),
            field_names,
            sel_vector: None,
        }]);
    }

    let num_left_cols = left_rows.first().map(|r| r.len()).unwrap_or(0);

    // OPTIONAL MATCH is a left-outer join on the variables shared between the
    // two sides. When a shared column exists (identical field name on both
    // sides) join on it; otherwise the two sides are independent and every
    // left row pairs with every right row (cross product). Positional merging
    // would silently drop right rows and mispair the i-th left row with the
    // i-th right row whenever cardinalities differ.
    let join_col_left = left_names.iter().position(|n| right_names.contains(n));
    let join_col_right = join_col_left.and_then(|li| right_names.iter().position(|n| *n == left_names[li]));

    let mut combined: Vec<Vec<Value>> = Vec::new();
    match (join_col_left, join_col_right) {
        (Some(li), Some(ri)) => {
            // Build a hash index over the right side's join column.
            let mut hash: HashMap<u64, Vec<usize>> = HashMap::new();
            for (i, row) in right_rows.iter().enumerate() {
                if let Some(val) = row.get(ri) {
                    if !matches!(val, Value::Null) {
                        hash.entry(value_hash(val)).or_default().push(i);
                    }
                }
            }
            for lrow in &left_rows {
                let key = lrow.get(li);
                let mut emitted = false;
                if let Some(key) = key {
                    if !matches!(key, Value::Null) {
                        if let Some(bucket) = hash.get(&value_hash(key)) {
                            for &i in bucket {
                                if &right_rows[i][ri] == key {
                                    let mut row = Vec::with_capacity(num_left_cols + num_right_cols);
                                    row.extend_from_slice(lrow);
                                    row.extend_from_slice(&right_rows[i]);
                                    combined.push(row);
                                    emitted = true;
                                }
                            }
                        }
                    }
                }
                if !emitted {
                    let mut row = Vec::with_capacity(num_left_cols + num_right_cols);
                    row.extend_from_slice(lrow);
                    row.extend(std::iter::repeat_n(Value::Null, num_right_cols));
                    combined.push(row);
                }
            }
        }
        _ => {
            // Cross product left-outer join: each left row pairs with all
            // right rows.
            for lrow in &left_rows {
                for rrow in &right_rows {
                    let mut row = Vec::with_capacity(num_left_cols + num_right_cols);
                    row.extend_from_slice(lrow);
                    row.extend_from_slice(rrow);
                    combined.push(row);
                }
            }
        }
    }

    if combined.is_empty() {
        return Ok(vec![]);
    }

    let (fields, field_types) = rows_to_columns(&combined);
    let size = combined.len();
    let mut field_names = left_names;
    field_names.extend(right_names);
    Ok(vec![DataChunk {
        fields,
        field_types,
        size,
        field_names,
        sel_vector: None,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_common::types::PhysicalTypeID;
    use akar_common::vector::ValueVector;

    fn make_chunk(cols: &[Vec<Value>], names: &[&str]) -> DataChunk {
        let mut fields = Vec::with_capacity(cols.len());
        let mut types = Vec::with_capacity(cols.len());
        for col in cols {
            let first = col.iter().find(|v| !matches!(v, Value::Null)).unwrap_or(&Value::Int64(0));
            let ptype = match first {
                Value::Int64(_) => PhysicalTypeID::Int64,
                Value::String(_) => PhysicalTypeID::String,
                _ => PhysicalTypeID::Int64,
            };
            let mut v = ValueVector::new(ptype, col.len().max(1));
            for (i, val) in col.iter().enumerate() {
                let _ = v.set_value(i, val);
            }
            v.resize(col.len());
            fields.push(akar_common::arrow_vector::ArrowVector::from_legacy(&v).array);
            types.push(ptype);
        }
        let mut chunk = DataChunk::new(fields, types);
        chunk.field_names = names.iter().map(|s| s.to_string()).collect();
        chunk
    }

    #[test]
    fn test_merge_optional_empty_right_returns_left() {
        let left = make_chunk(&[vec![Value::Int64(1), Value::Int64(2)]], &["a.id"]);
        let merged = merge_optional_chunks(vec![left.clone()], vec![]).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].size, 2);
    }

    #[test]
    fn test_merge_optional_zero_row_right_pads_nulls() {
        // Right side yields a 0-row chunk but still carries its schema. The
        // merge must keep both left rows and pad `m.id` with NULLs so RETURN
        // resolves the optional column instead of falling back to a wrong one.
        let left = make_chunk(&[vec![Value::Int64(1), Value::Int64(2)]], &["n.id"]);
        let right = make_chunk(&[Vec::<Value>::new()], &["m.id"]);
        let merged = merge_optional_chunks(vec![left], vec![right]).unwrap();
        assert_eq!(merged[0].size, 2, "every left row survives");
        assert_eq!(merged[0].field_names, vec!["n.id", "m.id"]);
        assert!(merged[0].is_null(1, 0), "m.id must be null");
        assert!(merged[0].is_null(1, 1), "m.id must be null");
    }

    #[test]
    fn test_merge_optional_cross_product_no_dropped_rows() {
        // Different cardinalities (2 left, 2 right) — no shared column, so the
        // merge is a cross product. Every left row is kept and no right row is
        // dropped.
        let left = make_chunk(
            &[vec![Value::Int64(1), Value::Int64(2)], vec![Value::Int64(10), Value::Int64(20)]],
            &["a.id", "a.x"],
        );
        let right = make_chunk(&[vec![Value::Int64(5), Value::Int64(6)]], &["b.id"]);
        let merged = merge_optional_chunks(vec![left], vec![right]).unwrap();
        assert_eq!(merged[0].size, 4, "expected 2 left rows x 2 right rows");
        assert_eq!(merged[0].field_names, vec!["a.id", "a.x", "b.id"]);
    }

    #[test]
    fn test_merge_optional_joins_on_shared_column() {
        // Shared column `a.id` on both sides → proper left-outer join.
        let left = make_chunk(
            &[vec![Value::Int64(1), Value::Int64(2)], vec![Value::String("Alice".into()), Value::String("Bob".into())]],
            &["a.id", "a.name"],
        );
        let right = make_chunk(
            &[vec![Value::Int64(1), Value::Int64(3)], vec![Value::String("Rex".into()), Value::String("Tom".into())]],
            &["a.id", "a.pet"],
        );
        let merged = merge_optional_chunks(vec![left], vec![right]).unwrap();
        assert_eq!(merged[0].size, 2, "Alice joins Rex; Bob has no match -> nulls");
        let pet_col = merged[0].get_string(3, 0).map(str::to_string);
        assert_eq!(pet_col.as_deref(), Some("Rex"));
        assert!(merged[0].is_null(3, 1), "Bob's pet must be null");
    }
}
