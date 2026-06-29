//! Vector extension for Kuzu.
//!
//! Provides vector similarity search operations and an HNSW index for
//! approximate nearest neighbour search.
//!
//! # Functions
//!
//! - `cosine_similarity(a, b)` — cosine similarity between two vectors
//! - `euclidean_distance(a, b)` — Euclidean distance between two vectors
//! - `dot_product(a, b)` — dot product of two vectors
//! - `l2_distance(a, b)` — L2 squared distance between two vectors
//!
//! # Index
//!
//! `HnswIndex` — multi-layer navigable small world graph for fast ANN search.
//! Supports configurable distance metrics (Cosine, Euclidean, L1, L2, Dot).

pub mod hnsw;

use kuzu_extension::{Extension, ExtensionContext};

/// The Vector extension adds embedding/vector support to Kuzu.
pub struct VectorExtension;

impl VectorExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for VectorExtension {
    fn name(&self) -> &'static str {
        "VECTOR"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use kuzu_function::registry::ScalarFunction;
        use kuzu_common::types::Value;
        use std::sync::Arc;

        // Register cosine_similarity as a CustomScalar callback
        use kuzu_function::registry::TableFunction;

        // Register vector_similarity_scan as a table function (handled by processor)
        context.register_table_function(
            "vector_similarity_scan",
            TableFunction::Custom {
                name: "vector_similarity_scan".into(),
            },
        );

        context.register_scalar_function(
            "cosine_similarity",
            ScalarFunction::CustomScalar {
                name: "cosine_similarity".into(),
                execute: Arc::new(|args: &[Value]| {
                    if args.len() < 2 {
                        return Err("cosine_similarity requires 2 arguments".into());
                    }
                    let a = extract_f64_list(&args[0])?;
                    let b = extract_f64_list(&args[1])?;
                    let result = crate::cosine_similarity(&a, &b);
                    Ok(Value::Double(result))
                }),
            },
        );

        context.register_scalar_function(
            "euclidean_distance",
            ScalarFunction::CustomScalar {
                name: "euclidean_distance".into(),
                execute: Arc::new(|args: &[Value]| {
                    if args.len() < 2 {
                        return Err("euclidean_distance requires 2 arguments".into());
                    }
                    let a = extract_f64_list(&args[0])?;
                    let b = extract_f64_list(&args[1])?;
                    let result = crate::euclidean_distance(&a, &b);
                    Ok(Value::Double(result))
                }),
            },
        );

        context.register_scalar_function(
            "dot_product",
            ScalarFunction::CustomScalar {
                name: "dot_product".into(),
                execute: Arc::new(|args: &[Value]| {
                    if args.len() < 2 {
                        return Err("dot_product requires 2 arguments".into());
                    }
                    let a = extract_f64_list(&args[0])?;
                    let b = extract_f64_list(&args[1])?;
                    let result = crate::dot_product(&a, &b);
                    Ok(Value::Double(result))
                }),
            },
        );

        context.register_scalar_function(
            "l2_distance",
            ScalarFunction::CustomScalar {
                name: "l2_distance".into(),
                execute: Arc::new(|args: &[Value]| {
                    if args.len() < 2 {
                        return Err("l2_distance requires 2 arguments".into());
                    }
                    let a = extract_f64_list(&args[0])?;
                    let b = extract_f64_list(&args[1])?;
                    let result = crate::l2_distance(&a, &b);
                    Ok(Value::Double(result))
                }),
            },
        );

        tracing::info!("Vector extension loaded: 5 functions registered (4 scalar + 1 table)");
        Ok(())
    }
}

/// Helper: extract a `Vec<f64>` from a `Value` (expects `Value::List` of numbers).
fn extract_f64_list(val: &kuzu_common::types::Value) -> Result<Vec<f64>, String> {
    match val {
        kuzu_common::types::Value::List(items) => {
            let mut result = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    kuzu_common::types::Value::Double(d) => result.push(*d),
                    kuzu_common::types::Value::Int64(i) => result.push(*i as f64),
                    kuzu_common::types::Value::Int32(i) => result.push(*i as f64),
                    kuzu_common::types::Value::Float(f) => result.push(*f as f64),
                    other => {
                        return Err(format!(
                            "Expected numeric value in vector list, got {:?}",
                            other
                        ));
                    }
                }
            }
            Ok(result)
        }
        other => Err(format!("Expected List value for vector, got {:?}", other)),
    }
}

/// Compute cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Compute Euclidean distance between two vectors.
pub fn euclidean_distance(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f64>()
        .sqrt()
}

/// Compute dot product of two vectors.
pub fn dot_product(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Compute L2 squared distance (squared Euclidean).
pub fn l2_distance(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum()
}

/// Normalize a vector to unit length.
pub fn normalize(v: &[f64]) -> Vec<f64> {
    let norm: f64 = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_euclidean_distance() {
        let a = vec![0.0, 0.0];
        let b = vec![3.0, 4.0];
        assert!((euclidean_distance(&a, &b) - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_dot_product() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        assert!((dot_product(&a, &b) - 32.0).abs() < 1e-10);
    }

    #[test]
    fn test_l2_distance() {
        let a = vec![0.0, 0.0];
        let b = vec![3.0, 4.0];
        assert!((l2_distance(&a, &b) - 25.0).abs() < 1e-10);
    }

    #[test]
    fn test_normalize() {
        let v = vec![3.0, 4.0];
        let n = normalize(&v);
        assert!((n[0] - 0.6).abs() < 1e-10);
        assert!((n[1] - 0.8).abs() < 1e-10);
    }

    #[test]
    fn test_cosine_different_lengths() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_empty_vectors() {
        assert!((euclidean_distance(&[], &[]) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_vector_extension_name() {
        let ext = VectorExtension::new();
        assert_eq!(ext.name(), "VECTOR");
    }

    #[test]
    fn test_dot_product_negative() {
        let a = vec![1.0, -1.0];
        let b = vec![-1.0, 1.0];
        assert!((dot_product(&a, &b) + 2.0).abs() < 1e-10);
    }
}
