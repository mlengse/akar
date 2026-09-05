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

pub mod lstm;

#[cfg(feature = "onnx-embedding")]
pub mod embed;

#[cfg(feature = "onnx-embedding")]
pub(crate) mod sbyo;

#[cfg(feature = "onnx-embedding")]
mod sparse;

#[cfg(feature = "extension")]
pub mod extension;

pub use lstm::{LstmCell, LstmConfig, LstmModel, TrainingResult, train};
