//! Vector extension for Akar.
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

use akar_extension::{Extension, ExtensionContext};

/// The Vector extension adds embedding/vector support to Akar.
pub struct VectorExtension;

impl Default for VectorExtension {
    fn default() -> Self {
        Self::new()
    }
}

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
        use akar_common::types::Value;
        use akar_function::registry::ScalarFunction;
        use std::sync::Arc;

        // Register cosine_similarity as a CustomScalar callback
        use akar_function::registry::TableFunction;

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
fn extract_f64_list(val: &akar_common::types::Value) -> Result<Vec<f64>, String> {
    match val {
        akar_common::types::Value::List(items) => {
            let mut result = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    akar_common::types::Value::Double(d) => result.push(*d),
                    akar_common::types::Value::Int64(i) => result.push(*i as f64),
                    akar_common::types::Value::Int32(i) => result.push(*i as f64),
                    akar_common::types::Value::Float(f) => result.push(*f as f64),
                    other => {
                        return Err(format!("Expected numeric value in vector list, got {:?}", other));
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

// --------------- kNN Multi-Signal Re-ranker ---------------

/// Default weights for multi-signal re-ranking: (embedding, temporal, frequency, graph).
pub const DEFAULT_RERANK_WEIGHTS: RerankWeights = RerankWeights {
    embedding: 0.6,
    temporal: 0.15,
    frequency: 0.1,
    graph: 0.15,
};

/// Weight configuration for multi-signal re-ranking.
#[derive(Debug, Clone, Copy)]
pub struct RerankWeights {
    pub embedding: f64,
    pub temporal: f64,
    pub frequency: f64,
    pub graph: f64,
}

impl Default for RerankWeights {
    fn default() -> Self {
        DEFAULT_RERANK_WEIGHTS
    }
}

/// A candidate with all signal values for re-ranking.
#[derive(Debug, Clone)]
pub struct RerankCandidate {
    /// Node/entity ID.
    pub id: usize,
    /// Embedding similarity score (raw cosine, will be mapped to [0,1]).
    pub embedding_score: f64,
    /// Age in time units (e.g., days). 0 = just now.
    pub age: f64,
    /// Access/mention frequency. 0 = never accessed.
    pub frequency: f64,
    /// Graph proximity score. 0 = no connection, 1 = same community.
    pub graph_score: f64,
}

/// Compute combined re-ranking score from multiple signals.
///
/// Formula: `w_e*(cos+1)/2 + w_t*exp(-age*0.01) + w_f*ln(freq+1)/ln(1000) + w_g*graph`
///
/// All components are normalized to [0,1] before weighting.
pub fn compute_rerank_score(candidate: &RerankCandidate, weights: &RerankWeights) -> f64 {
    // Embedding: cosine ∈ [-1,1] → [0,1]
    let embedding = (candidate.embedding_score + 1.0) / 2.0;
    // Temporal: exponential decay, half-life ~69 time units
    let temporal = (-candidate.age * 0.01).exp();
    // Frequency: logarithmic scaling, saturates around 1000
    let frequency = if candidate.frequency > 0.0 {
        (candidate.frequency + 1.0).ln() / 1000.0_f64.ln()
    } else {
        0.0
    };
    // Graph: already in [0,1]
    let graph = candidate.graph_score.clamp(0.0, 1.0);

    weights.embedding * embedding
        + weights.temporal * temporal
        + weights.frequency * frequency
        + weights.graph * graph
}

/// Re-rank candidates by combined multi-signal score and return top-k.
///
/// Candidates are scored using `compute_rerank_score` with the given weights,
/// then sorted by descending score. Returns at most `top_k` results.
pub fn rerank_knn(
    candidates: &[RerankCandidate],
    weights: &RerankWeights,
    top_k: usize,
) -> Vec<(usize, f64)> {
    let mut scored: Vec<(usize, f64)> = candidates
        .iter()
        .map(|c| (c.id, compute_rerank_score(c, weights)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);
    scored
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

    // ── re-ranker tests ───────────────────────────────────────────────────

    #[test]
    fn test_rerank_score_basic() {
        let c = RerankCandidate {
            id: 0,
            embedding_score: 1.0, // perfect match → (1+1)/2 = 1.0
            age: 0.0,             // just now → exp(0) = 1.0
            frequency: 999.0,     // high freq → ~1.0
            graph_score: 1.0,     // same community
        };
        let score = compute_rerank_score(&c, &DEFAULT_RERANK_WEIGHTS);
        // Should be close to 1.0 (all signals maxed)
        assert!(score > 0.9, "score should be > 0.9, got {score}");
    }

    #[test]
    fn test_rerank_score_low_quality() {
        let c = RerankCandidate {
            id: 1,
            embedding_score: -1.0, // worst match → 0.0
            age: 1000.0,           // very old → ~0.0
            frequency: 0.0,        // never accessed → 0.0
            graph_score: 0.0,      // no connection
        };
        let score = compute_rerank_score(&c, &DEFAULT_RERANK_WEIGHTS);
        assert!(score < 0.1, "score should be < 0.1, got {score}");
    }

    #[test]
    fn test_rerank_top_k() {
        let candidates = vec![
            RerankCandidate { id: 0, embedding_score: 0.5, age: 100.0, frequency: 10.0, graph_score: 0.5 },
            RerankCandidate { id: 1, embedding_score: 0.9, age: 0.0, frequency: 100.0, graph_score: 0.9 },
            RerankCandidate { id: 2, embedding_score: 0.1, age: 500.0, frequency: 1.0, graph_score: 0.1 },
            RerankCandidate { id: 3, embedding_score: 0.8, age: 10.0, frequency: 50.0, graph_score: 0.7 },
        ];
        let top2 = rerank_knn(&candidates, &DEFAULT_RERANK_WEIGHTS, 2);
        assert_eq!(top2.len(), 2);
        // Best candidate (id=1) should be first
        assert_eq!(top2[0].0, 1);
        // Second best (id=3) should be second
        assert_eq!(top2[1].0, 3);
    }

    #[test]
    fn test_rerank_weight_shift() {
        // With only embedding weight, age should not matter
        let weights = RerankWeights { embedding: 1.0, temporal: 0.0, frequency: 0.0, graph: 0.0 };
        let c1 = RerankCandidate { id: 0, embedding_score: 0.8, age: 0.0, frequency: 0.0, graph_score: 0.0 };
        let c2 = RerankCandidate { id: 1, embedding_score: 0.8, age: 999.0, frequency: 0.0, graph_score: 0.0 };
        let s1 = compute_rerank_score(&c1, &weights);
        let s2 = compute_rerank_score(&c2, &weights);
        assert!((s1 - s2).abs() < 1e-10, "same embedding should give same score when temporal weight is 0");
    }
}
