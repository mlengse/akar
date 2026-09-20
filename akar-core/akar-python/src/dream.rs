//! PyO3 bindings for the dream engine's primitive configuration.
//!
//! Only the tuning struct is exposed: the dream *cycle* is host-owned (SPEC §13),
//! so Akar publishes no cycle-level result types to Python.

use pyo3::prelude::*;

use akar_dream::config::DreamConfig;

/// Python-visible dream configuration.
#[pyclass(module = "akar.dream", frozen, from_py_object)]
#[derive(Debug, Clone)]
pub struct PyDreamConfig {
    inner: DreamConfig,
}

#[pymethods]
impl PyDreamConfig {
    #[new]
    #[pyo3(signature = (
        max_memories=200,
        sample_recent_pct=0.6,
        sample_random_old_pct=0.2,
        sample_low_salience_pct=0.2,
        decay=0.85,
        threshold=0.01,
        max_hops=1,
        k_per_seed=20,
        prune_threshold=0.01,
        insight_min_community_size=3,
        louvain_resolution=1.0,
        max_bridge_nodes=10,
        nrem_weaken_rate=0.05,
    ))]
    fn new(
        max_memories: usize,
        sample_recent_pct: f64,
        sample_random_old_pct: f64,
        sample_low_salience_pct: f64,
        decay: f64,
        threshold: f64,
        max_hops: usize,
        k_per_seed: usize,
        prune_threshold: f64,
        insight_min_community_size: usize,
        louvain_resolution: f64,
        max_bridge_nodes: usize,
        nrem_weaken_rate: f64,
    ) -> Self {
        Self {
            inner: DreamConfig {
                max_memories,
                sample_recent_pct,
                sample_random_old_pct,
                sample_low_salience_pct,
                decay,
                threshold,
                max_hops,
                k_per_seed,
                prune_threshold,
                insight_min_community_size,
                louvain_resolution,
                max_bridge_nodes,
                nrem_weaken_rate,
            },
        }
    }

    #[getter]
    fn max_memories(&self) -> usize {
        self.inner.max_memories
    }
    #[getter]
    fn decay(&self) -> f64 {
        self.inner.decay
    }
    #[getter]
    fn threshold(&self) -> f64 {
        self.inner.threshold
    }
    #[getter]
    fn max_hops(&self) -> usize {
        self.inner.max_hops
    }
    #[getter]
    fn k_per_seed(&self) -> usize {
        self.inner.k_per_seed
    }
    #[getter]
    fn prune_threshold(&self) -> f64 {
        self.inner.prune_threshold
    }
    /// Base NREM decay per cycle; retention scoring only ever reduces it (P119.2).
    #[getter]
    fn nrem_weaken_rate(&self) -> f64 {
        self.inner.nrem_weaken_rate
    }

    fn __repr__(&self) -> String {
        format!(
            "PyDreamConfig(max_memories={}, decay={})",
            self.inner.max_memories, self.inner.decay
        )
    }
}

/// Register this submodule on the parent `akar` module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "dream")?;
    sub.add_class::<PyDreamConfig>()?;
    m.add_submodule(&sub)?;
    Ok(())
}
