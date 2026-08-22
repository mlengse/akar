//! Backend trait for dream engine storage operations.

/// A memory node in the graph.
#[derive(Debug, Clone)]
pub struct Memory {
    pub id: usize,
    pub salience: f64,
    pub created_at: f64,
    pub content: String,
}

/// An edge in the graph.
#[derive(Debug, Clone)]
pub struct Edge {
    pub source_id: usize,
    pub target_id: usize,
    pub weight: f64,
}

/// Result of a dream phase.
#[derive(Debug, Clone)]
pub struct PhaseResult {
    pub phase: &'static str,
    pub stats: std::collections::HashMap<String, f64>,
}

/// Backend trait for dream engine storage operations.
///
/// Implementations provide the interface between the dream engine
/// and the underlying storage (akar engine, or a mock for testing).
pub trait DreamBackend {
    /// Sample memories for the NREM phase.
    fn sample_for_dream(
        &self,
        max_memories: usize,
        recent_pct: f64,
        random_old_pct: f64,
        low_salience_pct: f64,
    ) -> Vec<Memory>;

    /// Get all edges in the graph.
    fn get_connections(&self) -> Vec<Edge>;

    /// Strengthen an edge's weight.
    fn strengthen_edge(&self, source_id: usize, target_id: usize, amount: f64);

    /// Weaken an edge's weight.
    fn weaken_edge(&self, source_id: usize, target_id: usize, amount: f64);

    /// Prune (delete) an edge.
    fn prune_edge(&self, source_id: usize, target_id: usize);

    /// Update superseded edges (set valid_to).
    fn update_supersedes(&self) -> usize;

    /// Find bridges between isolated communities using centroid cosine.
    fn find_bridges(
        &self,
        communities: &[Vec<usize>],
        embeddings: &[[f64; 384]],
        max_bridges: usize,
    ) -> Vec<(usize, usize)>;

    /// Create bridge edges between communities.
    fn create_bridge_edges(&self, bridges: &[(usize, usize)]);

    /// Get community assignments via Louvain.
    fn get_communities(&self) -> Vec<usize>;

    /// Write community assignments to storage.
    fn write_communities(&self, assignments: &[usize]);

    /// Extract atomic facts from memories.
    fn extract_afe_facts(&self, memories: &[Memory]) -> Vec<(String, usize)>;

    /// Write AFE facts as new memory nodes.
    fn write_afe_facts(&self, facts: &[(String, usize)]);

    /// Merge AFE clusters into synthesis memories.
    fn run_synthesis(&self) -> usize;

    /// Recompute DAE embeddings for all memories.
    fn recompute_dae(&self) -> usize;
}

/// Mock backend for testing.
#[cfg(test)]
pub struct MockBackend {
    pub memories: std::cell::RefCell<Vec<Memory>>,
    pub edges: std::cell::RefCell<Vec<Edge>>,
    pub strengthen_count: std::cell::RefCell<usize>,
    pub weaken_count: std::cell::RefCell<usize>,
    pub prune_count: std::cell::RefCell<usize>,
}

#[cfg(test)]
impl Default for MockBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl MockBackend {
    pub fn new() -> Self {
        Self {
            memories: std::cell::RefCell::new(Vec::new()),
            edges: std::cell::RefCell::new(Vec::new()),
            strengthen_count: std::cell::RefCell::new(0),
            weaken_count: std::cell::RefCell::new(0),
            prune_count: std::cell::RefCell::new(0),
        }
    }
}

#[cfg(test)]
impl DreamBackend for MockBackend {
    fn sample_for_dream(
        &self,
        max_memories: usize,
        _recent_pct: f64,
        _random_old_pct: f64,
        _low_salience_pct: f64,
    ) -> Vec<Memory> {
        self.memories.borrow().iter().take(max_memories).cloned().collect()
    }

    fn get_connections(&self) -> Vec<Edge> {
        self.edges.borrow().clone()
    }

    fn strengthen_edge(&self, _source_id: usize, _target_id: usize, _amount: f64) {
        *self.strengthen_count.borrow_mut() += 1;
    }

    fn weaken_edge(&self, _source_id: usize, _target_id: usize, _amount: f64) {
        *self.weaken_count.borrow_mut() += 1;
    }

    fn prune_edge(&self, _source_id: usize, _target_id: usize) {
        *self.prune_count.borrow_mut() += 1;
    }

    fn update_supersedes(&self) -> usize {
        0
    }

    fn find_bridges(
        &self,
        _communities: &[Vec<usize>],
        _embeddings: &[[f64; 384]],
        _max_bridges: usize,
    ) -> Vec<(usize, usize)> {
        Vec::new()
    }

    fn create_bridge_edges(&self, _bridges: &[(usize, usize)]) {}

    fn get_communities(&self) -> Vec<usize> {
        Vec::new()
    }

    fn write_communities(&self, _assignments: &[usize]) {}

    fn extract_afe_facts(&self, _memories: &[Memory]) -> Vec<(String, usize)> {
        Vec::new()
    }

    fn write_afe_facts(&self, _facts: &[(String, usize)]) {}

    fn run_synthesis(&self) -> usize {
        0
    }

    fn recompute_dae(&self) -> usize {
        0
    }
}
