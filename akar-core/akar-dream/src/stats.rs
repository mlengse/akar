//! Primitive result type for a dream phase.
//!
//! One [`PhaseStats`] is the output of *one* phase call, not a summary of a
//! cycle: folding the seven phases into a cycle-level report requires knowing
//! which phases ran, and that ordering decision belongs to the host (SPEC §13).

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
