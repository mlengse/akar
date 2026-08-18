//! PyO3 bindings for kNN / vector similarity (akar-vector).

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Cosine similarity between two vectors.
#[pyfunction]
fn cosine_similarity(a: Vec<f64>, b: Vec<f64>) -> PyResult<f64> {
    if a.len() != b.len() {
        return Err(PyValueError::new_err(format!(
            "vector dimensions differ: {} vs {}",
            a.len(),
            b.len(),
        )));
    }
    Ok(akar_vector::cosine_similarity(&a, &b))
}

/// Euclidean distance between two vectors.
#[pyfunction]
fn euclidean_distance(a: Vec<f64>, b: Vec<f64>) -> PyResult<f64> {
    Ok(akar_vector::euclidean_distance(&a, &b))
}

/// Dot product of two vectors.
#[pyfunction]
fn dot_product(a: Vec<f64>, b: Vec<f64>) -> PyResult<f64> {
    Ok(akar_vector::dot_product(&a, &b))
}

/// L2 (euclidean) distance between two vectors.
#[pyfunction]
fn l2_distance(a: Vec<f64>, b: Vec<f64>) -> PyResult<f64> {
    Ok(akar_vector::l2_distance(&a, &b))
}

/// Normalize a vector to unit length.
#[pyfunction]
fn normalize(v: Vec<f64>) -> PyResult<Vec<f64>> {
    Ok(akar_vector::normalize(&v))
}

/// Brute-force kNN search over a set of vectors.
///
/// Returns `k` closest `(index, score)` pairs sorted by descending score.
/// `metric`: "cosine" (default), "euclidean", "l2", "dot".
#[pyfunction]
#[pyo3(signature = (vectors, query, k, metric="cosine"))]
fn knn_search(
    vectors: Vec<Vec<f64>>,
    query: Vec<f64>,
    k: usize,
    metric: &str,
) -> PyResult<Vec<(usize, f64)>> {
    if vectors.is_empty() {
        return Ok(Vec::new());
    }
    let dim = vectors[0].len();
    if query.len() != dim {
        return Err(PyValueError::new_err(format!(
            "query dimension {} != vector dimension {}",
            query.len(),
            dim,
        )));
    }

    let score_fn: Box<dyn Fn(&[f64], &[f64]) -> f64> = match metric {
        "cosine" => Box::new(|a, b| akar_vector::cosine_similarity(a, b)),
        "euclidean" | "l2" => {
            Box::new(|a, b| -akar_vector::euclidean_distance(a, b))
        }
        "dot" => Box::new(|a, b| akar_vector::dot_product(a, b)),
        _ => {
            return Err(PyValueError::new_err(format!(
                "Unknown metric: {metric}. Use 'cosine', 'euclidean', 'l2', or 'dot'."
            )));
        }
    };

    let mut scores: Vec<(usize, f64)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (i, score_fn(&query, v)))
        .collect();

    // Partial sort for top-k
    if k < scores.len() {
        scores.select_nth_unstable_by(k - 1, |a, b| b.1.partial_cmp(&a.1).unwrap());
        scores.truncate(k);
    }
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    Ok(scores)
}

/// Register this submodule on the parent `akar` module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "knn")?;
    sub.add_function(wrap_pyfunction!(cosine_similarity, &sub)?)?;
    sub.add_function(wrap_pyfunction!(euclidean_distance, &sub)?)?;
    sub.add_function(wrap_pyfunction!(dot_product, &sub)?)?;
    sub.add_function(wrap_pyfunction!(l2_distance, &sub)?)?;
    sub.add_function(wrap_pyfunction!(normalize, &sub)?)?;
    sub.add_function(wrap_pyfunction!(knn_search, &sub)?)?;
    m.add_submodule(&sub)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_basic() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity(a, b).unwrap()).abs() < 1e-10);
    }

    #[test]
    fn test_knn_search_basic() {
        let vectors = vec![
            vec![1.0, 0.0],
            vec![0.0, 1.0],
            vec![1.0, 1.0],
        ];
        let query = vec![1.0, 0.0];
        let result = knn_search(vectors, query, 2, "cosine").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, 0); // index 0 is closest
    }

    #[test]
    fn test_knn_search_euclidean() {
        let vectors = vec![
            vec![0.0, 0.0],
            vec![1.0, 0.0],
            vec![10.0, 10.0],
        ];
        let query = vec![0.0, 0.0];
        let result = knn_search(vectors, query, 1, "euclidean").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 0);
    }

    #[test]
    fn test_normalize_unit_length() {
        let v = vec![3.0, 4.0];
        let n = normalize(v).unwrap();
        let len: f64 = n.iter().map(|x| x * x).sum::<f64>().sqrt();
        assert!((len - 1.0).abs() < 1e-10);
    }
}
