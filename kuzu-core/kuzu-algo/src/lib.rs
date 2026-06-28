//! Graph algorithm extension for Kuzu.
//!
//! Provides table functions that execute graph algorithms:
//! - PageRank (alias PR)
//! - Weakly Connected Components (alias WCC)
//! - Strongly Connected Components — Tarjan (alias SCC)
//! - Strongly Connected Components — Kosaraju (alias SCC_KO)
//! - K-Core Decomposition (alias KCORE)
//! - Louvain Community Detection
//! - Spanning Forest (alias SF)
//!
//! All algorithms operate on the CSR adjacency built from existing
//! node/rel tables in the database.

use kuzu_extension::{Extension, ExtensionContext};

/// The graph algorithms extension.
pub struct AlgoExtension;

impl AlgoExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for AlgoExtension {
    fn name(&self) -> &'static str {
        "ALGO"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use kuzu_function::registry::TableFunction;

        // Register table functions — all use `Custom` variant with algorithm name
        context.register_table_function(
            "page_rank",
            TableFunction::Custom { name: "page_rank".into() },
        );
        context.register_table_function(
            "pr",
            TableFunction::Custom { name: "page_rank".into() },
        );
        context.register_table_function(
            "weakly_connected_components",
            TableFunction::Custom { name: "wcc".into() },
        );
        context.register_table_function(
            "wcc",
            TableFunction::Custom { name: "wcc".into() },
        );
        context.register_table_function(
            "strongly_connected_components",
            TableFunction::Custom { name: "scc_tarjan".into() },
        );
        context.register_table_function(
            "scc",
            TableFunction::Custom { name: "scc_tarjan".into() },
        );
        context.register_table_function(
            "strongly_connected_components_kosaraju",
            TableFunction::Custom { name: "scc_kosaraju".into() },
        );
        context.register_table_function(
            "scc_ko",
            TableFunction::Custom { name: "scc_kosaraju".into() },
        );
        context.register_table_function(
            "k_core_decomposition",
            TableFunction::Custom { name: "k_core".into() },
        );
        context.register_table_function(
            "kcore",
            TableFunction::Custom { name: "k_core".into() },
        );
        context.register_table_function(
            "louvain",
            TableFunction::Custom { name: "louvain".into() },
        );
        context.register_table_function(
            "spanning_forest",
            TableFunction::Custom { name: "spanning_forest".into() },
        );
        context.register_table_function(
            "sf",
            TableFunction::Custom { name: "spanning_forest".into() },
        );

        tracing::info!("ALGO extension loaded: 14 function registrations (7 algorithms + 7 aliases)");

        Ok(())
    }
}

// ==================== Algorithm Implementations ====================
//
// All algorithms work on CSRAdjacency from kuzu-graph.
// In a real execution, the graph is built from storage first.

use kuzu_graph::CSRAdjacency;

/// Result of a graph algorithm.
pub struct AlgoResult {
    pub name: String,
    pub values: Vec<f64>,
}

// --------------- PageRank (wraps kuzu-graph implementation) ---------------

/// Compute PageRank — wraps existing `kuzu_graph::page_rank`.
pub fn compute_page_rank(csr: &CSRAdjacency) -> AlgoResult {
    let result = kuzu_graph::page_rank(csr, 0.85, 100, 1e-6);
    AlgoResult {
        name: "page_rank".into(),
        values: result.values,
    }
}

// --------------- Weakly Connected Components (wraps kuzu-graph) ---------------

/// Compute WCC — wraps existing `kuzu_graph::weakly_connected_components`.
pub fn compute_wcc(csr: &CSRAdjacency) -> AlgoResult {
    let result = kuzu_graph::weakly_connected_components(csr);
    AlgoResult {
        name: "wcc".into(),
        values: result.values,
    }
}

// --------------- Strongly Connected Components (Tarjan) ---------------

/// Compute SCC using Tarjan's algorithm.
/// Returns component ID (0-based) for each node.
pub fn compute_scc_tarjan(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    let mut index = 0usize;
    let mut indices = vec![None; n];
    let mut lowlink = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut component = vec![0usize; n];
    let mut comp_id = 0usize;

    fn strongconnect(
        v: usize,
        csr: &CSRAdjacency,
        index: &mut usize,
        indices: &mut [Option<usize>],
        lowlink: &mut [usize],
        on_stack: &mut [bool],
        stack: &mut Vec<usize>,
        component: &mut [usize],
        comp_id: &mut usize,
    ) {
        indices[v] = Some(*index);
        lowlink[v] = *index;
        *index += 1;
        stack.push(v);
        on_stack[v] = true;

        for (_, dst) in csr.neighbors(v) {
            let w = dst.offset as usize;
            if w >= csr.num_nodes() {
                continue;
            }
            if indices[w].is_none() {
                strongconnect(w, csr, index, indices, lowlink, on_stack, stack, component, comp_id);
                lowlink[v] = lowlink[v].min(lowlink[w]);
            } else if on_stack[w] {
                lowlink[v] = lowlink[v].min(indices[w].unwrap());
            }
        }

        if lowlink[v] == indices[v].unwrap() {
            // Start a new SCC
            loop {
                let w = stack.pop().unwrap();
                on_stack[w] = false;
                component[w] = *comp_id;
                if w == v {
                    break;
                }
            }
            *comp_id += 1;
        }
    }

    for v in 0..n {
        if indices[v].is_none() {
            strongconnect(
                v, csr, &mut index, &mut indices, &mut lowlink,
                &mut on_stack, &mut stack, &mut component, &mut comp_id,
            );
        }
    }

    AlgoResult {
        name: "scc_tarjan".into(),
        values: component.iter().map(|&c| c as f64).collect(),
    }
}

// --------------- Strongly Connected Components (Kosaraju) ---------------

/// Compute SCC using Kosaraju's algorithm.
pub fn compute_scc_kosaraju(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    let mut visited = vec![false; n];
    let mut order = Vec::new();

    // Phase 1: DFS to get finish order
    fn dfs1(v: usize, csr: &CSRAdjacency, visited: &mut [bool], order: &mut Vec<usize>) {
        visited[v] = true;
        for (_, dst) in csr.neighbors(v) {
            let w = dst.offset as usize;
            if w < csr.num_nodes() && !visited[w] {
                dfs1(w, csr, visited, order);
            }
        }
        order.push(v);
    }

    for v in 0..n {
        if !visited[v] {
            dfs1(v, csr, &mut visited, &mut order);
        }
    }

    // Phase 2: DFS on reversed graph (simulated via in-edge query)
    // Since we don't have reverse adjacency, we scan all nodes' neighbors.
    // For efficiency, build reverse CSR on the fly.
    let mut rev_adj: Vec<Vec<usize>> = vec![vec![]; n];
    for v in 0..n {
        for (_, dst) in csr.neighbors(v) {
            let w = dst.offset as usize;
            if w < csr.num_nodes() {
                rev_adj[w].push(v);
            }
        }
    }

    let mut component = vec![0usize; n];
    let mut comp_id = 0usize;
    let mut visited2 = vec![false; n];

    fn dfs2(
        v: usize,
        rev_adj: &[Vec<usize>],
        visited: &mut [bool],
        component: &mut [usize],
        comp_id: usize,
    ) {
        visited[v] = true;
        component[v] = comp_id;
        for &w in &rev_adj[v] {
            if !visited[w] {
                dfs2(w, rev_adj, visited, component, comp_id);
            }
        }
    }

    for &v in order.iter().rev() {
        if !visited2[v] {
            dfs2(v, &rev_adj, &mut visited2, &mut component, comp_id);
            comp_id += 1;
        }
    }

    AlgoResult {
        name: "scc_kosaraju".into(),
        values: component.iter().map(|&c| c as f64).collect(),
    }
}

// --------------- K-Core Decomposition ---------------

/// Compute k-core decomposition using iterative peeling.
/// Returns the core number for each node (0-based: max k such that
/// the node is part of the k-core).
pub fn compute_k_core(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    let mut degree: Vec<usize> = (0..n).map(|i| csr.neighbors(i).len()).collect();
    let mut core = vec![0usize; n];
    let mut active = vec![true; n];
    let mut max_core = 0usize;

    loop {
        // Find min degree among active nodes
        let min_deg = degree.iter().enumerate()
            .filter(|&(i, _)| active[i])
            .map(|(_, &d)| d)
            .min();

        let k = match min_deg {
            Some(d) => d,
            None => break,
        };

        max_core = max_core.max(k);

        // Peeling: remove all active nodes with degree == k
        let mut changed = true;
        while changed {
            changed = false;
            for v in 0..n {
                if active[v] && degree[v] <= k {
                    active[v] = false;
                    core[v] = k;
                    changed = true;
                    // Decrease degree of all neighbors
                    for (_, dst) in csr.neighbors(v) {
                        let w = dst.offset as usize;
                        if w < n && active[w] && degree[w] > 0 {
                            degree[w] -= 1;
                        }
                    }
                }
            }
        }
    }

    AlgoResult {
        name: "k_core".into(),
        values: core.iter().map(|&c| c as f64).collect(),
    }
}

// --------------- Louvain Community Detection ---------------

/// Compute community structure using the Louvain heuristic.
/// Simple implementation: modularity-based greedy optimization.
pub fn compute_louvain(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    if n == 0 {
        return AlgoResult { name: "louvain".into(), values: vec![] };
    }

    // Total edge weight (count)
    let mut m: f64 = 0.0;
    for v in 0..n {
        m += csr.neighbors(v).len() as f64;
    }
    m /= 2.0; // undirected: each edge counted twice

    if m == 0.0 {
        return AlgoResult {
            name: "louvain".into(),
            values: (0..n).map(|_| 0.0).collect(),
        };
    }

    // Initialize each node to its own community
    let mut community: Vec<usize> = (0..n).collect();
    // Degree of each node
    let degree: Vec<f64> = (0..n).map(|i| csr.neighbors(i).len() as f64).collect();

    let mut improved = true;
    let max_passes = 20;
    let mut pass = 0;

    while improved && pass < max_passes {
        improved = false;
        pass += 1;

        for v in 0..n {
            let current_comm = community[v];
            let neighbors: Vec<usize> = csr.neighbors(v)
                .iter()
                .map(|(_, dst)| dst.offset as usize)
                .filter(|&w| w < n)
                .collect();

            // Compute neighbor community weights
            let mut comm_weights: std::collections::HashMap<usize, f64> = std::collections::HashMap::new();
            for &w in &neighbors {
                *comm_weights.entry(community[w]).or_insert(0.0) += 1.0;
            }

            // Remove v from current community
            let self_weight = comm_weights.get(&current_comm).copied().unwrap_or(0.0);
            let _total_weight: f64 = neighbors.len() as f64;
            let ki = degree[v];

            // Current modularity contribution
            let sigma_tot = degree.iter()
                .enumerate()
                .filter(|&(i, _)| community[i] == current_comm)
                .map(|(_, &d)| d)
                .sum::<f64>();

            let remove_mod = (self_weight) / m - (ki * sigma_tot) / (2.0 * m * m);

            // Find best community
            let mut best_comm = current_comm;
            let mut best_gain = 0.0;

            for (&comm, &weight) in &comm_weights {
                if comm == current_comm {
                    continue;
                }
                let sigma_tot2 = degree.iter()
                    .enumerate()
                    .filter(|&(i, _)| community[i] == comm)
                    .map(|(_, &d)| d)
                    .sum::<f64>();

                let add_mod = (weight) / m - (ki * sigma_tot2) / (2.0 * m * m);
                let gain = add_mod - remove_mod;

                if gain > best_gain {
                    best_gain = gain;
                    best_comm = comm;
                }
            }

            if best_comm != current_comm {
                community[v] = best_comm;
                improved = true;
            }
        }
    }

    // Assign sequential community IDs
    let mut comm_map: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let values: Vec<f64> = community
        .iter()
        .map(|&c| {
            let len = comm_map.len();
            *comm_map.entry(c).or_insert(len) as f64
        })
        .collect();

    AlgoResult {
        name: "louvain".into(),
        values,
    }
}

// --------------- Spanning Forest (Kruskal) ---------------

/// Compute a spanning forest using Kruskal's algorithm.
/// Returns parent component ID for each node after building the forest.
/// For a connected graph, this produces a spanning tree.
pub fn compute_spanning_forest(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    if n == 0 {
        return AlgoResult { name: "spanning_forest".into(), values: vec![] };
    }

    // Union-Find data structure
    let mut parent: Vec<usize> = (0..n).collect();
    let mut rank = vec![0usize; n];

    fn find(parent: &mut [usize], x: usize) -> usize {
        if parent[x] != x {
            parent[x] = find(parent, parent[x]);
        }
        parent[x]
    }

    fn union(parent: &mut [usize], rank: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            match rank[ra].cmp(&rank[rb]) {
                std::cmp::Ordering::Less => parent[ra] = rb,
                std::cmp::Ordering::Greater => parent[rb] = ra,
                std::cmp::Ordering::Equal => {
                    parent[rb] = ra;
                    rank[ra] += 1;
                }
            }
        }
    }

    // Process each edge; for undirected, each edge appears twice in CSR
    for v in 0..n {
        for (_, dst) in csr.neighbors(v) {
            let w = dst.offset as usize;
            if w < n && v < w {
                // Only process each edge once (v < w)
                union(&mut parent, &mut rank, v, w);
            }
        }
    }

    // Compress all paths
    for i in 0..n {
        find(&mut parent, i);
    }

    // Assign component IDs
    let mut comp_map: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let values: Vec<f64> = parent
        .iter()
        .map(|&p| {
            let len = comp_map.len();
            *comp_map.entry(p).or_insert(len) as f64
        })
        .collect();

    AlgoResult {
        name: "spanning_forest".into(),
        values,
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use kuzu_graph::Edge;

    fn small_csr() -> CSRAdjacency {
        // Graph: 0--1--2--3
        //        |     |
        //        4--5--6
        let edges = vec![
            Edge { src_offset: 0, dst_offset: 1, rel_id: 0, rel_table_id: 0 },
            Edge { src_offset: 0, dst_offset: 4, rel_id: 1, rel_table_id: 0 },
            Edge { src_offset: 1, dst_offset: 2, rel_id: 2, rel_table_id: 0 },
            Edge { src_offset: 2, dst_offset: 3, rel_id: 3, rel_table_id: 0 },
            Edge { src_offset: 2, dst_offset: 6, rel_id: 4, rel_table_id: 0 },
            Edge { src_offset: 4, dst_offset: 5, rel_id: 5, rel_table_id: 0 },
            Edge { src_offset: 5, dst_offset: 6, rel_id: 6, rel_table_id: 0 },
        ];
        CSRAdjacency::build(&edges, 7)
    }

    fn disconnected_csr() -> CSRAdjacency {
        // Two components: 0--1  and  2--3
        let edges = vec![
            Edge { src_offset: 0, dst_offset: 1, rel_id: 0, rel_table_id: 0 },
            Edge { src_offset: 2, dst_offset: 3, rel_id: 1, rel_table_id: 0 },
        ];
        CSRAdjacency::build(&edges, 4)
    }

    #[test]
    fn test_page_rank() {
        let csr = small_csr();
        let result = compute_page_rank(&csr);
        assert_eq!(result.values.len(), 7);
        // All PageRank values should be positive
        for &v in &result.values {
            assert!(v > 0.0, "PageRank should be positive, got {v}");
        }
    }

    #[test]
    fn test_wcc_connected() {
        let csr = small_csr();
        let result = compute_wcc(&csr);
        assert_eq!(result.values.len(), 7);
        // Single component → all same component
        let first = result.values[0];
        assert!(result.values.iter().all(|&v| v == first));
    }

    #[test]
    fn test_wcc_disconnected() {
        let csr = disconnected_csr();
        let result = compute_wcc(&csr);
        assert_eq!(result.values.len(), 4);
        assert_eq!(result.values[0], result.values[1]); // same component
        assert_eq!(result.values[2], result.values[3]); // same component
        assert_ne!(result.values[0], result.values[2]); // different components
    }

    #[test]
    fn test_scc_tarjan() {
        let csr = small_csr();
        let result = compute_scc_tarjan(&csr);
        assert_eq!(result.values.len(), 7);
        // All nodes in the same SCC (undirected graph)
        let first = result.values[0];
        assert!(result.values.iter().all(|&v| v == first));
    }

    #[test]
    fn test_scc_kosaraju() {
        let csr = small_csr();
        let result = compute_scc_kosaraju(&csr);
        assert_eq!(result.values.len(), 7);
        let first = result.values[0];
        assert!(result.values.iter().all(|&v| v == first));
    }

    #[test]
    fn test_k_core() {
        let csr = small_csr();
        let result = compute_k_core(&csr);
        assert_eq!(result.values.len(), 7);
        // In a 3-regular-ish graph, all nodes should have core >= 2
        for &v in &result.values {
            assert!(v >= 1.0, "Core number should be >= 1");
        }
    }

    #[test]
    fn test_louvain() {
        let csr = small_csr();
        let result = compute_louvain(&csr);
        assert_eq!(result.values.len(), 7);
    }

    #[test]
    fn test_spanning_forest_connected() {
        let csr = small_csr();
        let result = compute_spanning_forest(&csr);
        assert_eq!(result.values.len(), 7);
        // Connected graph → single component
        let first = result.values[0];
        assert!(result.values.iter().all(|&v| v == first));
    }

    #[test]
    fn test_spanning_forest_disconnected() {
        let csr = disconnected_csr();
        let result = compute_spanning_forest(&csr);
        assert_eq!(result.values.len(), 4);
        assert_eq!(result.values[0], result.values[1]); // same component
        assert_eq!(result.values[2], result.values[3]); // same component
        assert_ne!(result.values[0], result.values[2]); // different components
    }

    #[test]
    fn test_algo_extension_registration() {
        // Verify the extension registers without error
        // (requires full Database context, so test just the struct)
        let ext = AlgoExtension::new();
        assert_eq!(ext.name(), "ALGO");
    }
}
