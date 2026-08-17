//! Parquet writer for the COPY TO command.
//!
//! Converts Akar `Value` rows to Arrow `RecordBatch` and writes to `.parquet` files.

use akar_common::error::StorageError;
use akar_common::types::{PhysicalTypeID, Value};
use arrow::array::*;
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use std::sync::Arc;

/// Write rows of Akar Values to a Parquet file.
///
/// Column names are inferred from the first row's length and named `col_0`, `col_1`, etc.
/// Types are inferred from the first non-null value in each column; when
/// `column_types` is provided, its declared physical types are used instead so
/// all-null columns (e.g. an untouched FLOAT[] embedding or BOOL flag) still
/// round-trip with the right Arrow type — the value-only inference would fall
/// back to Utf8 and the parquet reader would reject Utf8→List/Bool on import
/// (P53.37 repair_schema).
pub fn write_parquet(
    path: &str,
    rows: &[Vec<Value>],
    column_names: &[String],
    column_types: Option<&[PhysicalTypeID]>,
) -> Result<(), StorageError> {
    if rows.is_empty() {
        return write_empty_parquet(path, column_names);
    }

    let num_cols = column_names.len().max(rows[0].len());
    let mut arrow_cols: Vec<Box<dyn ArrayBuilder>> = Vec::with_capacity(num_cols);
    let mut arrow_types: Vec<ArrowDataType> = Vec::with_capacity(num_cols);

    // Determine types from first non-null values (or the declared type)
    for col_idx in 0..num_cols {
        let (dt, builder) = infer_column_type(rows, col_idx, num_cols, column_types.and_then(|t| t.get(col_idx)));
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

    let arrays: Vec<Arc<dyn Array>> = arrow_cols.into_iter().map(|mut b| b.finish()).collect();

    let batch = RecordBatch::try_new(schema, arrays)
        .map_err(|e| StorageError::Reader(format!("Failed to create RecordBatch: {e}")))?;

    write_batch(path, &batch)
}

fn write_empty_parquet(path: &str, column_names: &[String]) -> Result<(), StorageError> {
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
        .map_err(|e| StorageError::Reader(format!("Failed to create empty RecordBatch: {e}")))?;

    write_batch(path, &batch)
}

fn write_batch(path: &str, batch: &RecordBatch) -> Result<(), StorageError> {
    use parquet::arrow::ArrowWriter;
    use parquet::basic::Compression;
    use parquet::file::properties::WriterProperties;
    use std::fs::File;

    let file = File::create(path).map_err(|e| StorageError::Reader(format!("Cannot create file '{}': {}", path, e)))?;

    let props = WriterProperties::builder().set_compression(Compression::SNAPPY).build();

    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(props))
        .map_err(|e| StorageError::Reader(format!("Failed to create Parquet writer: {e}")))?;

    writer
        .write(batch)
        .map_err(|e| StorageError::Reader(format!("Failed to write batch: {e}")))?;

    writer
        .close()
        .map_err(|e| StorageError::Reader(format!("Failed to close Parquet writer: {e}")))?;

    Ok(())
}

fn infer_column_type(
    rows: &[Vec<Value>],
    col_idx: usize,
    _num_cols: usize,
    declared_type: Option<&PhysicalTypeID>,
) -> (ArrowDataType, Box<dyn ArrayBuilder>) {
    // A declared type wins over value inference: all-null columns must still
    // carry the schema's Arrow type (P53.37) — otherwise an untouched FLOAT[]
    // or BOOL column is written as Utf8 and the parquet reader rejects
    // Utf8→List/Bool on import, dropping every row.
    if let Some(dt) = declared_type {
        if *dt == PhysicalTypeID::List {
            return list_column_builder(rows, col_idx);
        }
        if let Some(builder) = builder_for_declared_type(*dt) {
            return builder;
        }
    }
    for row in rows {
        if let Some(val) = row.get(col_idx) {
            match val {
                Value::Null => continue,
                Value::Bool(_) => {
                    return (
                        ArrowDataType::Boolean,
                        Box::new(BooleanBuilder::with_capacity(rows.len())),
                    );
                }
                Value::Int8(_) => return (ArrowDataType::Int8, Box::new(Int8Builder::with_capacity(rows.len()))),
                Value::Int16(_) => return (ArrowDataType::Int16, Box::new(Int16Builder::with_capacity(rows.len()))),
                Value::Int32(_) => return (ArrowDataType::Int32, Box::new(Int32Builder::with_capacity(rows.len()))),
                Value::Int64(_) => return (ArrowDataType::Int64, Box::new(Int64Builder::with_capacity(rows.len()))),
                Value::UInt8(_) => return (ArrowDataType::UInt8, Box::new(UInt8Builder::with_capacity(rows.len()))),
                Value::UInt16(_) => {
                    return (
                        ArrowDataType::UInt16,
                        Box::new(UInt16Builder::with_capacity(rows.len())),
                    );
                }
                Value::UInt32(_) => {
                    return (
                        ArrowDataType::UInt32,
                        Box::new(UInt32Builder::with_capacity(rows.len())),
                    );
                }
                Value::UInt64(_) => {
                    return (
                        ArrowDataType::UInt64,
                        Box::new(UInt64Builder::with_capacity(rows.len())),
                    );
                }
                Value::Float(_) => {
                    return (
                        ArrowDataType::Float32,
                        Box::new(Float32Builder::with_capacity(rows.len())),
                    );
                }
                Value::Double(_) => {
                    return (
                        ArrowDataType::Float64,
                        Box::new(Float64Builder::with_capacity(rows.len())),
                    );
                }
                Value::String(_) => {
                    return (
                        ArrowDataType::Utf8,
                        Box::new(StringBuilder::with_capacity(rows.len(), rows.len() * 32)),
                    );
                }
                Value::Date(_) => {
                    return (
                        ArrowDataType::Date32,
                        Box::new(Date32Builder::with_capacity(rows.len())),
                    );
                }
                Value::Timestamp(_) => {
                    return (ArrowDataType::Int64, Box::new(Int64Builder::with_capacity(rows.len())));
                }
                Value::Interval(_) => return (ArrowDataType::Int64, Box::new(Int64Builder::with_capacity(rows.len()))),
                Value::Blob(_) => {
                    return (
                        ArrowDataType::Binary,
                        Box::new(BinaryBuilder::with_capacity(rows.len(), rows.len() * 32)),
                    );
                }
                Value::List(items) => {
                    // FLOAT[] / INT[] columns (e.g. embeddings) must round-trip as a
                    // real Arrow List, not a Utf8 fallback — the parquet reader
                    // rejects Utf8→List (P53.37 repair_schema round-trip).
                    let _ = items;
                    return list_column_builder(rows, col_idx);
                }
                _ => continue,
            }
        }
    }
    // Default to String if all null
    (
        ArrowDataType::Utf8,
        Box::new(StringBuilder::with_capacity(rows.len(), rows.len() * 32)),
    )
}

/// Build a List column builder; the inner element type is taken from the first
/// non-null list item, defaulting to Float64 for all-null lists (the FLOAT[]
/// embedding case — nulls carry no inner values so the exact type only matters
/// for the reader's schema compatibility check).
fn list_column_builder(rows: &[Vec<Value>], col_idx: usize) -> (ArrowDataType, Box<dyn ArrayBuilder>) {
    let inner = rows
        .iter()
        .filter_map(|r| r.get(col_idx))
        .filter_map(|v| match v {
            Value::List(items) => Some(items),
            _ => None,
        })
        .flatten()
        .find(|v| !matches!(v, Value::Null))
        .map(infer_scalar_type)
        .unwrap_or(ArrowDataType::Float64);
    let field = Arc::new(Field::new("item", inner.clone(), true));
    let dt = ArrowDataType::List(field);
    match inner {
        ArrowDataType::Float64 => (dt, Box::new(ListBuilder::new(Float64Builder::new()))),
        ArrowDataType::Float32 => (dt, Box::new(ListBuilder::new(Float32Builder::new()))),
        ArrowDataType::Int64 => (dt, Box::new(ListBuilder::new(Int64Builder::new()))),
        ArrowDataType::Int32 => (dt, Box::new(ListBuilder::new(Int32Builder::new()))),
        _ => (dt, Box::new(ListBuilder::new(StringBuilder::new()))),
    }
}

/// Map a declared physical type to an Arrow builder (all-null-safe). Returns
/// `None` for types without a direct scalar mapping (List/Struct/Interval).
fn builder_for_declared_type(dt: PhysicalTypeID) -> Option<(ArrowDataType, Box<dyn ArrayBuilder>)> {
    match dt {
        PhysicalTypeID::Bool => Some((ArrowDataType::Boolean, Box::new(BooleanBuilder::new()))),
        PhysicalTypeID::Int8 => Some((ArrowDataType::Int8, Box::new(Int8Builder::new()))),
        PhysicalTypeID::Int16 => Some((ArrowDataType::Int16, Box::new(Int16Builder::new()))),
        PhysicalTypeID::Int32 => Some((ArrowDataType::Int32, Box::new(Int32Builder::new()))),
        PhysicalTypeID::Int64 => Some((ArrowDataType::Int64, Box::new(Int64Builder::new()))),
        PhysicalTypeID::UInt8 => Some((ArrowDataType::UInt8, Box::new(UInt8Builder::new()))),
        PhysicalTypeID::UInt16 => Some((ArrowDataType::UInt16, Box::new(UInt16Builder::new()))),
        PhysicalTypeID::UInt32 => Some((ArrowDataType::UInt32, Box::new(UInt32Builder::new()))),
        PhysicalTypeID::UInt64 => Some((ArrowDataType::UInt64, Box::new(UInt64Builder::new()))),
        PhysicalTypeID::Float => Some((ArrowDataType::Float32, Box::new(Float32Builder::new()))),
        PhysicalTypeID::Double => Some((ArrowDataType::Float64, Box::new(Float64Builder::new()))),
        PhysicalTypeID::String => Some((ArrowDataType::Utf8, Box::new(StringBuilder::new()))),
        _ => None,
    }
}

/// Map a scalar `Value` to its Arrow data type (used for list element types).
fn infer_scalar_type(val: &Value) -> ArrowDataType {
    match val {
        Value::Bool(_) => ArrowDataType::Boolean,
        Value::Int8(_) => ArrowDataType::Int8,
        Value::Int16(_) => ArrowDataType::Int16,
        Value::Int32(_) => ArrowDataType::Int32,
        Value::Int64(_) => ArrowDataType::Int64,
        Value::UInt8(_) => ArrowDataType::UInt8,
        Value::UInt16(_) => ArrowDataType::UInt16,
        Value::UInt32(_) => ArrowDataType::UInt32,
        Value::UInt64(_) => ArrowDataType::UInt64,
        Value::Float(_) => ArrowDataType::Float32,
        Value::Double(_) => ArrowDataType::Float64,
        Value::String(_) => ArrowDataType::Utf8,
        Value::Date(_) => ArrowDataType::Date32,
        Value::Blob(_) => ArrowDataType::Binary,
        _ => ArrowDataType::Utf8,
    }
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
            // A NULL value in a list column must still advance the list offset
            // (as a null list) or the record batch column lengths diverge (P53.37).
            append_or_null!(null ListBuilder<Float64Builder>);
            append_or_null!(null ListBuilder<Float32Builder>);
            append_or_null!(null ListBuilder<Int64Builder>);
            append_or_null!(null ListBuilder<Int32Builder>);
            append_or_null!(null ListBuilder<StringBuilder>);
        }
        Value::Bool(v) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<BooleanBuilder>() {
                b.append_value(*v);
            }
        }
        Value::Int8(v) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<Int8Builder>() {
                b.append_value(*v);
            }
        }
        Value::Int16(v) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<Int16Builder>() {
                b.append_value(*v);
            }
        }
        Value::Int32(v) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<Int32Builder>() {
                b.append_value(*v);
            }
        }
        Value::Int64(v) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<Int64Builder>() {
                b.append_value(*v);
            }
        }
        Value::UInt8(v) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<UInt8Builder>() {
                b.append_value(*v);
            }
        }
        Value::UInt16(v) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<UInt16Builder>() {
                b.append_value(*v);
            }
        }
        Value::UInt32(v) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<UInt32Builder>() {
                b.append_value(*v);
            }
        }
        Value::UInt64(v) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<UInt64Builder>() {
                b.append_value(*v);
            }
        }
        Value::Float(v) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<Float32Builder>() {
                b.append_value(*v);
            }
        }
        Value::Double(v) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<Float64Builder>() {
                b.append_value(*v);
            }
        }
        Value::String(s) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<StringBuilder>() {
                b.append_value(s.as_str());
            }
        }
        Value::Date(d) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<Date32Builder>() {
                b.append_value(d.days_since_epoch());
            }
        }
        Value::Timestamp(ts) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<Int64Builder>() {
                b.append_value(ts.micros_since_epoch());
            }
        }
        Value::Interval(_) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<Int64Builder>() {
                b.append_null();
            }
        }
        Value::Blob(data) => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<BinaryBuilder>() {
                b.append_value(data.as_slice());
            }
        }
        Value::List(items) => {
            // Append a typed list (FLOAT[] embeddings etc.) so the value
            // round-trips through parquet as a real list, not Utf8 (P53.37).
            macro_rules! append_list {
                ($builder_type:ty, $arm:pat => $conv:expr) => {
                    if let Some(b) = builder.as_any_mut().downcast_mut::<$builder_type>() {
                        for v in items {
                            match v {
                                Value::Null => b.values().append_null(),
                                $arm => b.values().append_value($conv),
                                _ => b.values().append_null(),
                            }
                        }
                        b.append(true);
                        return;
                    }
                };
            }
            append_list!(ListBuilder<Float64Builder>, Value::Double(d) => *d);
            append_list!(ListBuilder<Float64Builder>, Value::Float(f) => *f as f64);
            append_list!(ListBuilder<Float32Builder>, Value::Float(f) => *f);
            append_list!(ListBuilder<Float32Builder>, Value::Double(d) => *d as f32);
            append_list!(ListBuilder<Int64Builder>, Value::Int64(i) => *i);
            append_list!(ListBuilder<Int32Builder>, Value::Int32(i) => *i);
            append_list!(ListBuilder<StringBuilder>, Value::String(s) => s.as_str());
            // Unknown inner type — append a null list.
            if let Some(b) = builder.as_any_mut().downcast_mut::<ListBuilder<StringBuilder>>() {
                b.append(false);
            }
        }
        _ => {
            if let Some(b) = builder.as_any_mut().downcast_mut::<StringBuilder>() {
                b.append_null();
            }
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

        write_parquet(&path_str, &rows, &column_names, None).unwrap();

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

        write_parquet(&path_str, &rows, &column_names, None).unwrap();

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

        write_parquet(&path_str, &rows, &column_names, None).unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let builder = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        assert_eq!(builder.schema().fields().len(), 2);
        // Empty parquet may produce 0 batches (no row groups); just verify it opens
        let _reader = builder.build().unwrap();
    }
}
