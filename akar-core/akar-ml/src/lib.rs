//! Local ML inference and training for Akar.
//!
//! Provides a 1-layer LSTM implementation for sequence prediction,
//! fully self-contained (pure Rust, no FFI).
//!
//! # Components
//!
//! - `LstmCell` — single LSTM gate computation (forget/input/candidate/output)
//! - `LstmModel` — 1-layer LSTM with input→hidden→output projection
//! - `train` — BPTT training loop with configurable epochs/learning rate
//! - `save_model` / `load_model` — serde JSON serialization
//!
//! # Embeddings (ONNX, feature `onnx-embedding`)
//!
//! In-process text embedding over ONNX Runtime (see [`embed`]):
//! [`embed::FastEmbedProvider`] (dense), [`embed::SparseEmbedProvider`],
//! [`embed::Bgem3Provider`] (dense + sparse + ColBERT), and
//! [`embed::RerankProvider`]. Sessions build lazily on first inference.
//!
//! ## DirectML GPU on Windows (feature `directml`)
//!
//! Enable the additive `directml` Cargo feature and pass
//! [`embed::directml_execution_provider`] into a provider config's
//! `execution_providers` list to prefer the DirectML (DirectX 12) GPU
//! provider. Honor known constraints (handled automatically):
//!
//! - DirectML disables ORT's memory-pattern optimization and parallel
//!   execution on the session — fastembed applies
//!   `with_memory_pattern(false)` + `with_parallel_execution(false)` when it
//!   detects a DirectML EP (`fastembed/src/common.rs:init_session_builder`).
//! - No GPU present: EP registration fails at session build. Use
//!   `ort::ep::ExecutionProviderDispatch::fail_silently()` to keep ORT's CPU
//!   provider as the fallback; ops the DirectML provider cannot place fall
//!   back to CPU per-node by default (`disable_cpu_fallback` is not yet
//!   exposed by akar configs).
//! - The SBYO sparse offline path ([`embed::SparseEmbedProvider`]
//!   `try_from_user_defined` / `new_from_dir`) builds its own native ort
//!   session that takes no execution providers and always runs on CPU.
//!
//! ## Bundled model assets, air-gapped (feature `bundle-default-models`)
//!
//! Ships one or more lightweight ONNX + tokenizer models with the crate so
//! offline deployments never touch the network. The per-model layout is the
//! canonical `models/<name>/` directory — `model.onnx`, `tokenizer.json`,
//! `config.json`, `special_tokens_map.json`, `tokenizer_config.json` — see
//! `akar-ml/models/README.md`. At build time `build.rs` copies the git-ignored
//! staging tree `models/.staging/<name>/` into `$OUT_DIR/assets/<name>/`
//! (idempotent, no network); the runtime accessor is
//! [`assets::bundled_model_dir`]. No staged assets at build time degrades to
//! `None` (the feature is additive; the default gate is unaffected).

pub mod lstm;

#[cfg(feature = "onnx-embedding")]
pub mod embed;

// Re-exported so consumers can name embedding output types and the capability
// traits (P96) from the crate root: `akar_ml::{MultiEmbeddingProvider, ...}`.
#[cfg(feature = "onnx-embedding")]
pub use embed::{EmbeddingProvider, MultiEmbeddingOutput, MultiEmbeddingProvider, RerankerProvider, SparseEmbedding};

#[cfg(feature = "onnx-embedding")]
pub(crate) mod sbyo;

#[cfg(feature = "bundle-default-models")]
pub mod assets;

#[cfg(feature = "onnx-embedding")]
mod sparse;

#[cfg(feature = "extension")]
pub mod extension;

pub use lstm::{LstmCell, LstmConfig, LstmModel, TrainingResult, train};
