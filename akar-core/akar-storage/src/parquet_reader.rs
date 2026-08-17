//! Parquet reader for the COPY FROM command.
//!
//! Reads Parquet files and converts Arrow columnar data to Akar `Value`
//! types based on a provided schema (column names + types from the catalog).

use akar_catalog::CatalogColumn;
use akar_common::types::{Date, Interval, LogicalTypeID, Timestamp, Value};
use arrow::array::*;
use arrow::datatypes::{DataType as ArrowDataType, TimeUnit};
use arrow::record_batch::RecordBatch;

/// Error type for Parquet reader operations.
#[derive(Debug)]
pub enum ParquetReaderError {
    /// I/O or format error from the Parquet/Arrow layer.
    ParquetError(String),
    /// Schema mismatch: a column name from the catalog was not found in the file.
    ColumnNotFound {
        column_name: String,
        available: Vec<String>,
    },
    /// Type mismatch: column exists but has incompatible type.
    TypeMismatch {
        column_name: String,
        arrow_type: String,
        expected_type: String,
    },
    /// Row-level conversion error.
    ConversionError {
        column_name: String,
        row: usize,
        message: String,
    },
}

impl std::fmt::Display for ParquetReaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParquetReaderError::ParquetError(e) => write!(f, "Parquet error: {e}"),
            ParquetReaderError::ColumnNotFound { column_name, available } => write!(
                f,
                "Column '{}' not found in Parquet file. Available columns: [{}]",
                column_name,
                available.join(", ")
            ),
            ParquetReaderError::TypeMismatch {
                column_name,
                arrow_type,
                expected_type,
            } => write!(
                f,
                "Type mismatch for column '{}': Parquet has {arrow_type}, expected {expected_type}",
                column_name
            ),
            ParquetReaderError::ConversionError {
                column_name,
                row,
                message,
            } => write!(
                f,
                "Conversion error for column '{}' at row {row}: {message}",
                column_name
            ),
        }
    }
}

impl std::error::Error for ParquetReaderError {}

impl From<parquet::errors::ParquetError> for ParquetReaderError {
    fn from(e: parquet::errors::ParquetError) -> Self {
        ParquetReaderError::ParquetError(e.to_string())
    }
}

impl From<arrow::error::ArrowError> for ParquetReaderError {
    fn from(e: arrow::error::ArrowError) -> Self {
        ParquetReaderError::ParquetError(e.to_string())
    }
}

/// Result alias for Parquet reader operations.
pub type ParquetResult<T> = Result<T, ParquetReaderError>;

/// Read a Parquet file and convert columns to Akar `Value`s matching the schema.
///
/// The `columns` parameter defines the target schema. Columns are matched by
/// name against the Parquet file's schema; order in the result follows the
/// `columns` slice order.
///
/// # Arguments
///
/// * `path` - Path to the `.parquet` file.
/// * `columns` - Target column schema (name + type). Only these columns are
///   read from the file.
///
/// # Returns
///
/// A vector of rows, where each row is a `Vec<Value>` with length equal to
/// `columns.len()`.
pub fn read_parquet(
    path: &str,
    vfs: &akar_common::file_system::VirtualFileSystemRegistry,
    columns: &[CatalogColumn],
) -> ParquetResult<Vec<Vec<Value>>> {
    let mut file = vfs
        .open_read(path)
        .map_err(|e| ParquetReaderError::ParquetError(format!("Cannot open file: {e}")))?;

    // For now, read the entire file into memory to pass it to ParquetRecordBatchReaderBuilder
    // as bytes::Bytes, since Box<dyn FileRead> doesn't natively implement ChunkReader.
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|e| ParquetReaderError::ParquetError(format!("Read error: {e}")))?;
    let bytes = bytes::Bytes::from(buffer);

    let builder = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(bytes)?;
    let schema = builder.schema().clone();
    let reader = builder.build()?;

    // Build a map of column name → Arrow field for quick lookup
    let arrow_fields: Vec<(String, &ArrowDataType)> = schema
        .fields()
        .iter()
        .map(|f| (f.name().clone(), f.data_type()))
        .collect();

    let available_names: Vec<String> = arrow_fields.iter().map(|(n, _)| n.clone()).collect();

    // For each catalog column, find its index in the Arrow schema and validate types
    let mut col_indices: Vec<usize> = Vec::with_capacity(columns.len());
    for col in columns {
        let pos = arrow_fields
            .iter()
            .position(|(name, _)| name.eq_ignore_ascii_case(&col.name));
        match pos {
            Some(idx) => {
                let (_, arrow_type) = &arrow_fields[idx];
                validate_type_compatibility(arrow_type, col.logical_type).map_err(|_| {
                    ParquetReaderError::TypeMismatch {
                        column_name: col.name.clone(),
                        arrow_type: format!("{arrow_type:?}"),
                        expected_type: format!("{:?}", col.logical_type),
                    }
                })?;
                col_indices.push(idx);
            }
            None => {
                return Err(ParquetReaderError::ColumnNotFound {
                    column_name: col.name.clone(),
                    available: available_names.clone(),
                });
            }
        }
    }

    // Read all row groups and convert
    let mut results: Vec<Vec<Value>> = Vec::new();

    for batch_result in reader {
        let batch: RecordBatch = batch_result?;
        let num_rows = batch.num_rows();

        // Pre-allocate result rows
        if results.is_empty() {
            results.reserve(num_rows * 4); // rough initial estimate
        }

        for row_idx in 0..num_rows {
            let mut row = Vec::with_capacity(columns.len());
            for (catalog_idx, &arrow_col_idx) in col_indices.iter().enumerate() {
                let col = &columns[catalog_idx];
                let array = batch.column(arrow_col_idx);
                let value = arrow_array_to_value(array, row_idx, &col.name, col.logical_type, results.len())?;
                row.push(value);
            }
            results.push(row);
        }
    }

    Ok(results)
}

/// A streaming parquet reader that yields batches of rows on demand.
///
/// Unlike `read_parquet`, this avoids materializing the entire file into
/// `Vec<Vec<Value>>` at once. Each call to `next()` reads and converts one
/// Arrow `RecordBatch`.
pub struct ParquetStreamReader {
    reader: parquet::arrow::arrow_reader::ParquetRecordBatchReader,
    col_indices: Vec<usize>,
    columns: Vec<CatalogColumn>,
}

impl Iterator for ParquetStreamReader {
    type Item = ParquetResult<Vec<Vec<Value>>>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.reader.next() {
            Some(Ok(batch)) => {
                let num_rows = batch.num_rows();
                let mut rows = Vec::with_capacity(num_rows);
                for row_idx in 0..num_rows {
                    let mut row = Vec::with_capacity(self.columns.len());
                    for (catalog_idx, &arrow_col_idx) in self.col_indices.iter().enumerate() {
                        let col = &self.columns[catalog_idx];
                        let array = batch.column(arrow_col_idx);
                        let value = match arrow_array_to_value(array, row_idx, &col.name, col.logical_type, rows.len())
                        {
                            Ok(v) => v,
                            Err(e) => return Some(Err(e)),
                        };
                        row.push(value);
                    }
                    rows.push(row);
                }
                Some(Ok(rows))
            }
            Some(Err(e)) => Some(Err(ParquetReaderError::ParquetError(e.to_string()))),
            None => None,
        }
    }
}

/// Open a Parquet file and return a streaming reader that yields row batches.
///
/// The file is read into memory (required by the Parquet format for footer
/// access), but rows are converted to `Vec<Value>` per batch rather than
/// materializing the entire dataset at once.
pub fn stream_parquet(
    path: &str,
    vfs: &akar_common::file_system::VirtualFileSystemRegistry,
    columns: &[CatalogColumn],
) -> ParquetResult<ParquetStreamReader> {
    let mut file = vfs
        .open_read(path)
        .map_err(|e| ParquetReaderError::ParquetError(format!("Cannot open file: {e}")))?;

    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|e| ParquetReaderError::ParquetError(format!("Read error: {e}")))?;
    let bytes = bytes::Bytes::from(buffer);

    let builder = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(bytes)?;
    let schema = builder.schema().clone();
    let reader = builder.build()?;

    let arrow_fields: Vec<(String, &arrow::datatypes::DataType)> = schema
        .fields()
        .iter()
        .map(|f| (f.name().clone(), f.data_type()))
        .collect();

    let available_names: Vec<String> = arrow_fields.iter().map(|(n, _)| n.clone()).collect();

    let mut col_indices: Vec<usize> = Vec::with_capacity(columns.len());
    for col in columns {
        let pos = arrow_fields
            .iter()
            .position(|(name, _)| name.eq_ignore_ascii_case(&col.name));
        match pos {
            Some(idx) => {
                let (_, arrow_type) = &arrow_fields[idx];
                validate_type_compatibility(arrow_type, col.logical_type).map_err(|_| {
                    ParquetReaderError::TypeMismatch {
                        column_name: col.name.clone(),
                        arrow_type: format!("{arrow_type:?}"),
                        expected_type: format!("{:?}", col.logical_type),
                    }
                })?;
                col_indices.push(idx);
            }
            None => {
                return Err(ParquetReaderError::ColumnNotFound {
                    column_name: col.name.clone(),
                    available: available_names.clone(),
                });
            }
        }
    }

    Ok(ParquetStreamReader {
        reader,
        col_indices,
        columns: columns.to_vec(),
    })
}

// ─── Type validation ────────────────────────────────────────────────────────────

/// Check that an Arrow `DataType` is compatible with the expected Akar `LogicalTypeID`.
fn validate_type_compatibility(arrow_type: &ArrowDataType, expected: LogicalTypeID) -> Result<(), ()> {
    match (arrow_type, expected) {
        (ArrowDataType::Boolean, LogicalTypeID::Bool) => Ok(()),
        (ArrowDataType::Int8, LogicalTypeID::Int8) => Ok(()),
        (ArrowDataType::Int16, LogicalTypeID::Int16) => Ok(()),
        (ArrowDataType::Int32, LogicalTypeID::Int32) => Ok(()),
        (ArrowDataType::Int64, LogicalTypeID::Int64 | LogicalTypeID::Serial) => Ok(()),
        (ArrowDataType::UInt8, LogicalTypeID::UInt8) => Ok(()),
        (ArrowDataType::UInt16, LogicalTypeID::UInt16) => Ok(()),
        (ArrowDataType::UInt32, LogicalTypeID::UInt32) => Ok(()),
        (ArrowDataType::UInt64, LogicalTypeID::UInt64) => Ok(()),
        (ArrowDataType::Float32, LogicalTypeID::Float) => Ok(()),
        (ArrowDataType::Float64, LogicalTypeID::Double) => Ok(()),
        (ArrowDataType::Utf8 | ArrowDataType::LargeUtf8, LogicalTypeID::String) => Ok(()),
        (ArrowDataType::Binary | ArrowDataType::LargeBinary, LogicalTypeID::Blob) => Ok(()),
        (ArrowDataType::Date32 | ArrowDataType::Date64, LogicalTypeID::Date) => Ok(()),
        (ArrowDataType::Timestamp(TimeUnit::Second, _), LogicalTypeID::TimestampSec) => Ok(()),
        (ArrowDataType::Timestamp(TimeUnit::Millisecond, _), LogicalTypeID::TimestampMs) => Ok(()),
        (ArrowDataType::Timestamp(TimeUnit::Microsecond, _), LogicalTypeID::Timestamp) => Ok(()),
        (ArrowDataType::Timestamp(TimeUnit::Nanosecond, _), LogicalTypeID::TimestampNs) => Ok(()),
        (ArrowDataType::Duration(_), LogicalTypeID::Interval) => Ok(()),
        (ArrowDataType::List(_), LogicalTypeID::List) => Ok(()),
        (ArrowDataType::Struct(_), LogicalTypeID::Struct) => Ok(()),
        (ArrowDataType::Map(_, _), LogicalTypeID::Map) => Ok(()),
        // Allow numeric widening: smaller ints can be read as larger targets
        (ArrowDataType::Int8, LogicalTypeID::Int64 | LogicalTypeID::Int32 | LogicalTypeID::Int16) => Ok(()),
        (ArrowDataType::Int16, LogicalTypeID::Int64 | LogicalTypeID::Int32) => Ok(()),
        (ArrowDataType::Int32, LogicalTypeID::Int64) => Ok(()),
        (ArrowDataType::UInt8, LogicalTypeID::UInt64 | LogicalTypeID::UInt32 | LogicalTypeID::UInt16) => Ok(()),
        (ArrowDataType::UInt16, LogicalTypeID::UInt64 | LogicalTypeID::UInt32) => Ok(()),
        (ArrowDataType::UInt32, LogicalTypeID::UInt64) => Ok(()),
        (ArrowDataType::Float32, LogicalTypeID::Double) => Ok(()),
        // Fallback: accept String target for any Arrow type (coercion done during conversion)
        (_, LogicalTypeID::String) => Ok(()),
        _ => Err(()),
    }
}

// ─── Array → Value conversion ───────────────────────────────────────────────────

/// Convert a value from an Arrow array at the given row index to a Akar `Value`.
fn arrow_array_to_value(
    array: &dyn Array,
    row: usize,
    column_name: &str,
    target_type: LogicalTypeID,
    _global_row: usize,
) -> ParquetResult<Value> {
    // Handle nulls
    if array.is_null(row) {
        return Ok(Value::Null);
    }

    match target_type {
        LogicalTypeID::Bool => {
            let arr = downcast::<BooleanArray>(array, column_name)?;
            Ok(Value::Bool(arr.value(row)))
        }
        LogicalTypeID::Int64 | LogicalTypeID::Serial => {
            let val = cast_int_to_i64(array, row, column_name)?;
            Ok(Value::Int64(val))
        }
        LogicalTypeID::Int32 => {
            let val = cast_int_to_i64(array, row, column_name)?;
            Ok(Value::Int32(val as i32))
        }
        LogicalTypeID::Int16 => {
            let val = cast_int_to_i64(array, row, column_name)?;
            Ok(Value::Int16(val as i16))
        }
        LogicalTypeID::Int8 => {
            let val = cast_int_to_i64(array, row, column_name)?;
            Ok(Value::Int8(val as i8))
        }
        LogicalTypeID::UInt64 => {
            let val = cast_uint_to_u64(array, row, column_name)?;
            Ok(Value::UInt64(val))
        }
        LogicalTypeID::UInt32 => {
            let val = cast_uint_to_u64(array, row, column_name)?;
            Ok(Value::UInt32(val as u32))
        }
        LogicalTypeID::UInt16 => {
            let val = cast_uint_to_u64(array, row, column_name)?;
            Ok(Value::UInt16(val as u16))
        }
        LogicalTypeID::UInt8 => {
            let val = cast_uint_to_u64(array, row, column_name)?;
            Ok(Value::UInt8(val as u8))
        }
        LogicalTypeID::Double => {
            let val = cast_to_f64(array, row, column_name)?;
            Ok(Value::Double(val))
        }
        LogicalTypeID::Float => {
            let val = cast_to_f64(array, row, column_name)?;
            Ok(Value::Float(val as f32))
        }
        LogicalTypeID::String => {
            let s = array_to_string(array, row, column_name)?;
            Ok(Value::String(s))
        }
        LogicalTypeID::Blob => {
            let arr = downcast::<BinaryArray>(array, column_name)?;
            Ok(Value::Blob(arr.value(row).to_vec()))
        }
        LogicalTypeID::Date => {
            let val = cast_date_to_days(array, row, column_name)?;
            Ok(Value::Date(Date::from_days_since_epoch(val)))
        }
        LogicalTypeID::Timestamp => {
            let micros = cast_timestamp_to_micros(array, row, column_name)?;
            Ok(Value::Timestamp(Timestamp::from_micros_since_epoch(micros)))
        }
        LogicalTypeID::TimestampMs => {
            let micros = cast_timestamp_to_micros(array, row, column_name)?;
            Ok(Value::TimestampMs(Timestamp::from_micros_since_epoch(micros)))
        }
        LogicalTypeID::TimestampSec => {
            let micros = cast_timestamp_to_micros(array, row, column_name)?;
            Ok(Value::TimestampSec(Timestamp(micros / 1_000_000)))
        }
        LogicalTypeID::TimestampNs => {
            let micros = cast_timestamp_to_micros(array, row, column_name)?;
            Ok(Value::TimestampNs(Timestamp(micros * 1000)))
        }
        LogicalTypeID::TimestampTz => {
            let micros = cast_timestamp_to_micros(array, row, column_name)?;
            Ok(Value::TimestampTz(akar_common::types::TimestampTZ(micros)))
        }
        LogicalTypeID::Interval => {
            let arr = downcast::<DurationMicrosecondArray>(array, column_name)?;
            Ok(Value::Interval(Interval::new(0, 0, arr.value(row))))
        }
        LogicalTypeID::List => {
            let vals = array_list_to_values(array, row, column_name)?;
            Ok(Value::List(vals))
        }
        LogicalTypeID::Struct => {
            let vals = array_struct_to_values(array, row, column_name)?;
            Ok(Value::Struct(vals))
        }
        LogicalTypeID::Map => {
            let vals = array_map_to_values(array, row, column_name)?;
            Ok(Value::Map(vals))
        }
        // Fallback: string representation
        _ => {
            let s = array_to_string(array, row, column_name)?;
            Ok(Value::String(s))
        }
    }
}

// ─── Downcast helper ────────────────────────────────────────────────────────────

fn downcast<'a, T: Array + 'static>(array: &'a dyn Array, column_name: &str) -> ParquetResult<&'a T> {
    array
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| ParquetReaderError::ConversionError {
            column_name: column_name.to_string(),
            row: 0,
            message: format!(
                "expected array type {} but got {:?}",
                std::any::type_name::<T>(),
                array.data_type()
            ),
        })
}

// ─── Numeric casting ────────────────────────────────────────────────────────────

/// Extract an i64 from any integer Arrow array (with widening).
fn cast_int_to_i64(array: &dyn Array, row: usize, column_name: &str) -> ParquetResult<i64> {
    if let Some(arr) = array.as_any().downcast_ref::<Int8Array>() {
        return Ok(arr.value(row) as i64);
    }
    if let Some(arr) = array.as_any().downcast_ref::<Int16Array>() {
        return Ok(arr.value(row) as i64);
    }
    if let Some(arr) = array.as_any().downcast_ref::<Int32Array>() {
        return Ok(arr.value(row) as i64);
    }
    if let Some(arr) = array.as_any().downcast_ref::<Int64Array>() {
        return Ok(arr.value(row));
    }
    Err(ParquetReaderError::ConversionError {
        column_name: column_name.to_string(),
        row,
        message: format!("cannot cast {:?} to Int64", array.data_type()),
    })
}

/// Extract a u64 from any unsigned integer Arrow array.
fn cast_uint_to_u64(array: &dyn Array, row: usize, column_name: &str) -> ParquetResult<u64> {
    if let Some(arr) = array.as_any().downcast_ref::<UInt8Array>() {
        return Ok(arr.value(row) as u64);
    }
    if let Some(arr) = array.as_any().downcast_ref::<UInt16Array>() {
        return Ok(arr.value(row) as u64);
    }
    if let Some(arr) = array.as_any().downcast_ref::<UInt32Array>() {
        return Ok(arr.value(row) as u64);
    }
    if let Some(arr) = array.as_any().downcast_ref::<UInt64Array>() {
        return Ok(arr.value(row));
    }
    Err(ParquetReaderError::ConversionError {
        column_name: column_name.to_string(),
        row,
        message: format!("cannot cast {:?} to UInt64", array.data_type()),
    })
}

/// Extract an f64 from float or integer Arrow arrays.
fn cast_to_f64(array: &dyn Array, row: usize, column_name: &str) -> ParquetResult<f64> {
    if let Some(arr) = array.as_any().downcast_ref::<Float32Array>() {
        return Ok(arr.value(row) as f64);
    }
    if let Some(arr) = array.as_any().downcast_ref::<Float64Array>() {
        return Ok(arr.value(row));
    }
    if let Some(arr) = array.as_any().downcast_ref::<Int64Array>() {
        return Ok(arr.value(row) as f64);
    }
    if let Some(arr) = array.as_any().downcast_ref::<Int32Array>() {
        return Ok(arr.value(row) as f64);
    }
    Err(ParquetReaderError::ConversionError {
        column_name: column_name.to_string(),
        row,
        message: format!("cannot cast {:?} to Float64", array.data_type()),
    })
}

/// Extract a string from various Arrow array types.
fn array_to_string(array: &dyn Array, row: usize, _column_name: &str) -> ParquetResult<String> {
    if let Some(arr) = array.as_any().downcast_ref::<StringArray>() {
        return Ok(arr.value(row).to_string());
    }
    if let Some(arr) = array.as_any().downcast_ref::<LargeStringArray>() {
        return Ok(arr.value(row).to_string());
    }
    if let Some(arr) = array.as_any().downcast_ref::<BinaryArray>() {
        return Ok(String::from_utf8_lossy(arr.value(row)).to_string());
    }
    if let Some(arr) = array.as_any().downcast_ref::<Int64Array>() {
        return Ok(arr.value(row).to_string());
    }
    if let Some(arr) = array.as_any().downcast_ref::<Float64Array>() {
        return Ok(arr.value(row).to_string());
    }
    if let Some(arr) = array.as_any().downcast_ref::<BooleanArray>() {
        return Ok(arr.value(row).to_string());
    }
    // Fallback: use Debug formatting
    Ok(format!("{:?}", array))
}

// ─── Date/timestamp casting ─────────────────────────────────────────────────────

/// Extract days since epoch from Date32/Date64 Arrow arrays.
fn cast_date_to_days(array: &dyn Array, row: usize, column_name: &str) -> ParquetResult<i32> {
    if let Some(arr) = array.as_any().downcast_ref::<Date32Array>() {
        return Ok(arr.value(row));
    }
    if let Some(arr) = array.as_any().downcast_ref::<Date64Array>() {
        // Date64 is milliseconds since epoch
        return Ok((arr.value(row) / 86_400_000) as i32);
    }
    Err(ParquetReaderError::ConversionError {
        column_name: column_name.to_string(),
        row,
        message: format!("cannot cast {:?} to Date", array.data_type()),
    })
}

/// Extract microseconds since epoch from Timestamp Arrow arrays.
fn cast_timestamp_to_micros(array: &dyn Array, row: usize, column_name: &str) -> ParquetResult<i64> {
    if let Some(arr) = array.as_any().downcast_ref::<TimestampSecondArray>() {
        return Ok(arr.value(row) * 1_000_000);
    }
    if let Some(arr) = array.as_any().downcast_ref::<TimestampMillisecondArray>() {
        return Ok(arr.value(row) * 1_000);
    }
    if let Some(arr) = array.as_any().downcast_ref::<TimestampMicrosecondArray>() {
        return Ok(arr.value(row));
    }
    if let Some(arr) = array.as_any().downcast_ref::<TimestampNanosecondArray>() {
        return Ok(arr.value(row) / 1_000);
    }
    Err(ParquetReaderError::ConversionError {
        column_name: column_name.to_string(),
        row,
        message: format!("cannot cast {:?} to Timestamp", array.data_type()),
    })
}

// ─── Complex type helpers ───────────────────────────────────────────────────────

/// Convert a List array entry to `Vec<Value>`, preserving the element type
/// (Float64 → `Value::Double`, Int64 → `Value::Int64`, ...) so FLOAT[]
/// embeddings round-trip through parquet (P53.37). The previous String
/// fallback turned every element into `Value::String`, which the FLOAT[]
/// column could not coerce.
fn array_list_to_values(array: &dyn Array, row: usize, column_name: &str) -> ParquetResult<Vec<Value>> {
    let list_arr = downcast::<ListArray>(array, column_name)?;
    let values = list_arr.value(row);
    let mut result = Vec::with_capacity(values.len());
    for i in 0..values.len() {
        if values.is_null(i) {
            result.push(Value::Null);
        } else if let Some(f) = values.as_any().downcast_ref::<Float64Array>() {
            result.push(Value::Double(f.value(i)));
        } else if let Some(f) = values.as_any().downcast_ref::<Float32Array>() {
            result.push(Value::Float(f.value(i)));
        } else if let Some(a) = values.as_any().downcast_ref::<Int64Array>() {
            result.push(Value::Int64(a.value(i)));
        } else if let Some(a) = values.as_any().downcast_ref::<Int32Array>() {
            result.push(Value::Int32(a.value(i)));
        } else if let Some(a) = values.as_any().downcast_ref::<Int16Array>() {
            result.push(Value::Int16(a.value(i)));
        } else if let Some(a) = values.as_any().downcast_ref::<Int8Array>() {
            result.push(Value::Int8(a.value(i)));
        } else if let Some(a) = values.as_any().downcast_ref::<BooleanArray>() {
            result.push(Value::Bool(a.value(i)));
        } else if let Some(s) = values.as_any().downcast_ref::<StringArray>() {
            result.push(Value::String(s.value(i).to_string()));
        } else {
            // Unknown element type — generic string fallback.
            let s = array_to_string(&values, i, column_name)?;
            result.push(Value::String(s));
        }
    }
    Ok(result)
}

/// Convert a Struct array entry to `Vec<(String, Value)>`.
fn array_struct_to_values(array: &dyn Array, row: usize, column_name: &str) -> ParquetResult<Vec<(String, Value)>> {
    let struct_arr = downcast::<StructArray>(array, column_name)?;
    let mut result = Vec::with_capacity(struct_arr.num_columns());
    for col_idx in 0..struct_arr.num_columns() {
        let field = struct_arr.column(col_idx);
        let field_name = struct_arr
            .fields()
            .get(col_idx)
            .map(|f| f.name().clone())
            .unwrap_or_else(|| format!("_{col_idx}"));
        let val = if field.is_null(row) {
            Value::Null
        } else {
            Value::String(array_to_string(field.as_ref(), row, column_name)?)
        };
        result.push((field_name, val));
    }
    Ok(result)
}

/// Convert a Map array entry to `Vec<(Value, Value)>`.
fn array_map_to_values(array: &dyn Array, row: usize, column_name: &str) -> ParquetResult<Vec<(Value, Value)>> {
    let map_arr = downcast::<MapArray>(array, column_name)?;
    let entries = map_arr.value(row);
    let keys = map_arr.keys();
    let values = map_arr.values();

    let mut result = Vec::new();
    if let Some(_entries_struct) = entries.as_any().downcast_ref::<StructArray>() {
        // Map entries are stored as a list of structs {key, value}
        for i in 0..entries.len() {
            let key_val = if keys.is_null(i) {
                Value::Null
            } else {
                Value::String(array_to_string(keys, i, column_name)?)
            };
            let val_val = if values.is_null(i) {
                Value::Null
            } else {
                Value::String(array_to_string(values, i, column_name)?)
            };
            result.push((key_val, val_val));
        }
    }
    Ok(result)
}

// ─── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::datatypes::{DataType, Field, Schema};
    use std::sync::Arc;

    /// Write a RecordBatch to a temporary parquet file and return the path.
    fn write_parquet_batch(dir: &tempfile::TempDir, filename: &str, batch: &RecordBatch) -> std::path::PathBuf {
        let path = dir.path().join(filename);
        let file = std::fs::File::create(&path).unwrap();
        let schema = batch.schema();
        let mut writer = parquet::arrow::ArrowWriter::try_new(file, schema, None).unwrap();
        writer.write(batch).unwrap();
        writer.close().unwrap();
        path
    }

    fn test_schema() -> Vec<CatalogColumn> {
        vec![
            CatalogColumn {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "name".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: true,
                default_value: None,
            },
            CatalogColumn {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "age".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "score".into(),
                logical_type: LogicalTypeID::Double,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "active".into(),
                logical_type: LogicalTypeID::Bool,
                is_primary_key: false,
                default_value: None,
            },
        ]
    }

    #[test]
    fn test_validate_type_compatibility() {
        assert!(validate_type_compatibility(&ArrowDataType::Int64, LogicalTypeID::Int64).is_ok());
        assert!(validate_type_compatibility(&ArrowDataType::Utf8, LogicalTypeID::String).is_ok());
        assert!(validate_type_compatibility(&ArrowDataType::Boolean, LogicalTypeID::Bool).is_ok());
        assert!(validate_type_compatibility(&ArrowDataType::Float64, LogicalTypeID::Double).is_ok());
        // Widening
        assert!(validate_type_compatibility(&ArrowDataType::Int32, LogicalTypeID::Int64).is_ok());
        assert!(validate_type_compatibility(&ArrowDataType::Int8, LogicalTypeID::Int64).is_ok());
        // Mismatch
        assert!(validate_type_compatibility(&ArrowDataType::Int64, LogicalTypeID::String).is_ok()); // String accepts anything
        assert!(validate_type_compatibility(&ArrowDataType::Boolean, LogicalTypeID::Int64).is_err());
    }

    #[test]
    fn test_cast_int_to_i64() {
        let i32_arr = Int32Array::from(vec![42, -1]);
        assert_eq!(cast_int_to_i64(&i32_arr, 0, "col").unwrap(), 42i64);
        assert_eq!(cast_int_to_i64(&i32_arr, 1, "col").unwrap(), -1i64);

        let i64_arr = Int64Array::from(vec![999_999_999_999i64]);
        assert_eq!(cast_int_to_i64(&i64_arr, 0, "col").unwrap(), 999_999_999_999i64);
    }

    #[test]
    fn test_cast_to_f64() {
        let f64_arr = Float64Array::from(vec![3.15]);
        assert!((cast_to_f64(&f64_arr, 0, "col").unwrap() - 3.15).abs() < 1e-10);

        let f32_arr = Float32Array::from(vec![2.5f32]);
        assert!((cast_to_f64(&f32_arr, 0, "col").unwrap() - 2.5).abs() < 1e-10);
    }

    #[test]
    fn test_array_to_string() {
        let str_arr = StringArray::from(vec!["hello"]);
        assert_eq!(array_to_string(&str_arr, 0, "col").unwrap(), "hello");

        let int_arr = Int64Array::from(vec![42]);
        assert_eq!(array_to_string(&int_arr, 0, "col").unwrap(), "42");
    }

    #[test]
    fn test_cast_date_to_days() {
        let date_arr = Date32Array::from(vec![0i32, 19723i32]); // 1970-01-01, 2024-01-01
        assert_eq!(cast_date_to_days(&date_arr, 0, "col").unwrap(), 0);
        assert_eq!(cast_date_to_days(&date_arr, 1, "col").unwrap(), 19723);
    }

    #[test]
    fn test_cast_timestamp_to_micros() {
        use arrow::array::TimestampMicrosecondArray;
        let ts_arr = TimestampMicrosecondArray::from(vec![1_700_000_000_000_000i64]);
        let micros = cast_timestamp_to_micros(&ts_arr, 0, "col").unwrap();
        assert_eq!(micros, 1_700_000_000_000_000i64);
    }

    #[test]
    fn test_column_not_found() {
        let dir = tempfile::tempdir().unwrap();
        // Create a parquet file with one valid column
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Int64, false),
            Field::new("y", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1])),
                Arc::new(StringArray::from(vec!["a"])),
            ],
        )
        .unwrap();
        let path = write_parquet_batch(&dir, "test.parquet", &batch);

        let columns = vec![CatalogColumn {
            compression: akar_common::enums::CompressionType::Uncompressed,
            name: "missing_col".into(),
            logical_type: LogicalTypeID::Int64,
            is_primary_key: false,
            default_value: None,
        }];

        let result = read_parquet(
            path.to_str().unwrap(),
            &akar_common::file_system::VirtualFileSystemRegistry::new(),
            &columns,
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            ParquetReaderError::ColumnNotFound { column_name, .. } => {
                assert_eq!(column_name, "missing_col");
            }
            e => panic!("Expected ColumnNotFound, got: {e}"),
        }
    }

    #[test]
    fn test_type_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let schema = Arc::new(Schema::new(vec![Field::new("val", DataType::Boolean, false)]));
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(BooleanArray::from(vec![true]))]).unwrap();
        let path = write_parquet_batch(&dir, "mismatch.parquet", &batch);

        let columns = vec![CatalogColumn {
            compression: akar_common::enums::CompressionType::Uncompressed,
            name: "val".into(),
            logical_type: LogicalTypeID::Int64,
            is_primary_key: false,
            default_value: None,
        }];

        let result = read_parquet(
            path.to_str().unwrap(),
            &akar_common::file_system::VirtualFileSystemRegistry::new(),
            &columns,
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            ParquetReaderError::TypeMismatch { .. } => {} // expected
            e => panic!("Expected TypeMismatch, got: {e}"),
        }
    }

    #[test]
    fn test_file_not_found() {
        let result = read_parquet(
            "nonexistent.parquet",
            &akar_common::file_system::VirtualFileSystemRegistry::new(),
            &test_schema(),
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            ParquetReaderError::ParquetError(_) => {} // expected
            _ => panic!("Expected ParquetError"),
        }
    }

    #[test]
    fn test_arrow_array_to_value_basic() {
        // Bool
        let bool_arr = BooleanArray::from(vec![Some(true), None, Some(false)]);
        assert_eq!(
            arrow_array_to_value(&bool_arr, 0, "b", LogicalTypeID::Bool, 0).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            arrow_array_to_value(&bool_arr, 1, "b", LogicalTypeID::Bool, 0).unwrap(),
            Value::Null
        );

        // Int64
        let int_arr = Int64Array::from(vec![Some(42), None]);
        assert_eq!(
            arrow_array_to_value(&int_arr, 0, "i", LogicalTypeID::Int64, 0).unwrap(),
            Value::Int64(42)
        );
        assert_eq!(
            arrow_array_to_value(&int_arr, 1, "i", LogicalTypeID::Int64, 0).unwrap(),
            Value::Null
        );

        // String
        let str_arr = StringArray::from(vec![Some("hello"), None]);
        assert_eq!(
            arrow_array_to_value(&str_arr, 0, "s", LogicalTypeID::String, 0).unwrap(),
            Value::String("hello".into())
        );
        assert_eq!(
            arrow_array_to_value(&str_arr, 1, "s", LogicalTypeID::String, 0).unwrap(),
            Value::Null
        );

        // Double
        let f64_arr = Float64Array::from(vec![Some(3.15), None]);
        assert_eq!(
            arrow_array_to_value(&f64_arr, 0, "d", LogicalTypeID::Double, 0).unwrap(),
            Value::Double(3.15)
        );
    }

    #[test]
    fn test_arrow_array_widening() {
        // Int32 → Int64 widening
        let i32_arr = Int32Array::from(vec![100]);
        assert_eq!(
            arrow_array_to_value(&i32_arr, 0, "i", LogicalTypeID::Int64, 0).unwrap(),
            Value::Int64(100)
        );

        // Float32 → Double widening
        let f32_arr = Float32Array::from(vec![2.5f32]);
        let val = arrow_array_to_value(&f32_arr, 0, "f", LogicalTypeID::Double, 0).unwrap();
        if let Value::Double(d) = val {
            assert!((d - 2.5).abs() < 1e-6);
        } else {
            panic!("Expected Double");
        }
    }

    #[test]
    fn test_unsigned_int_types() {
        let u8_arr = UInt8Array::from(vec![200u8]);
        assert_eq!(
            arrow_array_to_value(&u8_arr, 0, "u", LogicalTypeID::UInt8, 0).unwrap(),
            Value::UInt8(200)
        );

        let u32_arr = UInt32Array::from(vec![100000u32]);
        assert_eq!(
            arrow_array_to_value(&u32_arr, 0, "u", LogicalTypeID::UInt32, 0).unwrap(),
            Value::UInt32(100000)
        );

        let u64_arr = UInt64Array::from(vec![u64::MAX]);
        assert_eq!(
            arrow_array_to_value(&u64_arr, 0, "u", LogicalTypeID::UInt64, 0).unwrap(),
            Value::UInt64(u64::MAX)
        );
    }

    #[test]
    fn test_date_conversion() {
        let date_arr = Date32Array::from(vec![7439i32]); // 1990-05-15
        let val = arrow_array_to_value(&date_arr, 0, "d", LogicalTypeID::Date, 0).unwrap();
        if let Value::Date(d) = val {
            assert_eq!(d.days_since_epoch(), 7439);
        } else {
            panic!("Expected Date");
        }
    }

    #[test]
    fn test_timestamp_conversion() {
        use arrow::array::TimestampMicrosecondArray;
        let ts_arr = TimestampMicrosecondArray::from(vec![1_704_198_600_000_000i64]); // 2024-01-02 12:30:00.000000
        let val = arrow_array_to_value(&ts_arr, 0, "t", LogicalTypeID::Timestamp, 0).unwrap();
        if let Value::Timestamp(ts) = val {
            assert_eq!(ts.micros_since_epoch(), 1_704_198_600_000_000);
        } else {
            panic!("Expected Timestamp");
        }
    }

    #[test]
    fn test_blob_conversion() {
        let blob_arr = BinaryArray::from(vec![&b"hello"[..]]);
        let val = arrow_array_to_value(&blob_arr, 0, "b", LogicalTypeID::Blob, 0).unwrap();
        assert_eq!(val, Value::Blob(b"hello".to_vec()));
    }

    #[test]
    fn test_round_trip_parquet() {
        let dir = tempfile::tempdir().unwrap();

        // Write a parquet file with known data
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("age", DataType::Int64, false),
            Field::new("score", DataType::Float64, false),
            Field::new("active", DataType::Boolean, false),
        ]));

        let names = StringArray::from(vec!["Alice", "Bob", "Charlie"]);
        let ages = Int64Array::from(vec![30, 25, 35]);
        let scores = Float64Array::from(vec![95.5, 87.3, 91.2]);
        let actives = BooleanArray::from(vec![true, false, true]);

        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(names), Arc::new(ages), Arc::new(scores), Arc::new(actives)],
        )
        .unwrap();

        let parquet_path = write_parquet_batch(&dir, "roundtrip.parquet", &batch);

        // Read it back using our reader
        let columns = test_schema();
        let rows = read_parquet(
            parquet_path.to_str().unwrap(),
            &akar_common::file_system::VirtualFileSystemRegistry::new(),
            &columns,
        )
        .unwrap();

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0][0], Value::String("Alice".into()));
        assert_eq!(rows[0][1], Value::Int64(30));
        assert_eq!(rows[0][2], Value::Double(95.5));
        assert_eq!(rows[0][3], Value::Bool(true));

        assert_eq!(rows[1][0], Value::String("Bob".into()));
        assert_eq!(rows[1][1], Value::Int64(25));
        assert_eq!(rows[1][2], Value::Double(87.3));
        assert_eq!(rows[1][3], Value::Bool(false));

        assert_eq!(rows[2][0], Value::String("Charlie".into()));
        assert_eq!(rows[2][1], Value::Int64(35));
        assert_eq!(rows[2][2], Value::Double(91.2));
        assert_eq!(rows[2][3], Value::Bool(true));
    }

    #[test]
    fn test_round_trip_with_nulls() {
        let dir = tempfile::tempdir().unwrap();

        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, true),
            Field::new("age", DataType::Int64, true),
        ]));

        let names = StringArray::from(vec![Some("Alice"), None, Some("Charlie")]);
        let ages = Int64Array::from(vec![Some(30), Some(25), None]);

        let batch = RecordBatch::try_new(schema, vec![Arc::new(names), Arc::new(ages)]).unwrap();

        let parquet_path = write_parquet_batch(&dir, "nulls.parquet", &batch);

        let columns = vec![
            CatalogColumn {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "name".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "age".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
                default_value: None,
            },
        ];

        let rows = read_parquet(
            parquet_path.to_str().unwrap(),
            &akar_common::file_system::VirtualFileSystemRegistry::new(),
            &columns,
        )
        .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0][0], Value::String("Alice".into()));
        assert_eq!(rows[0][1], Value::Int64(30));
        assert_eq!(rows[1][0], Value::Null);
        assert_eq!(rows[1][1], Value::Int64(25));
        assert_eq!(rows[2][0], Value::String("Charlie".into()));
        assert_eq!(rows[2][1], Value::Null);
    }

    #[test]
    fn test_round_trip_unsigned_and_floats() {
        let dir = tempfile::tempdir().unwrap();

        let schema = Arc::new(Schema::new(vec![
            Field::new("small", DataType::UInt8, false),
            Field::new("medium", DataType::UInt32, false),
            Field::new("large", DataType::UInt64, false),
            Field::new("temp", DataType::Float32, false),
        ]));

        let small = UInt8Array::from(vec![100u8, 200u8]);
        let medium = UInt32Array::from(vec![1000u32, 50000u32]);
        let large = UInt64Array::from(vec![100000u64, u64::MAX]);
        let temp = Float32Array::from(vec![36.5f32, 98.6f32]);

        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(small), Arc::new(medium), Arc::new(large), Arc::new(temp)],
        )
        .unwrap();

        let parquet_path = write_parquet_batch(&dir, "uints.parquet", &batch);

        let columns = vec![
            CatalogColumn {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "small".into(),
                logical_type: LogicalTypeID::UInt8,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "medium".into(),
                logical_type: LogicalTypeID::UInt32,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "large".into(),
                logical_type: LogicalTypeID::UInt64,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "temp".into(),
                logical_type: LogicalTypeID::Float,
                is_primary_key: false,
                default_value: None,
            },
        ];

        let rows = read_parquet(
            parquet_path.to_str().unwrap(),
            &akar_common::file_system::VirtualFileSystemRegistry::new(),
            &columns,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], Value::UInt8(100));
        assert_eq!(rows[0][1], Value::UInt32(1000));
        assert_eq!(rows[0][2], Value::UInt64(100000));
        if let Value::Float(f) = rows[0][3] {
            assert!((f - 36.5).abs() < 1e-5);
        } else {
            panic!("Expected Float");
        }
        assert_eq!(rows[1][0], Value::UInt8(200));
        assert_eq!(rows[1][2], Value::UInt64(u64::MAX));
    }
}
