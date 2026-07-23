use arrow::array::{Int64Array, ArrayRef};
use arrow::compute::{add, eq};
use std::sync::Arc;

fn main() {
    let a = Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef;
    let b = Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef;
    let _ = add(&a, &b).unwrap();
    let _ = eq(&a, &b).unwrap();
    println!("OK");
}
