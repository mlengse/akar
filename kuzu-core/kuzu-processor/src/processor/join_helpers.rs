use kuzu_parser::ast::Expression;
use kuzu_common::vector::DataChunk;

pub fn derive_join_column_indices(
    join_keys: &[Expression],
    build_chunks: &[DataChunk],
    probe_chunks: &[DataChunk],
) -> (Vec<u32>, Vec<u32>) {
    let build_names: Vec<&str> = build_chunks
        .first()
        .map(|c| c.field_names.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    let probe_names: Vec<&str> = probe_chunks
        .first()
        .map(|c| c.field_names.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();

    let mut build_cols: Vec<u32> = Vec::new();
    let mut probe_cols: Vec<u32> = Vec::new();

    for key in join_keys {
        if let Expression::BinaryOp(
            kuzu_parser::ast::BinaryOp::Equal, left, right,
        ) = key
        {
            let left_prop = extract_join_prop(left);
            let right_prop = extract_join_prop(right);

            if let (Some(lp), Some(rp)) = (left_prop, right_prop) {
                let build_idx = build_names
                    .iter()
                    .position(|&n| n == lp)
                    .or_else(|| build_names.iter().position(|&n| n == rp))
                    .unwrap_or(0) as u32;
                let probe_idx = probe_names
                    .iter()
                    .position(|&n| n == rp)
                    .or_else(|| probe_names.iter().position(|&n| n == lp))
                    .unwrap_or(0) as u32;
                build_cols.push(build_idx);
                probe_cols.push(probe_idx);
            }
        }
    }

    if build_cols.is_empty() {
        (vec![0], vec![0])
    } else {
        (build_cols, probe_cols)
    }
}

fn extract_join_prop(expr: &Expression) -> Option<String> {
    match expr {
        Expression::PropertyAccess(obj, prop) => {
            if let Expression::Variable(var) = &**obj {
                Some(format!("{}.{}", var, prop))
            } else {
                Some(prop.clone())
            }
        }
        Expression::Variable(name) => Some(name.clone()),
        _ => None,
    }
}
