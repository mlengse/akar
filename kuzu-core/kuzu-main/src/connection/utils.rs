use kuzu_common::types::Value;

/// Convert a Value to its string representation for CSV output.
pub(crate) fn value_to_csv_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Int64(i) => i.to_string(),
        Value::Int32(i) => i.to_string(),
        Value::Double(f) => f.to_string(),
        Value::String(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

/// Convert a Value to an AST Expression (Constant).
pub(crate) fn value_to_ast_constant(val: &Value) -> kuzu_parser::ast::Expression {
    match val {
        Value::Null => kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::Null),
        Value::Bool(b) => kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::Bool(*b)),
        Value::Int64(i) => kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::Integer(*i)),
        Value::Double(f) => kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::Float(*f)),
        Value::String(s) => kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::String(s.clone())),
        _ => kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::Null),
    }
}

/// Convert an AST Constant to a Value.
pub(crate) fn ast_constant_to_value(c: &kuzu_parser::ast::Constant) -> Value {
    match c {
        kuzu_parser::ast::Constant::Null => Value::Null,
        kuzu_parser::ast::Constant::Bool(b) => Value::Bool(*b),
        kuzu_parser::ast::Constant::Integer(i) => Value::Int64(*i),
        kuzu_parser::ast::Constant::Float(f) => Value::Double(*f),
        kuzu_parser::ast::Constant::String(s) => Value::String(s.clone()),
    }
}


/// Convert a Value to its string representation for hash index key lookup.
pub(crate) fn pk_value_to_string(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int64(i) => i.to_string(),
        Value::Int32(i) => i.to_string(),
        Value::Double(f) => f.to_string(),
        Value::String(s) => s.clone(),
        Value::Date(d) => format!("Date({})", d.0),
        Value::Timestamp(ts) => format!("Timestamp({})", ts.0),
        other => format!("{other:?}"),
    }
}

/// Convert `Vec<Vec<Value>>` rows into a `DataChunk` with named columns.
#[allow(dead_code)]
pub(crate) fn rows_to_datachunk(
    rows: Vec<Vec<Value>>,
    column_names: &[&str],
) -> kuzu_common::vector::DataChunk {
    use kuzu_common::types::PhysicalTypeID;
    use kuzu_common::vector::ValueVector;

    if rows.is_empty() {
        let fields = column_names
            .iter()
            .map(|_| ValueVector::new(PhysicalTypeID::String, 0))
            .collect();
        let mut chunk = kuzu_common::vector::DataChunk::new(fields);
        chunk.field_names = column_names.iter().map(|s| s.to_string()).collect();
        return chunk;
    }
    let num_columns = rows[0].len();
    let num_rows = rows.len();
    let mut cols: Vec<ValueVector> = (0..num_columns)
        .map(|_| ValueVector::new(PhysicalTypeID::String, num_rows))
        .collect();

    for row in &rows {
        for (col_idx, v) in row.iter().enumerate() {
            let display = match v {
                Value::Null => "NULL".to_string(),
                Value::String(s) => s.clone(),
                Value::Int64(i) => i.to_string(),
                Value::Int32(i) => i.to_string(),
                Value::Double(f) => f.to_string(),
                Value::Bool(b) => b.to_string(),
                other => format!("{other:?}"),
            };
            cols[col_idx].push_string(&display);
        }
    }

    let mut chunk = kuzu_common::vector::DataChunk::new(cols);
    chunk.field_names = column_names.iter().map(|s| s.to_string()).collect();
    chunk.size = num_rows;
    chunk
}

/// Format a byte count into a human-readable string (e.g., "1.2 MB").
pub(crate) fn format_storage_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
