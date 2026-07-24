use akar_catalog::Catalog;
use akar_common::types::Value;
use std::sync::{Arc, Mutex};

/// Create a sequence resolution callback for use with the query processor.
///
/// The callback resolves `currval(seq_name)` and `nextval(seq_name)` by
/// looking up the named sequence in the catalog.
pub(crate) fn make_sequence_callback(
    catalog: Arc<Mutex<Catalog>>,
) -> Arc<dyn Fn(&str, bool) -> Result<Value, String> + Send + Sync> {
    Arc::new(move |seq_name: &str, is_nextval: bool| -> Result<Value, String> {
        let mut cat = catalog.lock().map_err(|e| format!("Catalog lock error: {e}"))?;
        if is_nextval {
            match cat.get_sequence_mut(seq_name) {
                Some(entry) => Ok(Value::Int64(entry.next_k_val(1))),
                None => Err(format!("Sequence '{}' not found", seq_name)),
            }
        } else {
            match cat.get_sequence(seq_name) {
                Some(entry) => Ok(Value::Int64(entry.curr_val())),
                None => Err(format!("Sequence '{}' not found", seq_name)),
            }
        }
    })
}

/// Register `currval` and `nextval` as scalar functions in the function registry.
///
/// These are the SQL-callable sequence functions (e.g. `SELECT nextval('my_seq')`).
/// Deduplicates the logic that was previously inlined in `Database::new`.
pub(crate) fn register_sequence_scalars(
    registry: &mut akar_function::FunctionRegistry,
    catalog: Arc<Mutex<Catalog>>,
) {
    use akar_function::registry::ScalarFunction;

    let curr_catalog = catalog.clone();
    registry.register_scalar(
        "currval",
        ScalarFunction::CustomScalar {
            name: "currval".into(),
            execute: Arc::new(move |args: &[Value]| -> Result<Value, String> {
                if args.is_empty() {
                    return Err("currval requires a sequence name argument".into());
                }
                let seq_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => return Err(format!("currval expects a string, got {:?}", other.logical_type())),
                };
                let cat = curr_catalog.lock().map_err(|e| format!("Catalog lock error: {e}"))?;
                let seq = cat
                    .get_sequence(&seq_name)
                    .ok_or_else(|| format!("Sequence '{}' not found", seq_name))?;
                Ok(Value::Int64(seq.curr_val()))
            }),
        },
    );

    let next_catalog = catalog;
    registry.register_scalar(
        "nextval",
        ScalarFunction::CustomScalar {
            name: "nextval".into(),
            execute: Arc::new(move |args: &[Value]| -> Result<Value, String> {
                if args.is_empty() {
                    return Err("nextval requires a sequence name argument".into());
                }
                let seq_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => return Err(format!("nextval expects a string, got {:?}", other.logical_type())),
                };
                let mut cat = next_catalog.lock().map_err(|e| format!("Catalog lock error: {e}"))?;
                let seq = cat
                    .get_sequence_mut(&seq_name)
                    .ok_or_else(|| format!("Sequence '{}' not found", seq_name))?;
                let result = seq.next_k_val(1);
                Ok(Value::Int64(result))
            }),
        },
    );
}

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
pub(crate) fn value_to_ast_constant(val: &Value) -> akar_parser::ast::Expression {
    match val {
        Value::Null => akar_parser::ast::Expression::Constant(akar_parser::ast::Constant::Null),
        Value::Bool(b) => akar_parser::ast::Expression::Constant(akar_parser::ast::Constant::Bool(*b)),
        Value::Int64(i) => akar_parser::ast::Expression::Constant(akar_parser::ast::Constant::Integer(*i)),
        Value::Double(f) => akar_parser::ast::Expression::Constant(akar_parser::ast::Constant::Float(*f)),
        Value::String(s) => akar_parser::ast::Expression::Constant(akar_parser::ast::Constant::String(s.clone())),
        _ => akar_parser::ast::Expression::Constant(akar_parser::ast::Constant::Null),
    }
}

/// Convert an AST Constant to a Value.
pub(crate) fn ast_constant_to_value(c: &akar_parser::ast::Constant) -> Value {
    match c {
        akar_parser::ast::Constant::Null => Value::Null,
        akar_parser::ast::Constant::Bool(b) => Value::Bool(*b),
        akar_parser::ast::Constant::Integer(i) => Value::Int64(*i),
        akar_parser::ast::Constant::Float(f) => Value::Double(*f),
        akar_parser::ast::Constant::String(s) => Value::String(s.clone()),
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
pub(crate) fn rows_to_datachunk(rows: Vec<Vec<Value>>, column_names: &[&str]) -> akar_common::vector::DataChunk {
    use akar_common::types::PhysicalTypeID;
    use akar_common::vector::ValueVector;

    if rows.is_empty() {
        let fields_legacy: Vec<ValueVector> = column_names
            .iter()
            .map(|_| ValueVector::new(PhysicalTypeID::String, 0))
            .collect();
        let fields = fields_legacy
            .iter()
            .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
            .collect::<Vec<_>>();
        let field_types = fields_legacy.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
        let mut chunk = akar_common::vector::DataChunk::new(fields, field_types);
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

    let arrow_cols = cols
        .iter()
        .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
        .collect::<Vec<_>>();
    let arrow_col_types = cols.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
    let mut chunk = akar_common::vector::DataChunk::new(arrow_cols, arrow_col_types);
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
