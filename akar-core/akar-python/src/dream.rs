//! PyO3 bindings for dream engine (akar-dream).

use pyo3::prelude::*;

use akar_dream::config::DreamConfig;
use akar_dream::orchestrator::{DreamStats, PhaseStats};

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
        enable_nrem=true,
        enable_supersedes=true,
        enable_rem=true,
        enable_insights=true,
        enable_afe=true,
        enable_synthesis=true,
        enable_dae=true,
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
        enable_nrem: bool,
        enable_supersedes: bool,
        enable_rem: bool,
        enable_insights: bool,
        enable_afe: bool,
        enable_synthesis: bool,
        enable_dae: bool,
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
                enable_nrem,
                enable_supersedes,
                enable_rem,
                enable_insights,
                enable_afe,
                enable_synthesis,
                enable_dae,
            },
        }
    }

    #[getter]
    fn max_memories(&self) -> usize { self.inner.max_memories }
    #[getter]
    fn decay(&self) -> f64 { self.inner.decay }
    #[getter]
    fn threshold(&self) -> f64 { self.inner.threshold }
    #[getter]
    fn max_hops(&self) -> usize { self.inner.max_hops }
    #[getter]
    fn k_per_seed(&self) -> usize { self.inner.k_per_seed }
    #[getter]
    fn prune_threshold(&self) -> f64 { self.inner.prune_threshold }

    fn __repr__(&self) -> String {
        format!("PyDreamConfig(max_memories={}, decay={})", self.inner.max_memories, self.inner.decay)
    }
}

/// Python-visible dream phase statistics.
#[pyclass(module = "akar.dream", frozen, from_py_object)]
#[derive(Debug, Clone, Default)]
pub struct PyPhaseStats {
    inner: PhaseStats,
}

#[pymethods]
impl PyPhaseStats {
    #[getter]
    fn strengthened(&self) -> usize { self.inner.strengthened }
    #[getter]
    fn weakened(&self) -> usize { self.inner.weakened }
    #[getter]
    fn pruned(&self) -> usize { self.inner.pruned }
    #[getter]
    fn bridges(&self) -> usize { self.inner.bridges }
    #[getter]
    fn insights(&self) -> usize { self.inner.insights }
    #[getter]
    fn facts(&self) -> usize { self.inner.facts }
    #[getter]
    fn synthesized(&self) -> usize { self.inner.synthesized }
    #[getter]
    fn recomputed(&self) -> usize { self.inner.recomputed }
}

/// Python-visible dream cycle statistics.
#[pyclass(module = "akar.dream", frozen, from_py_object)]
#[derive(Debug, Clone, Default)]
pub struct PyDreamStats {
    inner: DreamStats,
}

#[pymethods]
impl PyDreamStats {
    #[getter]
    fn nrem(&self) -> PyPhaseStats {
        PyPhaseStats { inner: self.inner.nrem.clone() }
    }
    #[getter]
    fn supersedes(&self) -> PyPhaseStats {
        PyPhaseStats { inner: self.inner.supersedes.clone() }
    }
    #[getter]
    fn rem(&self) -> PyPhaseStats {
        PyPhaseStats { inner: self.inner.rem.clone() }
    }
    #[getter]
    fn insights(&self) -> PyPhaseStats {
        PyPhaseStats { inner: self.inner.insights.clone() }
    }
    #[getter]
    fn afe(&self) -> PyPhaseStats {
        PyPhaseStats { inner: self.inner.afe.clone() }
    }
    #[getter]
    fn synthesis(&self) -> PyPhaseStats {
        PyPhaseStats { inner: self.inner.synthesis.clone() }
    }
    #[getter]
    fn dae(&self) -> PyPhaseStats {
        PyPhaseStats { inner: self.inner.dae.clone() }
    }
    #[getter]
    fn duration_ms(&self) -> f64 { self.inner.duration_ms }
    #[getter]
    fn dream_id(&self) -> u64 { self.inner.dream_id }

    fn __repr__(&self) -> String {
        format!("PyDreamStats(dream_id={})", self.inner.dream_id)
    }
}

/// Register this submodule on the parent `akar` module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "dream")?;
    sub.add_class::<PyDreamConfig>()?;
    sub.add_class::<PyPhaseStats>()?;
    sub.add_class::<PyDreamStats>()?;
    m.add_submodule(&sub)?;
    Ok(())
}
