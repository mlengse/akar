use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::ValueVector;

pub fn extract_all_rows_from_chunks(chunks: &[akar_common::vector::DataChunk]) -> Vec<Vec<Value>> {
    let mut all_rows = Vec::new();
    for chunk in chunks {
        let rows = extract_all_rows(chunk);
        all_rows.extend(rows);
    }
    all_rows
}

pub fn extract_all_rows(chunk: &akar_common::vector::DataChunk) -> Vec<Vec<Value>> {
    let num_rows = chunk.size;
    let num_cols = chunk.fields.len();
    let mut rows: Vec<Vec<Value>> = Vec::with_capacity(num_rows);
    for row in 0..num_rows {
        let mut values: Vec<Value> = Vec::with_capacity(num_cols);
        for col in 0..num_cols {
            let val = chunk.get_value(col, row).unwrap_or(Value::Null);
            values.push(val);
        }
        rows.push(values);
    }
    rows
}

pub fn rows_to_columns(rows: &[Vec<Value>]) -> (Vec<arrow::array::ArrayRef>, Vec<PhysicalTypeID>) {
    if rows.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let num_cols = rows[0].len();
    let num_rows = rows.len();

    let mut fields = Vec::with_capacity(num_cols);
    let mut field_types = Vec::with_capacity(num_cols);

    for col in 0..num_cols {
        let first_val = &rows[0][col];
        let phys_type = value_to_physical_type(first_val);
        let mut vec = ValueVector::new(phys_type, num_rows.max(1));
        for (row_idx, row) in rows.iter().enumerate() {
            let val = &row[col];
            let _ = vec.set_value(row_idx, val);
        }
        vec.resize(num_rows);
        fields.push(akar_common::arrow_vector::ArrowVector::from_legacy(&vec).array);
        field_types.push(phys_type);
    }
    (fields, field_types)
}

pub fn value_to_physical_type(val: &Value) -> PhysicalTypeID {
    match val {
        Value::Null => PhysicalTypeID::Any,
        Value::Bool(_) => PhysicalTypeID::Bool,
        Value::Int64(_) => PhysicalTypeID::Int64,
        Value::Int32(_) => PhysicalTypeID::Int32,
        Value::Int16(_) => PhysicalTypeID::Int16,
        Value::Int8(_) => PhysicalTypeID::Int8,
        Value::UInt64(_) => PhysicalTypeID::UInt64,
        Value::UInt32(_) => PhysicalTypeID::UInt32,
        Value::UInt16(_) => PhysicalTypeID::UInt16,
        Value::UInt8(_) => PhysicalTypeID::UInt8,
        Value::Int128(_) => PhysicalTypeID::Int128,
        Value::Double(_) => PhysicalTypeID::Double,
        Value::Float(_) => PhysicalTypeID::Float,
        Value::String(_) | Value::Blob(_) => PhysicalTypeID::String,
        Value::Date(_)
        | Value::Timestamp(_)
        | Value::TimestampTz(_)
        | Value::TimestampNs(_)
        | Value::TimestampMs(_)
        | Value::TimestampSec(_)
        | Value::Interval(_) => PhysicalTypeID::Int64,
        Value::InternalID(_) => PhysicalTypeID::Int64,
        Value::UInt128(_) => PhysicalTypeID::Int128,
        Value::Json(_) => PhysicalTypeID::String,
        Value::DTime(_) => PhysicalTypeID::Int64,
        Value::Union(_, _) => PhysicalTypeID::Struct,
        Value::List(_) => PhysicalTypeID::List,
        Value::Map(_) => PhysicalTypeID::Struct,
        Value::Struct(_) => PhysicalTypeID::Struct,
    }
}
