use crate::selection::SelectionVector;
use crate::types::{PhysicalTypeID, Value};
use crate::vector::ValueVector;
use arrow::array::{
    Array, ArrayRef, BooleanArray, BooleanBuilder, Float32Array, Float32Builder, Float64Array, Float64Builder,
    Int32Array, Int32Builder, Int64Array, Int64Builder, ListArray, StringArray, StringBuilder, StructArray,
    UInt64Array, UInt64Builder,
};
use arrow::buffer::{NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow::datatypes::{DataType, Field};
use std::sync::Arc;

pub trait VectorAccess {
    fn size(&self) -> usize;
    fn physical_type(&self) -> PhysicalTypeID;
    fn is_null(&self, row: usize) -> bool;

    fn get_i64(&self, row: usize) -> Option<i64>;
    fn get_i32(&self, row: usize) -> Option<i32>;
    fn get_f64(&self, row: usize) -> Option<f64>;
    fn get_f32(&self, row: usize) -> Option<f32>;
    fn get_bool(&self, row: usize) -> Option<bool>;
    fn get_value(&self, row: usize) -> Option<Value>;

    fn get_i64_sel(&self, pos: usize, sel: &SelectionVector) -> Option<i64> {
        if pos < sel.size {
            self.get_i64(sel.indices[pos] as usize)
        } else {
            None
        }
    }
    fn get_i32_sel(&self, pos: usize, sel: &SelectionVector) -> Option<i32> {
        if pos < sel.size {
            self.get_i32(sel.indices[pos] as usize)
        } else {
            None
        }
    }
    fn get_f64_sel(&self, pos: usize, sel: &SelectionVector) -> Option<f64> {
        if pos < sel.size {
            self.get_f64(sel.indices[pos] as usize)
        } else {
            None
        }
    }
    fn get_f32_sel(&self, pos: usize, sel: &SelectionVector) -> Option<f32> {
        if pos < sel.size {
            self.get_f32(sel.indices[pos] as usize)
        } else {
            None
        }
    }
    fn get_bool_sel(&self, pos: usize, sel: &SelectionVector) -> Option<bool> {
        if pos < sel.size {
            self.get_bool(sel.indices[pos] as usize)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct ArrowVector {
    pub array: ArrayRef,
    pub physical_type: PhysicalTypeID,
}

impl ArrowVector {
    pub fn new(array: ArrayRef, physical_type: PhysicalTypeID) -> Self {
        Self { array, physical_type }
    }

    pub fn from_legacy(vec: &ValueVector) -> Self {
        let phys_type = vec.physical_type();
        let size = vec.size();

        // Fast path for primitive arrays
        let build_primitive_array = |data_type: arrow::datatypes::DataType, type_size: usize| -> ArrayRef {
            let num_bytes = size.div_ceil(8);
            let mut null_buffer = arrow::buffer::MutableBuffer::from_len_zeroed(num_bytes);
            let slice = null_buffer.as_slice_mut();
            for i in 0..size {
                if !vec.is_null(i) {
                    arrow::util::bit_util::set_bit(slice, i);
                }
            }
            let null_buffer = null_buffer.into();
            let data_buffer = arrow::buffer::Buffer::from_slice_ref(&vec.data()[..size * type_size]);

            let array_data = arrow::array::ArrayData::builder(data_type.clone())
                .len(size)
                .add_buffer(data_buffer)
                .null_bit_buffer(Some(null_buffer))
                .build()
                .unwrap();
            arrow::array::make_array(array_data)
        };

        let array: ArrayRef = match phys_type {
            PhysicalTypeID::Bool => {
                let mut builder = arrow::array::BooleanBuilder::with_capacity(size);
                for i in 0..size {
                    if vec.is_null(i) {
                        builder.append_null();
                    } else {
                        builder.append_value(vec.get_bool(i).unwrap_or(false));
                    }
                }
                Arc::new(builder.finish())
            }
            PhysicalTypeID::Int64 => build_primitive_array(arrow::datatypes::DataType::Int64, 8),
            PhysicalTypeID::UInt64 => build_primitive_array(arrow::datatypes::DataType::UInt64, 8),
            PhysicalTypeID::Int32 => build_primitive_array(arrow::datatypes::DataType::Int32, 4),
            PhysicalTypeID::Double => build_primitive_array(arrow::datatypes::DataType::Float64, 8),
            PhysicalTypeID::Float => build_primitive_array(arrow::datatypes::DataType::Float32, 4),
            PhysicalTypeID::String => {
                let mut builder = arrow::array::StringBuilder::with_capacity(size, size * 16);
                for i in 0..size {
                    if vec.is_null(i) {
                        builder.append_null();
                    } else {
                        if let Some(val) = vec.get_value(i) {
                            if let Value::String(s) = val {
                                builder.append_value(&s);
                            } else {
                                builder.append_null();
                            }
                        } else {
                            builder.append_null();
                        }
                    }
                }
                Arc::new(builder.finish())
            }
            PhysicalTypeID::List | PhysicalTypeID::Array => {
                // A legacy vector has no side-storage for variable-length list
                // payloads; emit empty lists for non-null rows (callers should
                // prefer `arrow_array_from_values` for real list data).
                let values: Vec<Value> = (0..size)
                    .map(|i| {
                        if vec.is_null(i) {
                            Value::Null
                        } else {
                            vec.get_value(i).unwrap_or(Value::Null)
                        }
                    })
                    .collect();
                arrow_array_from_values(&values)
            }
            PhysicalTypeID::Struct => {
                // Same limitation as List: emit the legacy placeholder value.
                let values: Vec<Value> = (0..size)
                    .map(|i| {
                        if vec.is_null(i) {
                            Value::Null
                        } else {
                            vec.get_value(i).unwrap_or(Value::Null)
                        }
                    })
                    .collect();
                arrow_array_from_values(&values)
            }
            _ => {
                let mut builder = arrow::array::Int64Builder::with_capacity(size);
                for _ in 0..size {
                    builder.append_null();
                }
                Arc::new(builder.finish())
            }
        };
        Self::new(array, phys_type)
    }

    pub fn data_type(&self) -> DataType {
        self.array.data_type().clone()
    }
}

impl VectorAccess for ArrowVector {
    #[inline(always)]
    fn size(&self) -> usize {
        self.array.len()
    }

    #[inline(always)]
    fn physical_type(&self) -> PhysicalTypeID {
        self.physical_type
    }

    #[inline(always)]
    fn is_null(&self, row: usize) -> bool {
        if row >= self.array.len() {
            return true;
        }
        self.array.is_null(row)
    }

    #[inline]
    fn get_i64(&self, row: usize) -> Option<i64> {
        if row >= self.array.len() {
            return None;
        }
        let array = self.array.as_any().downcast_ref::<Int64Array>()?;
        if array.is_null(row) {
            None
        } else {
            Some(array.value(row))
        }
    }

    #[inline]
    fn get_i32(&self, row: usize) -> Option<i32> {
        if row >= self.array.len() {
            return None;
        }
        let array = self.array.as_any().downcast_ref::<Int32Array>()?;
        if array.is_null(row) {
            None
        } else {
            Some(array.value(row))
        }
    }

    #[inline]
    fn get_f64(&self, row: usize) -> Option<f64> {
        if row >= self.array.len() {
            return None;
        }
        let array = self.array.as_any().downcast_ref::<Float64Array>()?;
        if array.is_null(row) {
            None
        } else {
            Some(array.value(row))
        }
    }

    #[inline]
    fn get_f32(&self, row: usize) -> Option<f32> {
        if row >= self.array.len() {
            return None;
        }
        let array = self.array.as_any().downcast_ref::<Float32Array>()?;
        if array.is_null(row) {
            None
        } else {
            Some(array.value(row))
        }
    }

    #[inline]
    fn get_bool(&self, row: usize) -> Option<bool> {
        if row >= self.array.len() {
            return None;
        }
        let array = self.array.as_any().downcast_ref::<BooleanArray>()?;
        if array.is_null(row) {
            None
        } else {
            Some(array.value(row))
        }
    }

    #[inline]
    fn get_value(&self, row: usize) -> Option<Value> {
        if row >= self.array.len() || self.array.is_null(row) {
            return None;
        }
        match self.physical_type {
            PhysicalTypeID::Bool => self.get_bool(row).map(Value::Bool),
            PhysicalTypeID::Int64 => self.get_i64(row).map(Value::Int64),
            PhysicalTypeID::UInt64 => {
                let array = self.array.as_any().downcast_ref::<UInt64Array>()?;
                Some(Value::UInt64(array.value(row)))
            }
            PhysicalTypeID::Int32 => self.get_i32(row).map(Value::Int32),
            PhysicalTypeID::Double => self.get_f64(row).map(Value::Double),
            PhysicalTypeID::Float => self.get_f32(row).map(Value::Float),
            PhysicalTypeID::String => {
                let array = self.array.as_any().downcast_ref::<StringArray>()?;
                Some(Value::String(array.value(row).to_string()))
            }
            PhysicalTypeID::List => convert_arrow_scalar(&self.array, row),
            PhysicalTypeID::Struct => convert_arrow_scalar(&self.array, row),
            _ => {
                // For unsupported types, return null
                None
            }
        }
    }
}

pub fn convert_arrow_scalar(array: &ArrayRef, row: usize) -> Option<Value> {
    if array.is_null(row) {
        return None;
    }
    match array.data_type() {
        DataType::Boolean => {
            let arr = array.as_any().downcast_ref::<BooleanArray>()?;
            Some(Value::Bool(arr.value(row)))
        }
        DataType::Int64 => {
            let arr = array.as_any().downcast_ref::<Int64Array>()?;
            Some(Value::Int64(arr.value(row)))
        }
        DataType::Int32 => {
            let arr = array.as_any().downcast_ref::<Int32Array>()?;
            Some(Value::Int32(arr.value(row)))
        }
        DataType::Float64 => {
            let arr = array.as_any().downcast_ref::<Float64Array>()?;
            Some(Value::Double(arr.value(row)))
        }
        DataType::Float32 => {
            let arr = array.as_any().downcast_ref::<Float32Array>()?;
            Some(Value::Float(arr.value(row)))
        }
        DataType::Utf8 | DataType::LargeUtf8 => {
            let arr = array.as_any().downcast_ref::<StringArray>();
            if let Some(s_arr) = arr {
                return Some(Value::String(s_arr.value(row).to_string()));
            }
            // LargeUtf8 would need LargeStringArray, but ignoring for now
            None
        }
        DataType::List(_) => {
            let arr = array.as_any().downcast_ref::<ListArray>()?;
            let list_array = arr.value(row); // This is an ArrayRef
            let mut list_vals = Vec::new();
            for i in 0..list_array.len() {
                if let Some(v) = convert_arrow_scalar(&list_array, i) {
                    list_vals.push(v);
                } else {
                    list_vals.push(Value::Null);
                }
            }
            Some(Value::List(list_vals))
        }
        DataType::Struct(fields) => {
            let arr = array.as_any().downcast_ref::<StructArray>()?;
            let mut entries = Vec::new();
            for (i, field) in fields.iter().enumerate() {
                let col = arr.column(i);
                let val = convert_arrow_scalar(col, row).unwrap_or(Value::Null);
                entries.push((field.name().clone(), val));
            }
            Some(Value::Struct(entries))
        }
        _ => None,
    }
}

/// Infer the Arrow `DataType` for a list of `Value`s.
///
/// The first non-null value is used as the type sample. All-null inputs (or
/// unknown types) default to `Int64`, matching the engine's null fallback.
/// `Value::List`/`Value::Struct`/`Value::Map` are inferred recursively so that
/// nested complex types produce the right `List`/`Struct` Arrow types.
pub fn infer_arrow_type(values: &[Value]) -> DataType {
    let sample = values.iter().find(|v| !matches!(v, Value::Null));
    match sample {
        Some(Value::Bool(_)) => DataType::Boolean,
        Some(Value::Int64(_))
        | Some(Value::Int128(_))
        | Some(Value::UInt128(_))
        | Some(Value::Date(_))
        | Some(Value::Timestamp(_))
        | Some(Value::TimestampTz(_))
        | Some(Value::TimestampNs(_))
        | Some(Value::TimestampMs(_))
        | Some(Value::TimestampSec(_))
        | Some(Value::DTime(_)) => DataType::Int64,
        Some(Value::UInt64(_)) => DataType::UInt64,
        Some(Value::Int32(_)) | Some(Value::UInt32(_)) => DataType::Int32,
        Some(Value::Double(_)) => DataType::Float64,
        Some(Value::Float(_)) => DataType::Float32,
        Some(Value::String(_)) | Some(Value::Json(_)) | Some(Value::Interval(_)) => DataType::Utf8,
        Some(Value::List(inner)) => DataType::List(Arc::new(Field::new("item", infer_arrow_type(inner), true))),
        Some(Value::Struct(entries)) => {
            let fields = entries
                .iter()
                .map(|(k, v)| Field::new(k.clone(), infer_arrow_type(std::slice::from_ref(v)), true))
                .collect();
            DataType::Struct(fields)
        }
        Some(Value::Map(entries)) => {
            let fields = entries
                .iter()
                .map(|(k, v)| {
                    let name = match k {
                        Value::String(s) => s.clone(),
                        other => format!("{other:?}"),
                    };
                    Field::new(name, infer_arrow_type(std::slice::from_ref(v)), true)
                })
                .collect();
            DataType::Struct(fields)
        }
        Some(Value::Union(_, v)) => infer_arrow_type(std::slice::from_ref(v.as_ref())),
        Some(Value::InternalID(_)) => DataType::Int64,
        Some(Value::Blob(_)) => DataType::Binary,
        _ => DataType::Int64,
    }
}

/// Build an Arrow `ArrayRef` from a `Vec<Value>`, recursively handling
/// primitives, `List`, `Struct` and `Map` values.
///
/// This is the inverse of [`convert_arrow_scalar`] and is used by the scan /
/// projection paths to emit complex-typed columns (e.g. `FLOAT[]`, `{..}`
/// literals) without going through a `ValueVector` (which has no side-storage
/// for variable-length List/Struct payloads).
pub fn arrow_array_from_values(values: &[Value]) -> ArrayRef {
    let data_type = infer_arrow_type(values);
    match &data_type {
        DataType::Boolean => {
            let mut builder = BooleanBuilder::with_capacity(values.len());
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::Bool(b) => builder.append_value(*b),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        DataType::Int64 => {
            let mut builder = Int64Builder::with_capacity(values.len());
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::Int64(n) => builder.append_value(*n),
                    Value::Int128(n) => builder.append_value(*n as i64),
                    Value::UInt128(n) => builder.append_value(*n as i64),
                    Value::Int32(n) => builder.append_value(*n as i64),
                    Value::UInt64(n) => builder.append_value(*n as i64),
                    Value::Date(n) => builder.append_value(n.0 as i64),
                    Value::Timestamp(n) | Value::TimestampNs(n) | Value::TimestampMs(n) | Value::TimestampSec(n) => {
                        builder.append_value(n.0)
                    }
                    Value::TimestampTz(n) => builder.append_value(n.0),
                    Value::DTime(n) => builder.append_value(*n),
                    Value::InternalID(id) => builder.append_value(id.offset as i64),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        DataType::UInt64 => {
            let mut builder = UInt64Builder::with_capacity(values.len());
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::UInt64(n) => builder.append_value(*n),
                    Value::Int64(n) if *n >= 0 => builder.append_value(*n as u64),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        DataType::Int32 => {
            let mut builder = Int32Builder::with_capacity(values.len());
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::Int32(n) => builder.append_value(*n),
                    Value::Int64(n) => builder.append_value(*n as i32),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        DataType::Float64 => {
            let mut builder = Float64Builder::with_capacity(values.len());
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::Double(n) => builder.append_value(*n),
                    Value::Float(n) => builder.append_value(*n as f64),
                    Value::Int64(n) => builder.append_value(*n as f64),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        DataType::Float32 => {
            let mut builder = Float32Builder::with_capacity(values.len());
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::Float(n) => builder.append_value(*n),
                    Value::Double(n) => builder.append_value(*n as f32),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        DataType::Utf8 => {
            let mut builder = StringBuilder::with_capacity(values.len(), values.len() * 16);
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::String(s) => builder.append_value(s),
                    Value::Interval(i) => {
                        builder.append_value(format!("{} months {} days {} microseconds", i.months, i.days, i.micros))
                    }
                    Value::Json(j) => builder.append_value(j.to_string()),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        DataType::List(_) => {
            let mut child_values: Vec<Value> = Vec::new();
            let mut offsets: Vec<i32> = Vec::with_capacity(values.len() + 1);
            let mut valid: Vec<bool> = Vec::with_capacity(values.len());
            offsets.push(0);
            for v in values {
                match v {
                    Value::List(inner) => {
                        child_values.extend(inner.iter().cloned());
                        offsets.push(child_values.len() as i32);
                        valid.push(true);
                    }
                    _ => {
                        offsets.push(child_values.len() as i32);
                        valid.push(false);
                    }
                }
            }
            let child = arrow_array_from_values(&child_values);
            let field = Arc::new(Field::new("item", child.data_type().clone(), true));
            let offsets_buf = OffsetBuffer::new(ScalarBuffer::from(offsets));
            let nulls = NullBuffer::from(valid);
            match ListArray::try_new(field, offsets_buf, child, Some(nulls)) {
                Ok(a) => Arc::new(a),
                Err(_) => {
                    let mut builder = Int64Builder::with_capacity(values.len());
                    builder.append_nulls(values.len());
                    Arc::new(builder.finish())
                }
            }
        }
        DataType::Struct(fields) => {
            let mut columns: Vec<ArrayRef> = Vec::with_capacity(fields.len());
            for field in fields.iter() {
                let col: Vec<Value> = values
                    .iter()
                    .map(|v| match v {
                        Value::Struct(entries) => entries
                            .iter()
                            .find(|(k, _)| k == field.name())
                            .map(|(_, val)| val.clone())
                            .unwrap_or(Value::Null),
                        Value::Map(entries) => entries
                            .iter()
                            .find(|(k, _)| matches!(k, Value::String(s) if s == field.name()))
                            .map(|(_, val)| val.clone())
                            .unwrap_or(Value::Null),
                        _ => Value::Null,
                    })
                    .collect();
                columns.push(arrow_array_from_values(&col));
            }
            let valid: Vec<bool> = values
                .iter()
                .map(|v| matches!(v, Value::Struct(_) | Value::Map(_)))
                .collect();
            let nulls = NullBuffer::from(valid);
            match StructArray::try_new(fields.clone(), columns, Some(nulls)) {
                Ok(a) => Arc::new(a),
                Err(_) => {
                    let mut builder = Int64Builder::with_capacity(values.len());
                    builder.append_nulls(values.len());
                    Arc::new(builder.finish())
                }
            }
        }
        DataType::Binary => {
            let mut builder = arrow::array::BinaryBuilder::with_capacity(values.len(), values.len() * 8);
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::Blob(b) => builder.append_value(b),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        _ => {
            let mut builder = Int64Builder::with_capacity(values.len());
            builder.append_nulls(values.len());
            Arc::new(builder.finish())
        }
    }
}

#[derive(Debug, Clone)]
pub enum Vector {
    Arrow(ArrowVector),
    Legacy(ValueVector),
}

impl Vector {
    #[inline]
    pub fn as_arrow(&self) -> Option<&ArrowVector> {
        match self {
            Vector::Arrow(a) => Some(a),
            _ => None,
        }
    }

    #[inline]
    pub fn as_legacy(&self) -> Option<&ValueVector> {
        match self {
            Vector::Legacy(l) => Some(l),
            _ => None,
        }
    }

    #[inline]
    pub fn as_legacy_mut(&mut self) -> Option<&mut ValueVector> {
        match self {
            Vector::Legacy(l) => Some(l),
            _ => None,
        }
    }
}

impl VectorAccess for Vector {
    fn size(&self) -> usize {
        match self {
            Vector::Arrow(a) => a.size(),
            Vector::Legacy(l) => l.size(),
        }
    }

    fn physical_type(&self) -> PhysicalTypeID {
        match self {
            Vector::Arrow(a) => a.physical_type(),
            Vector::Legacy(l) => l.physical_type(),
        }
    }

    fn is_null(&self, row: usize) -> bool {
        match self {
            Vector::Arrow(a) => a.is_null(row),
            Vector::Legacy(l) => l.is_null(row),
        }
    }

    fn get_i64(&self, row: usize) -> Option<i64> {
        match self {
            Vector::Arrow(a) => a.get_i64(row),
            Vector::Legacy(l) => l.get_i64(row),
        }
    }

    fn get_i32(&self, row: usize) -> Option<i32> {
        match self {
            Vector::Arrow(a) => a.get_i32(row),
            Vector::Legacy(l) => l.get_i32(row),
        }
    }

    fn get_f64(&self, row: usize) -> Option<f64> {
        match self {
            Vector::Arrow(a) => a.get_f64(row),
            Vector::Legacy(l) => l.get_double(row),
        }
    }

    fn get_f32(&self, row: usize) -> Option<f32> {
        match self {
            Vector::Arrow(a) => a.get_f32(row),
            Vector::Legacy(l) => {
                if l.is_null(row) {
                    return None;
                }
                let v = l.get_value(row)?;
                match v {
                    Value::Float(f) => Some(f),
                    Value::Double(d) => Some(d as f32),
                    _ => None,
                }
            }
        }
    }

    fn get_bool(&self, row: usize) -> Option<bool> {
        match self {
            Vector::Arrow(a) => a.get_bool(row),
            Vector::Legacy(l) => l.get_bool(row),
        }
    }

    fn get_value(&self, row: usize) -> Option<Value> {
        match self {
            Vector::Arrow(a) => a.get_value(row),
            Vector::Legacy(l) => l.get_value(row),
        }
    }
}

impl From<ValueVector> for Vector {
    fn from(v: ValueVector) -> Self {
        Vector::Legacy(v)
    }
}

impl From<ArrowVector> for Vector {
    fn from(a: ArrowVector) -> Self {
        Vector::Arrow(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PhysicalTypeID;
    use crate::vector::ValueVector;

    #[test]
    fn test_arrow_vector_from_legacy_i64() {
        let mut legacy = ValueVector::new(PhysicalTypeID::Int64, 5);
        legacy.set_i64(0, 10);
        legacy.set_i64(1, 20);
        legacy.set_i64(2, 30);
        legacy.set_null(3, true);
        legacy.set_i64(4, 50);
        legacy.resize(5);

        let arrow = ArrowVector::from_legacy(&legacy);
        assert_eq!(arrow.size(), 5);
        assert_eq!(arrow.get_i64(0), Some(10));
        assert_eq!(arrow.get_i64(1), Some(20));
        assert_eq!(arrow.get_i64(2), Some(30));
        assert_eq!(arrow.get_i64(3), None);
        assert_eq!(arrow.get_i64(4), Some(50));
    }

    #[test]
    fn test_arrow_vector_bool() {
        let mut legacy = ValueVector::new(PhysicalTypeID::Bool, 4);
        legacy.push_bool(true);
        legacy.push_bool(false);
        legacy.push_bool(true);
        legacy.set_null(3, true);
        legacy.resize(4);

        let arrow = ArrowVector::from_legacy(&legacy);
        assert_eq!(arrow.size(), 4);
        assert_eq!(arrow.get_bool(0), Some(true));
        assert_eq!(arrow.get_bool(1), Some(false));
        assert_eq!(arrow.get_bool(2), Some(true));
        assert_eq!(arrow.get_bool(3), None);
    }

    #[test]
    fn test_vector_enum_dispatch() {
        let mut legacy = ValueVector::new(PhysicalTypeID::Int64, 3);
        legacy.set_i64(0, 100);
        legacy.set_i64(1, 200);
        legacy.resize(2);

        let vec = Vector::Legacy(legacy);
        assert_eq!(vec.get_i64(0), Some(100));
        assert_eq!(vec.get_i64(1), Some(200));
        assert_eq!(vec.size(), 2);
    }

    #[test]
    fn test_selection_vector_access() {
        let mut legacy = ValueVector::new(PhysicalTypeID::Int64, 5);
        legacy.set_i64(0, 10);
        legacy.set_i64(1, 20);
        legacy.set_i64(2, 30);
        legacy.set_i64(3, 40);
        legacy.set_i64(4, 50);
        legacy.resize(5);

        let arrow = ArrowVector::from_legacy(&legacy);
        let sel = SelectionVector::from_slice(&[0, 2, 4]);

        assert_eq!(arrow.get_i64_sel(0, &sel), Some(10));
        assert_eq!(arrow.get_i64_sel(1, &sel), Some(30));
        assert_eq!(arrow.get_i64_sel(2, &sel), Some(50));
        assert_eq!(arrow.get_i64_sel(3, &sel), None);
    }

    #[test]
    fn test_arrow_array_from_list_values() {
        let values = vec![
            Value::List(vec![Value::Float(0.1), Value::Float(0.2)]),
            Value::List(vec![Value::Float(0.3)]),
            Value::Null,
        ];
        let arr = arrow_array_from_values(&values);
        assert_eq!(
            arr.data_type(),
            &DataType::List(Arc::new(Field::new("item", DataType::Float32, true)))
        );
        assert_eq!(arr.len(), 3);
        assert!(!arr.is_null(0));
        assert!(!arr.is_null(1));
        assert!(arr.is_null(2));
        let list_arr = arr.as_any().downcast_ref::<ListArray>().unwrap();
        let v0 = convert_arrow_scalar(&arr, 0).unwrap();
        assert_eq!(v0, Value::List(vec![Value::Float(0.1), Value::Float(0.2)]));
        let v1 = convert_arrow_scalar(&arr, 1).unwrap();
        assert_eq!(v1, Value::List(vec![Value::Float(0.3)]));
        assert_eq!(list_arr.value(0).len(), 2);
    }

    #[test]
    fn test_arrow_array_from_struct_values() {
        let values = vec![
            Value::Map(vec![(Value::String("id".into()), Value::Int64(1))]),
            Value::Null,
        ];
        let arr = arrow_array_from_values(&values);
        assert!(matches!(arr.data_type(), DataType::Struct(_)));
        assert_eq!(arr.len(), 2);
        assert!(!arr.is_null(0));
        assert!(arr.is_null(1));
        let v0 = convert_arrow_scalar(&arr, 0).unwrap();
        assert_eq!(v0, Value::Struct(vec![("id".to_string(), Value::Int64(1))]));
    }

    #[test]
    fn test_arrow_array_from_empty_list() {
        let values = vec![Value::List(vec![])];
        let arr = arrow_array_from_values(&values);
        assert!(matches!(arr.data_type(), DataType::List(_)));
        assert_eq!(convert_arrow_scalar(&arr, 0).unwrap(), Value::List(vec![]));
    }

    #[test]
    fn test_arrow_array_nested_list() {
        let values = vec![Value::List(vec![Value::List(vec![Value::Int64(1), Value::Int64(2)])])];
        let arr = arrow_array_from_values(&values);
        assert!(matches!(arr.data_type(), DataType::List(_)));
        let v0 = convert_arrow_scalar(&arr, 0).unwrap();
        assert_eq!(
            v0,
            Value::List(vec![Value::List(vec![Value::Int64(1), Value::Int64(2)])])
        );
    }
}
