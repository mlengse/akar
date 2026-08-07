use crate::processor::chunk_helpers::{extract_all_rows_from_chunks, rows_to_columns};
use akar_common::error::ProcessorError;
use akar_common::types::Value;
use akar_common::vector::DataChunk;
use akar_planner::logical_operator::LogicalOperator;

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

    let num_left_cols = left_rows.first().map(|r| r.len()).unwrap_or(0);
    let num_right_cols = right_rows.first().map(|r| r.len()).unwrap_or(0);
    let max_rows = left_rows.len();

    let mut combined: Vec<Vec<Value>> = Vec::with_capacity(max_rows);
    for i in 0..max_rows {
        let mut row = Vec::with_capacity(num_left_cols + num_right_cols);
        if i < left_rows.len() {
            row.extend_from_slice(&left_rows[i]);
        }
        if i < right_rows.len() {
            row.extend_from_slice(&right_rows[i]);
        } else {
            row.extend(std::iter::repeat_n(Value::Null, num_right_cols));
        }
        combined.push(row);
    }

    if combined.is_empty() {
        return Ok(vec![]);
    }

    let (fields, field_types) = rows_to_columns(&combined);
    let size = combined.len();
    Ok(vec![DataChunk {
        fields,
        field_types,
        size,
        field_names: vec![],
        sel_vector: None,
    }])
}
