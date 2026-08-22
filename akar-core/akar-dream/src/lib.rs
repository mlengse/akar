//! Dream engine orchestration for Akar.
//!
//! Implements the NREM → SUPERSEDES → REM → Insight → AFE → Synthesis → DAE
//! cycle for memory consolidation. All computation happens in Rust; Python
//! only calls `DreamOrchestrator::run_cycle()`.

pub mod backend;
pub mod config;
pub mod orchestrator;
pub mod phases;

pub use backend::DreamBackend;
pub use config::DreamConfig;
pub use orchestrator::{DreamOrchestrator, DreamStats};
