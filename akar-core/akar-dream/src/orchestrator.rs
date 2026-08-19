//! Dream engine orchestrator.

use crate::backend::DreamBackend;
use crate::config::DreamConfig;
use crate::phases;

/// Statistics from a full dream cycle.
#[derive(Debug, Clone, Default)]
pub struct DreamStats {
    pub nrem: PhaseStats,
    pub supersedes: PhaseStats,
    pub rem: PhaseStats,
    pub insights: PhaseStats,
    pub afe: PhaseStats,
    pub synthesis: PhaseStats,
    pub dae: PhaseStats,
    pub duration_ms: f64,
    pub dream_id: u64,
}

/// Statistics for a single phase.
#[derive(Debug, Clone, Default)]
pub struct PhaseStats {
    pub strengthened: usize,
    pub weakened: usize,
    pub pruned: usize,
    pub bridges: usize,
    pub insights: usize,
    pub facts: usize,
    pub synthesized: usize,
    pub recomputed: usize,
}

/// The dream orchestrator runs the full consolidation cycle.
pub struct DreamOrchestrator<B: DreamBackend> {
    config: DreamConfig,
    backend: B,
    dream_count: u64,
}

impl<B: DreamBackend> DreamOrchestrator<B> {
    pub fn new(config: DreamConfig, backend: B) -> Self {
        Self {
            config,
            backend,
            dream_count: 0,
        }
    }

    /// Run a full dream cycle: NREM → SUPERSEDES → REM → Insight → AFE → Synthesis → DAE.
    pub fn run_cycle(&mut self) -> DreamStats {
        let start = std::time::Instant::now();
        let mut stats = DreamStats::default();
        self.dream_count += 1;
        stats.dream_id = self.dream_count;

        // NREM
        if self.config.enable_nrem {
            stats.nrem = phases::nrem::run_nrem(&self.backend, &self.config);
        }

        // SUPERSEDES
        if self.config.enable_supersedes {
            stats.supersedes = phases::supersedes::run_supersedes(&self.backend);
        }

        // REM
        if self.config.enable_rem {
            stats.rem = phases::rem::run_rem(&self.backend, &self.config);
        }

        // INSIGHT
        if self.config.enable_insights {
            stats.insights = phases::insights::run_insights(&self.backend, &self.config);
        }

        // AFE
        if self.config.enable_afe {
            stats.afe = phases::afe::run_afe(&self.backend);
        }

        // SYNTHESIS
        if self.config.enable_synthesis {
            stats.synthesis = phases::synthesis::run_synthesis(&self.backend);
        }

        // DAE
        if self.config.enable_dae {
            stats.dae = phases::dae::run_dae(&self.backend);
        }

        stats.duration_ms = start.elapsed().as_secs_f64() * 1000.0;
        stats
    }

    pub fn dream_count(&self) -> u64 {
        self.dream_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;

    #[test]
    fn test_orchestrator_default_config() {
        let backend = MockBackend::new();
        let mut orchestrator = DreamOrchestrator::new(DreamConfig::default(), backend);
        let stats = orchestrator.run_cycle();
        assert_eq!(stats.dream_id, 1);
        assert!(stats.duration_ms >= 0.0);
    }

    #[test]
    fn test_orchestrator_incremental_dream_id() {
        let backend = MockBackend::new();
        let mut orchestrator = DreamOrchestrator::new(DreamConfig::default(), backend);
        let s1 = orchestrator.run_cycle();
        let s2 = orchestrator.run_cycle();
        assert_eq!(s1.dream_id, 1);
        assert_eq!(s2.dream_id, 2);
    }

    #[test]
    fn test_orchestrator_skip_phases() {
        let backend = MockBackend::new();
        let cfg = DreamConfig {
            enable_nrem: false,
            enable_rem: false,
            enable_insights: false,
            ..Default::default()
        };
        let mut orchestrator = DreamOrchestrator::new(cfg, backend);
        let stats = orchestrator.run_cycle();
        // Skipped phases have default (zero) stats
        assert_eq!(stats.nrem.strengthened, 0);
        assert_eq!(stats.rem.bridges, 0);
    }

    #[test]
    fn test_orchestrator_with_mock_data() {
        let backend = MockBackend::new();
        // Add some memories and edges
        for i in 0..10 {
            backend.memories.borrow_mut().push(crate::backend::Memory {
                id: i,
                salience: 0.5,
                created_at: 1000.0 + i as f64,
                content: format!("memory {i}"),
            });
        }
        for i in 0..9 {
            backend.edges.borrow_mut().push(crate::backend::Edge {
                source_id: i,
                target_id: i + 1,
                weight: 0.5,
            });
        }

        let mut orchestrator = DreamOrchestrator::new(DreamConfig::default(), backend);
        let stats = orchestrator.run_cycle();
        assert_eq!(stats.dream_id, 1);
        // NREM should have processed edges
        assert!(stats.nrem.strengthened > 0 || stats.nrem.weakened > 0 || stats.nrem.pruned > 0);
    }
}
