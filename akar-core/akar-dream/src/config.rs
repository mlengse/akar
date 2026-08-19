//! Configuration for the dream engine.

/// Configuration for a dream cycle.
#[derive(Debug, Clone)]
pub struct DreamConfig {
    /// Maximum number of memories to sample for NREM phase.
    pub max_memories: usize,
    /// Fraction of recent memories to sample.
    pub sample_recent_pct: f64,
    /// Fraction of random old memories to sample.
    pub sample_random_old_pct: f64,
    /// Fraction of low-salience memories to sample.
    pub sample_low_salience_pct: f64,
    /// Activation decay per hop (0.0..1.0).
    pub decay: f64,
    /// Minimum activation threshold to propagate.
    pub threshold: f64,
    /// Maximum BFS hops for spread activation.
    pub max_hops: usize,
    /// Maximum results per seed in batch spread activation.
    pub k_per_seed: usize,
    /// Edge weight below which to prune.
    pub prune_threshold: f64,
    /// Minimum community size for insight phase.
    pub insight_min_community_size: usize,
    /// Louvain resolution parameter.
    pub louvain_resolution: f64,
    /// Maximum bridge nodes to discover in REM phase.
    pub max_bridge_nodes: usize,
    /// Whether to enable each phase.
    pub enable_nrem: bool,
    pub enable_supersedes: bool,
    pub enable_rem: bool,
    pub enable_insights: bool,
    pub enable_afe: bool,
    pub enable_synthesis: bool,
    pub enable_dae: bool,
}

impl Default for DreamConfig {
    fn default() -> Self {
        Self {
            max_memories: 200,
            sample_recent_pct: 0.6,
            sample_random_old_pct: 0.2,
            sample_low_salience_pct: 0.2,
            decay: 0.85,
            threshold: 0.01,
            max_hops: 1,
            k_per_seed: 20,
            prune_threshold: 0.005,
            insight_min_community_size: 3,
            louvain_resolution: 1.0,
            max_bridge_nodes: 50,
            enable_nrem: true,
            enable_supersedes: true,
            enable_rem: true,
            enable_insights: true,
            enable_afe: true,
            enable_synthesis: true,
            enable_dae: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let cfg = DreamConfig::default();
        assert_eq!(cfg.max_memories, 200);
        assert!((cfg.decay - 0.85).abs() < 1e-10);
        assert!((cfg.threshold - 0.01).abs() < 1e-10);
        assert_eq!(cfg.max_hops, 1);
        assert_eq!(cfg.k_per_seed, 20);
        assert!(cfg.enable_nrem);
        assert!(cfg.enable_rem);
    }
}
