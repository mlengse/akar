//! PyO3 bindings for search fusion (akar-search).

use pyo3::prelude::*;
use pyo3::types::PyDict;

use akar_search::rrf::{rrf_fuse_owned, FusedItem, DEFAULT_K};
use akar_search::hybrid::{hybrid_search, SearchResult};
use akar_search::multi::multi_perspective_recall_with_id;

/// Reciprocal Rank Fusion: merge N ranked result lists.
///
/// - `sets`: list of ranked result lists, each a list of `(id: int, score: float)` tuples.
/// - `k`: RRF constant (default 60).
/// - `limit`: max results to return (default 20).
///
/// Returns list of `{id: int, score: float}` dicts sorted by descending RRF score.
#[pyfunction]
#[pyo3(signature = (sets, k=DEFAULT_K as usize, limit=20))]
fn rrf_fuse(
    py: Python<'_>,
    sets: Vec<Vec<(u64, f64)>>,
    k: usize,
    limit: usize,
) -> PyResult<Vec<Py<PyAny>>> {
    let fused: Vec<FusedItem<(u64, f64)>> = rrf_fuse_owned(sets, |&(id, _)| id, k, limit);

    let mut result = Vec::with_capacity(fused.len());
    for f in fused {
        let d = PyDict::new(py);
        d.set_item("id", f.item.0)?;
        d.set_item("score", f.rrf_score)?;
        result.push(d.unbind().into_any());
    }
    Ok(result)
}

/// Hybrid search: fuse vector results and FTS results via RRF.
///
/// - `vector_results`: list of `(id, score)` tuples from vector search.
/// - `fts_results`: list of `(id, score)` tuples from full-text search.
/// - `limit`: max results to return (default 20).
///
/// Returns list of `{id: int, score: float, channel: str}` dicts sorted by descending RRF score.
#[pyfunction]
#[pyo3(signature = (vector_results, fts_results, limit=20))]
fn hybrid_search_py(
    py: Python<'_>,
    vector_results: Vec<(u64, f64)>,
    fts_results: Vec<(u64, f64)>,
    limit: usize,
) -> PyResult<Vec<Py<PyAny>>> {
    let v_res: Vec<SearchResult> = vector_results
        .into_iter()
        .map(|(id, score)| SearchResult { id, score, channel: "vector" })
        .collect();
    let f_res: Vec<SearchResult> = fts_results
        .into_iter()
        .map(|(id, score)| SearchResult { id, score, channel: "fts" })
        .collect();

    let fused = hybrid_search(v_res, f_res, limit);

    let mut result = Vec::with_capacity(fused.len());
    for f in fused {
        let d = PyDict::new(py);
        d.set_item("id", f.item.id)?;
        d.set_item("score", f.rrf_score)?;
        d.set_item("channel", f.item.channel)?;
        result.push(d.unbind().into_any());
    }
    Ok(result)
}

/// Multi-perspective recall: run N search queries and fuse results via RRF.
///
/// - `queries`: list of query strings.
/// - `search_fn`: a callable that takes a query string and returns a list of `(id, score)` tuples.
/// - `k`: RRF constant (default 60).
/// - `limit`: max results to return (default 20).
///
/// Returns list of `{id: int, score: float}` dicts sorted by descending RRF score.
#[pyfunction]
#[pyo3(signature = (queries, search_fn, k=DEFAULT_K as usize, limit=20))]
fn multi_perspective_recall(
    py: Python<'_>,
    queries: Vec<String>,
    search_fn: Py<PyAny>,
    k: usize,
    limit: usize,
) -> PyResult<Vec<Py<PyAny>>> {
    let search_fn_ref = &search_fn;
    let borrowed: Vec<&str> = queries.iter().map(|s| s.as_str()).collect();
    let fused: Vec<FusedItem<(u64, f64)>> = multi_perspective_recall_with_id(
        &borrowed,
        |q: &str| -> Vec<(u64, f64)> {
            search_fn_ref.call1(py, (q,)).and_then(|r| r.extract(py)).unwrap_or_default()
        },
        |&(id, _)| id,
        k,
        limit,
    );

    let mut result = Vec::with_capacity(fused.len());
    for f in fused {
        let d = PyDict::new(py);
        d.set_item("id", f.item.0)?;
        d.set_item("score", f.rrf_score)?;
        result.push(d.unbind().into_any());
    }
    Ok(result)
}

/// Register this submodule on the parent `akar` module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "search")?;
    sub.add_function(wrap_pyfunction!(rrf_fuse, &sub)?)?;
    sub.add_function(wrap_pyfunction!(hybrid_search_py, &sub)?)?;
    sub.add_function(wrap_pyfunction!(multi_perspective_recall, &sub)?)?;
    m.add_submodule(&sub)?;
    Ok(())
}
