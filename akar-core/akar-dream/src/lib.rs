//! Dream phase compute primitives for Akar.
//!
//! This crate holds the dream phase **compute primitives** and the storage port
//! ([`DreamBackend`]) they run over. The *cycle* — which phases run, in what
//! order, when to trigger one, and how pause/resume behaves — is **host-owned**:
//! `akar-server` sequences it as the wire reference and `sulur-server` will own
//! it in production, per SPEC §13.

pub mod backend;
pub mod config;
pub mod phases;
pub mod stats;

#[cfg(feature = "embed")]
pub use akar_ml::embed::{
    EmbeddingProvider, MultiEmbeddingOutput, MultiEmbeddingProvider, RerankResult, RerankerProvider, SparseEmbedding,
};

pub use backend::{DreamBackend, Edge, Memory};
pub use config::DreamConfig;
pub use stats::PhaseStats;
