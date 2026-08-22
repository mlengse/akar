//! DAE phase: recompute denoising autoencoder embeddings.

use crate::backend::DreamBackend;
use crate::orchestrator::PhaseStats;

pub fn run_dae<B: DreamBackend>(backend: &B) -> PhaseStats {
    PhaseStats {
        recomputed: backend.recompute_dae(),
        ..PhaseStats::default()
    }
}
