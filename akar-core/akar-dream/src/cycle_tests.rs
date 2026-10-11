//! Cycle-composition tests — the four tests proving the seven `akar-dream`
//! phase primitives compose into a full cycle.
//!
//! These lived in `akar-dream` until P127 (which moved the orchestrator out),
//! then in the `akar-server` wire-reference crate, and return here per the
//! `akar-server` retirement plan §8.1(a): the published crate whose contract
//! they prove must not lose its tests.
//!
//! The *cycle* (which phases run, in what order) is host-owned (SPEC §13), so
//! the sequencer below is deliberately **test-only** — `akar-dream` ships
//! primitives, never an orchestrator. This module exists purely to assert the
//! primitives still compose.

use crate::backend::{DreamBackend, Edge, Memory, MockBackend};
use crate::config::DreamConfig;
use crate::phases;
use crate::stats::PhaseStats;

/// Minimal test-only sequencer over the seven phase primitives.
#[derive(Debug, Clone, Default)]
struct CycleStats {
    nrem: PhaseStats,
    supersedes: PhaseStats,
    rem: PhaseStats,
    insights: PhaseStats,
    afe: PhaseStats,
    synthesis: PhaseStats,
    dae: PhaseStats,
    duration_ms: f64,
    dream_id: u64,
}

struct Cycle<B: DreamBackend> {
    config: DreamConfig,
    backend: B,
    dream_count: u64,
}

impl<B: DreamBackend> Cycle<B> {
    fn new(config: DreamConfig, backend: B) -> Self {
        Self {
            config,
            backend,
            dream_count: 0,
        }
    }

    /// Run a full cycle: NREM → SUPERSEDES → REM → Insight → AFE → Synthesis → DAE.
    fn run_cycle(&mut self) -> CycleStats {
        let start = std::time::Instant::now();
        let mut stats = CycleStats::default();
        self.dream_count += 1;
        stats.dream_id = self.dream_count;

        stats.nrem = phases::nrem::run_nrem(&self.backend, &self.config);
        stats.supersedes = phases::supersedes::run_supersedes(&self.backend);
        stats.rem = phases::rem::run_rem(
            &self.backend,
            &self.config,
            #[cfg(feature = "embed")]
            None,
        );
        stats.insights = phases::insights::run_insights(&self.backend, &self.config);
        stats.afe = phases::afe::run_afe(&self.backend);
        stats.synthesis = phases::synthesis::run_synthesis(&self.backend);
        stats.dae = phases::dae::run_dae(&self.backend);

        stats.duration_ms = start.elapsed().as_secs_f64() * 1000.0;
        stats
    }
}

#[test]
fn test_dream_cycle_default_config() {
    let backend = MockBackend::new();
    let mut cycle = Cycle::new(DreamConfig::default(), backend);
    let stats = cycle.run_cycle();
    assert_eq!(stats.dream_id, 1);
    assert!(stats.duration_ms >= 0.0);
}

#[test]
fn test_dream_cycle_incremental_dream_id() {
    let backend = MockBackend::new();
    let mut cycle = Cycle::new(DreamConfig::default(), backend);
    let s1 = cycle.run_cycle();
    let s2 = cycle.run_cycle();
    assert_eq!(s1.dream_id, 1);
    assert_eq!(s2.dream_id, 2);
}

#[test]
fn test_dream_cycle_runs_all_phases_without_data() {
    // The cycle has no phase-skipping knobs, so a cycle over an empty backend
    // still runs all seven phases and each reports the default (zero) stats.
    let backend = MockBackend::new();
    let mut cycle = Cycle::new(DreamConfig::default(), backend);
    let stats = cycle.run_cycle();
    assert_eq!(stats.dream_id, 1);
    assert!(stats.duration_ms >= 0.0);
    let touched = |p: &PhaseStats| {
        p.strengthened + p.weakened + p.pruned + p.bridges + p.insights + p.facts + p.synthesized + p.recomputed
    };
    for phase in [
        &stats.nrem,
        &stats.supersedes,
        &stats.rem,
        &stats.insights,
        &stats.afe,
        &stats.synthesis,
        &stats.dae,
    ] {
        assert_eq!(touched(phase), 0, "every phase must run: {phase:?}");
    }
}

#[test]
fn test_dream_cycle_with_mock_data() {
    let backend = MockBackend::new();
    // Add some memories and edges
    for i in 0..10 {
        backend.memories.borrow_mut().push(Memory {
            id: i,
            salience: 0.5,
            created_at: 1000.0 + i as f64,
            content: format!("memory {i}"),
        });
    }
    for i in 0..9 {
        backend.edges.borrow_mut().push(Edge {
            source_id: i,
            target_id: i + 1,
            weight: 0.5,
        });
    }

    let mut cycle = Cycle::new(DreamConfig::default(), backend);
    let stats = cycle.run_cycle();
    assert_eq!(stats.dream_id, 1);
    // NREM should have processed edges
    assert!(stats.nrem.strengthened > 0 || stats.nrem.weakened > 0 || stats.nrem.pruned > 0);
}
