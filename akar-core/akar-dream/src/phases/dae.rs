//! DAE phase: recompute denoising autoencoder embeddings.

use crate::backend::DreamBackend;
use crate::orchestrator::PhaseStats;

pub fn run_dae<B: DreamBackend>(backend: &B) -> PhaseStats {
    let mut stats = PhaseStats::default();
    stats.recomputed = backend.recompute_dae();
    stats
}
