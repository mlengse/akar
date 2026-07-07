use kuzu_parser::ast::Expression;
use kuzu_common::vector::DataChunk;

pub fn resolve_projection_column_index(
    expr: &Expression,
    chunk: &DataChunk,
) -> Option<usize> {
    if let Expression::PropertyAccess(obj, prop) = expr {
        let col_name = if let Expression::Variable(var) = &**obj {
            format!("{}.{}", var, prop)
        } else {
            prop.clone()
        };
        if !chunk.field_names.is_empty() {
            if let Some(idx) = chunk
                .field_names
                .iter()
                .position(|n| n.ends_with(&format!(".{}", prop)) || n == &col_name || n == prop)
            {
                return Some(idx);
            }
        }
        if let Expression::Variable(var) = &**obj {
            if let Ok(idx) = var.parse::<usize>() {
                return Some(idx);
            }
        }
        return None;
    }
    if let Expression::Variable(name) = expr {
        if let Ok(idx) = name.parse::<usize>() {
            return Some(idx);
        }
        if !chunk.field_names.is_empty() {
            if let Some(idx) = chunk.field_names.iter().position(|n| n == name) {
                return Some(idx);
            }
        }
        return None;
    }
    None
}
