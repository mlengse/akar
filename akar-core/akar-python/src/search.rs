//! PyO3 bindings for search fusion (akar-search).

use pyo3::prelude::*;
use pyo3::types::PyDict;

use akar_search::hierarchical::{apply_authority, fuse_hierarchical, AuthorityConfig, HierarchicalRrfConfig};
use akar_search::hybrid::{hybrid_search, SearchResult};
use akar_search::multi::multi_perspective_recall_with_id;
use akar_search::rrf::{rrf_fuse_owned, weighted_rrf_fuse, FusedItem, DEFAULT_K};
use std::collections::HashMap;

/// Reciprocal Rank Fusion: merge N ranked result lists.
///
/// - `sets`: list of ranked result lists, each a list of `(id: int, score: float)` tuples.
/// - `k`: RRF constant (default 60).
/// - `limit`: max results to return (default 20).
///
/// Returns list of `{id: int, score: float}` dicts sorted by descending RRF score.
#[pyfunction]
#[pyo3(signature = (sets, k=DEFAULT_K as usize, limit=20))]
fn rrf_fuse(py: Python<'_>, sets: Vec<Vec<(u64, f64)>>, k: usize, limit: usize) -> PyResult<Vec<Py<PyAny>>> {
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
        .map(|(id, score)| SearchResult {
            id,
            score,
            channel: "vector",
        })
        .collect();
    let f_res: Vec<SearchResult> = fts_results
        .into_iter()
        .map(|(id, score)| SearchResult {
            id,
            score,
            channel: "fts",
        })
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

/// Weighted RRF: fuse N ranked result lists with per-channel weights.
///
/// - `sets`: list of ranked result lists, each a list of `(id: int, score: float)` tuples.
/// - `weights`: list of floats, one per set (e.g. `[1.0, 0.8, 0.6]`).
/// - `k`: RRF constant (default 60).
/// - `limit`: max results to return (default 20).
///
/// Formula: `weight / (k + rank)` per item (rank is 1-based).
/// Returns list of `{id: int, score: float}` dicts sorted by descending weighted RRF score.
#[pyfunction]
#[pyo3(signature = (sets, weights, k=DEFAULT_K as usize, limit=20))]
fn weighted_rrf_fuse_py(
    py: Python<'_>,
    sets: Vec<Vec<(u64, f64)>>,
    weights: Vec<f64>,
    k: usize,
    limit: usize,
) -> PyResult<Vec<Py<PyAny>>> {
    if sets.len() != weights.len() {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "sets and weights must have the same length",
        ));
    }
    let input: Vec<(Vec<(u64, f64)>, f64)> = sets.into_iter().zip(weights).collect();
    let fused: Vec<FusedItem<(u64, f64)>> = weighted_rrf_fuse(input, |&(id, _)| id, k, limit);

    let mut result = Vec::with_capacity(fused.len());
    for f in fused {
        let d = PyDict::new(py);
        d.set_item("id", f.item.0)?;
        d.set_item("score", f.rrf_score)?;
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
            search_fn_ref
                .call1(py, (q,))
                .and_then(|r| r.extract(py))
                .unwrap_or_default()
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

/// Hierarchical multi-vector RRF: fuse L0 summary, L1 content and BM25 streams.
///
/// - `l0_results` / `l1_results` / `bm25_results`: ranked `(id, score)` lists,
///   best first. Any of them may be empty.
/// - `l0_weight` / `l1_weight` / `bm25_weight`: per-level RRF weights
///   (defaults `2.0`, `1.0`, `1.0` — the coarse L0 summary level leads).
/// - `k`: RRF constant (default 60).
/// - `limit`: max results to return (default 20).
///
/// Returns list of `{id: int, score: float, channel: str}` dicts sorted by
/// descending fused score. `channel` is the first level the id appeared in.
#[pyfunction]
#[pyo3(signature = (l0_results, l1_results, bm25_results, l0_weight=2.0, l1_weight=1.0, bm25_weight=1.0, k=DEFAULT_K as usize, limit=20))]
fn hierarchical_rrf(
    py: Python<'_>,
    l0_results: Vec<(u64, f64)>,
    l1_results: Vec<(u64, f64)>,
    bm25_results: Vec<(u64, f64)>,
    l0_weight: f64,
    l1_weight: f64,
    bm25_weight: f64,
    k: usize,
    limit: usize,
) -> PyResult<Vec<Py<PyAny>>> {
    let to_results = |rows: Vec<(u64, f64)>, channel: &'static str| -> Vec<SearchResult> {
        rows.into_iter()
            .map(|(id, score)| SearchResult { id, score, channel })
            .collect()
    };

    let config = HierarchicalRrfConfig {
        l0_weight,
        l1_weight,
        bm25_weight,
        rrf_k: k,
        limit,
    };
    let fused = fuse_hierarchical(
        to_results(l0_results, "vector_l0"),
        to_results(l1_results, "vector_l1"),
        to_results(bm25_results, "fts"),
        config,
    );

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

/// Re-weight a fused ranking by per-item authority, then apply the limit.
///
/// - `items`: fused `(id, score)` pairs, best first.
/// - `authorities`: `(id, authority)` pairs, authority on the `0.0..=1.0` scale.
///   Ids that are absent are treated as **neutral** (`0.5`, i.e. multiplier
///   `1.0`) — missing authority data must not demote an item.
/// - `floor_multiplier` / `ceiling_multiplier`: the multipliers at authority
///   `0.0` and `1.0` (defaults `0.5` / `1.5`).
/// - `limit`: max results to return (default 20).
///
/// The limit is applied **after** re-weighting, so `limit=1` still surfaces the
/// most trusted item rather than the best-ranked one. Returns list of
/// `{id: int, score: float}` dicts sorted by descending re-weighted score.
#[pyfunction]
#[pyo3(signature = (items, authorities, floor_multiplier=0.5, ceiling_multiplier=1.5, limit=20))]
fn apply_authority_py(
    py: Python<'_>,
    items: Vec<(u64, f64)>,
    authorities: Vec<(u64, f64)>,
    floor_multiplier: f64,
    ceiling_multiplier: f64,
    limit: usize,
) -> PyResult<Vec<Py<PyAny>>> {
    let authority_by_id: HashMap<u64, f64> = authorities.into_iter().collect();
    let fused: Vec<FusedItem<SearchResult>> = items
        .into_iter()
        .map(|(id, score)| FusedItem {
            item: SearchResult {
                id,
                score,
                channel: "fused",
            },
            rrf_score: score,
        })
        .collect();

    let config = AuthorityConfig {
        floor_multiplier,
        ceiling_multiplier,
    };
    let reweighted = apply_authority(
        fused,
        |result| authority_by_id.get(&result.id).copied().unwrap_or(0.5),
        &config,
        limit,
    );

    let mut result = Vec::with_capacity(reweighted.len());
    for f in reweighted {
        let d = PyDict::new(py);
        d.set_item("id", f.item.id)?;
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
    sub.add_function(wrap_pyfunction!(weighted_rrf_fuse_py, &sub)?)?;
    sub.add_function(wrap_pyfunction!(multi_perspective_recall, &sub)?)?;
    sub.add_function(wrap_pyfunction!(hierarchical_rrf, &sub)?)?;
    sub.add_function(wrap_pyfunction!(apply_authority_py, &sub)?)?;
    m.add_submodule(&sub)?;
    Ok(())
}
