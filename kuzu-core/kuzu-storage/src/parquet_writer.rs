//! Parquet writer for the COPY TO command.
//!
//! Converts Kuzu `Value` rows to Arrow `RecordBatch` and writes to `.parquet` files.

use arrow::array::*;
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use kuzu_common::types::Value;
use std::sync::Arc;

/// Write rows of Kuzu Values to a Parquet file.
///
/// Column names are inferred from the first row's length and named `col_0`, `col_1`, etc.
/// Types are inferred from the first non-null value in each column.
pub fn write_parquet(path: &str, rows: &[Vec<Value>], column_names: &[String]) -> Result<(), String> {
    if rows.is_empty() {
        return write_empty_parquet(path, column_names);
    }

    let num_cols = column_names.len().max(rows[0].len());
    let mut arrow_cols: Vec<Box<dyn ArrayBuilder>> = Vec::with_capacity(num_cols);
    let mut arrow_types: Vec<ArrowDataType> = Vec::with_capacity(num_cols);

    // Determine types from first non-null values
    for col_idx in 0..num_cols {
        let (dt, builder) = infer_column_type(rows, col_idx, num_cols);
        arrow_types.push(dt);
        arrow_cols.push(builder);
    }

    // Append all rows
    for row in rows {
        for col_idx in 0..num_cols {
            let val = row.get(col_idx).unwrap_or(&Value::Null);
            append_value_to_builder(&mut arrow_cols[col_idx], val);
        }
    }

    // Build arrays and record batch
    let schema_fields: Vec<Field> = column_names
        .iter()
        .enumerate()
        .map(|(i, name)| Field::new(name, arrow_types[i].clone(), true))
        .collect();
    let schema = Arc::new(Schema::new(schema_fields));

    let arrays: Vec<Arc<dyn Array>> = arrow_cols
        .into_iter()
        .map(|mut b| b.finish())
        .collect();

    let batch = RecordBatch::try_new(schema, arrays)
        .map_err(|e| format!("Failed to create RecordBatch: {e}"))?;

    write_batch(path, &batch)
}

fn write_empty_parquet(path: &str, column_names: &[String]) -> Result<(), String> {
    let fields: Vec<Field> = column_names
        .iter()
        .map(|n| Field::new(n, ArrowDataType::Utf8, true))
        .collect();
    let schema = Arc::new(Schema::new(fields));

    let arrays: Vec<Arc<dyn Array>> = column_names
        .iter()
        .map(|_| Arc::new(StringArray::from(Vec::<&str>::new())) as Arc<dyn Array>)
        .collect();

    let batch = RecordBatch::try_new(schema, arrays)
        .map_err(|e| format!("Failed to create empty RecordBatch: {e}"))?;

    write_batch(path, &batch)
}

fn write_batch(path: &str, batch: &RecordBatch) -> Result<(), String> {
    use parquet::arrow::ArrowWriter;
    use parquet::basic::Compression;
    use parquet::file::properties::WriterProperties;
    use std::fs::File;

    let file = File::create(path)
        .map_err(|e| format!("Cannot create file '{}': {}", path, e))?;

    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();

    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(props))
        .map_err(|e| format!("Failed to create Parquet writer: {e}"))?;

    writer
        .write(batch)
        .map_err(|e| format!("Failed to write batch: {e}"))?;

    writer
        .close()
        .map_err(|e| format!("Failed to close Parquet writer: {e}"))?;

    Ok(())
}

fn infer_column_type(rows: &[Vec<Value>], col_idx: usize, _num_cols: usize) -> (ArrowDataType, Box<dyn ArrayBuilder>) {
    for row in rows {
        if let Some(val) = row.get(col_idx) {
            match val {
                Value::Null => continue,
                Value::Bool(_) => return (ArrowDataType::Boolean, Box::new(BooleanBuilder::with_capacity(rows.len()))),
                Value::Int8(_) => return (ArrowDataType::Int8, Box::new(Int8Builder::with_capacity(rows.len()))),
                Value::Int16(_) => return (ArrowDataType::Int16, Box::new(Int16Builder::with_capacity(rows.len()))),
                Value::Int32(_) => return (ArrowDataType::Int32, Box::new(Int32Builder::with_capacity(rows.len()))),
                Value::Int64(_) => return (ArrowDataType::Int64, Box::new(Int64Builder::with_capacity(rows.len()))),
                Value::UInt8(_) => return (ArrowDataType::UInt8, Box::new(UInt8Builder::with_capacity(rows.len()))),
                Value::UInt16(_) => return (ArrowDataType::UInt16, Box::new(UInt16Builder::with_capacity(rows.len()))),
                Value::UInt32(_) => return (ArrowDataType::UInt32, Box::new(UInt32Builder::with_capacity(rows.len()))),
                Value::UInt64(_) => return (ArrowDataType::UInt64, Box::new(UInt64Builder::with_capacity(rows.len()))),
                Value::Float(_) => return (ArrowDataType::Float32, Box::new(Float32Builder::with_capacity(rows.len()))),
                Value::Double(_) => return (ArrowDataType::Float64, Box::new(Float64Builder::with_capacity(rows.len()))),
                Value::String(_) => return (ArrowDataType::Utf8, Box::new(StringBuilder::with_capacity(rows.len(), rows.len() * 32))),
                Value::Date(_) => return (ArrowDataType::Date32, Box::new(Date32Builder::with_capacity(rows.len()))),
                Value::Timestamp(_) => return (ArrowDataType::Int64, Box::new(Int64Builder::with_capacity(rows.len()))),
                Value::Interval(_) => return (ArrowDataType::Int64, Box::new(Int64Builder::with_capacity(rows.len()))),
                Value::Blob(_) => return (ArrowDataType::Binary, Box::new(BinaryBuilder::with_capacity(rows.len(), rows.len() * 32))),
                _ => continue,
            }
        }
    }
    // Default to String if all null
    (ArrowDataType::Utf8, Box::new(StringBuilder::with_capacity(rows.len(), rows.len() * 32)))
}

fn append_value_to_builder(builder: &mut Box<dyn ArrayBuilder>, val: &Value) {
    match val {
        Value::Null => {
            match builder.as_any_mut().downcast_mut::<BooleanBuilder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<Int8Builder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<Int16Builder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<Int32Builder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<Int64Builder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<UInt8Builder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<UInt16Builder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<UInt32Builder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<UInt64Builder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<Float32Builder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<Float64Builder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<StringBuilder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<Date32Builder>() { Some(b) => { b.append_null(); return; } _ => {} }
            match builder.as_any_mut().downcast_mut::<BinaryBuilder>() { Some(b) => { b.append_null(); return; } _ => {} }
        }
        Value::Bool(v) => { if let Some(b) = builder.as_any_mut().downcast_mut::<BooleanBuilder>() { b.append_value(*v); } }
        Value::Int8(v) => { if let Some(b) = builder.as_any_mut().downcast_mut::<Int8Builder>() { b.append_value(*v); } }
        Value::Int16(v) => { if let Some(b) = builder.as_any_mut().downcast_mut::<Int16Builder>() { b.append_value(*v); } }
        Value::Int32(v) => { if let Some(b) = builder.as_any_mut().downcast_mut::<Int32Builder>() { b.append_value(*v); } }
        Value::Int64(v) => { if let Some(b) = builder.as_any_mut().downcast_mut::<Int64Builder>() { b.append_value(*v); } }
        Value::UInt8(v) => { if let Some(b) = builder.as_any_mut().downcast_mut::<UInt8Builder>() { b.append_value(*v); } }
        Value::UInt16(v) => { if let Some(b) = builder.as_any_mut().downcast_mut::<UInt16Builder>() { b.append_value(*v); } }
        Value::UInt32(v) => { if let Some(b) = builder.as_any_mut().downcast_mut::<UInt32Builder>() { b.append_value(*v); } }
        Value::UInt64(v) => { if let Some(b) = builder.as_any_mut().downcast_mut::<UInt64Builder>() { b.append_value(*v); } }
        Value::Float(v) => { if let Some(b) = builder.as_any_mut().downcast_mut::<Float32Builder>() { b.append_value(*v); } }
        Value::Double(v) => { if let Some(b) = builder.as_any_mut().downcast_mut::<Float64Builder>() { b.append_value(*v); } }
        Value::String(s) => { if let Some(b) = builder.as_any_mut().downcast_mut::<StringBuilder>() { b.append_value(s.as_str()); } }
        Value::Date(d) => { if let Some(b) = builder.as_any_mut().downcast_mut::<Date32Builder>() { b.append_value(d.days_since_epoch()); } }
        Value::Timestamp(ts) => { if let Some(b) = builder.as_any_mut().downcast_mut::<Int64Builder>() { b.append_value(ts.micros_since_epoch()); } }
        Value::Interval(_) => { if let Some(b) = builder.as_any_mut().downcast_mut::<Int64Builder>() { b.append_null(); } }
        Value::Blob(data) => { if let Some(b) = builder.as_any_mut().downcast_mut::<BinaryBuilder>() { b.append_value(data.as_slice()); } }
        _ => {
            // Other types → null
            if let Some(b) = builder.as_any_mut().downcast_mut::<StringBuilder>() { b.append_null(); }
        }
    }
}
