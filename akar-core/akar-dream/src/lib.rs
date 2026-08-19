//! Dream engine orchestration for Akar.
//!
//! Implements the NREM → SUPERSEDES → REM → Insight → AFE → Synthesis → DAE
//! cycle for memory consolidation. All computation happens in Rust; Python
//! only calls `DreamOrchestrator::run_cycle()`.

pub mod config;
pub mod backend;
pub mod orchestrator;
pub mod phases;

pub use config::DreamConfig;
pub use backend::DreamBackend;
pub use orchestrator::{DreamOrchestrator, DreamStats};
