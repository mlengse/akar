use crate::selection::SelectionVector;
use crate::types::{PhysicalTypeID, Value};
use crate::vector::ValueVector;
use arrow::array::{
    Array, ArrayRef, BooleanArray, Float32Array, Float64Array, Int32Array, Int64Array,
};
use arrow::datatypes::DataType;
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
        Self {
            array,
            physical_type,
        }
    }

    pub fn from_legacy(vec: &ValueVector) -> Self {
        let phys_type = vec.physical_type();
        let size = vec.size();
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
            PhysicalTypeID::Int64 => {
                let mut builder = arrow::array::Int64Builder::with_capacity(size);
                for i in 0..size {
                    if vec.is_null(i) {
                        builder.append_null();
                    } else {
                        builder.append_value(vec.get_i64(i).unwrap_or(0));
                    }
                }
                Arc::new(builder.finish())
            }
            PhysicalTypeID::Int32 => {
                let mut builder = arrow::array::Int32Builder::with_capacity(size);
                for i in 0..size {
                    if vec.is_null(i) {
                        builder.append_null();
                    } else {
                        builder.append_value(vec.get_i32(i).unwrap_or(0));
                    }
                }
                Arc::new(builder.finish())
            }
            PhysicalTypeID::Double => {
                let mut builder = arrow::array::Float64Builder::with_capacity(size);
                for i in 0..size {
                    if vec.is_null(i) {
                        builder.append_null();
                    } else {
                        builder.append_value(vec.get_double(i).unwrap_or(0.0));
                    }
                }
                Arc::new(builder.finish())
            }
            PhysicalTypeID::Float => {
                let mut builder = arrow::array::Float32Builder::with_capacity(size);
                for i in 0..size {
                    if vec.is_null(i) {
                        builder.append_null();
                    } else {
                        let val = vec.get_double(i).unwrap_or(0.0) as f32;
                        builder.append_value(val);
                    }
                }
                Arc::new(builder.finish())
            }
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
    fn size(&self) -> usize {
        self.array.len()
    }

    fn physical_type(&self) -> PhysicalTypeID {
        self.physical_type
    }

    fn is_null(&self, row: usize) -> bool {
        if row >= self.array.len() {
            return true;
        }
        self.array.is_null(row)
    }

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

    fn get_value(&self, row: usize) -> Option<Value> {
        if row >= self.array.len() || self.array.is_null(row) {
            return None;
        }
        match self.physical_type {
            PhysicalTypeID::Bool => {
                self.get_bool(row).map(Value::Bool)
            }
            PhysicalTypeID::Int64 => {
                self.get_i64(row).map(Value::Int64)
            }
            PhysicalTypeID::Int32 => {
                self.get_i32(row).map(Value::Int32)
            }
            PhysicalTypeID::Double => {
                self.get_f64(row).map(Value::Double)
            }
            PhysicalTypeID::Float => {
                self.get_f32(row).map(Value::Float)
            }
            PhysicalTypeID::String => {
                let array = self.array.as_any().downcast_ref::<arrow::array::StringArray>()?;
                Some(Value::String(array.value(row).to_string()))
            }
            _ => {
                // For unsupported types, return null
                None
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum Vector {
    Arrow(ArrowVector),
    Legacy(ValueVector),
}

impl Vector {
    pub fn as_arrow(&self) -> Option<&ArrowVector> {
        match self {
            Vector::Arrow(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_legacy(&self) -> Option<&ValueVector> {
        match self {
            Vector::Legacy(l) => Some(l),
            _ => None,
        }
    }

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
}
