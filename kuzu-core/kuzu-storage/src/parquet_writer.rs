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
    macro_rules! append_or_null {
        ($builder_type:ty, $val_expr:expr) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<$builder_type>() {
                b.append_value($val_expr);
                return;
            }
        };
        (null $builder_type:ty) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<$builder_type>() {
                b.append_null();
                return;
            }
        };
    }

    match val {
        Value::Null => {
            append_or_null!(null BooleanBuilder);
            append_or_null!(null Int8Builder);
            append_or_null!(null Int16Builder);
            append_or_null!(null Int32Builder);
            append_or_null!(null Int64Builder);
            append_or_null!(null UInt8Builder);
            append_or_null!(null UInt16Builder);
            append_or_null!(null UInt32Builder);
            append_or_null!(null UInt64Builder);
            append_or_null!(null Float32Builder);
            append_or_null!(null Float64Builder);
            append_or_null!(null StringBuilder);
            append_or_null!(null Date32Builder);
            append_or_null!(null BinaryBuilder);
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
            if let Some(b) = builder.as_any_mut().downcast_mut::<StringBuilder>() { b.append_null(); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_parquet_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.parquet");
        let path_str = path.to_str().unwrap().to_string();

        let rows = vec![
            vec![Value::Int64(1), Value::String("Alice".into())],
            vec![Value::Int64(2), Value::String("Bob".into())],
            vec![Value::Int64(3), Value::String("Charlie".into())],
        ];
        let column_names = vec!["id".into(), "name".into()];

        write_parquet(&path_str, &rows, &column_names).unwrap();

        // Read it back using parquet reader
        let file = std::fs::File::open(&path).unwrap();
        let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();
        let batches: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();
        assert!(!batches.is_empty());
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 2);
    }

    #[test]
    fn test_write_parquet_with_nulls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_nulls.parquet");
        let path_str = path.to_str().unwrap().to_string();

        let rows = vec![
            vec![Value::Null, Value::String("null_id".into())],
            vec![Value::Int64(42), Value::Null],
        ];
        let column_names = vec!["id".into(), "name".into()];

        write_parquet(&path_str, &rows, &column_names).unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();
        let batches: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();
        assert!(!batches.is_empty());
        assert_eq!(batches[0].num_rows(), 2);
    }

    #[test]
    fn test_write_empty_parquet() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.parquet");
        let path_str = path.to_str().unwrap().to_string();

        let rows: Vec<Vec<Value>> = vec![];
        let column_names = vec!["col_a".into(), "col_b".into()];

        write_parquet(&path_str, &rows, &column_names).unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let builder = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap();
        assert_eq!(builder.schema().fields().len(), 2);
        // Empty parquet may produce 0 batches (no row groups); just verify it opens
        let _reader = builder.build().unwrap();
    }
}
