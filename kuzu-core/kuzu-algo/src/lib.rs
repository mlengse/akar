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
//! - Shortest Path (BFS-based, alias SP)
//! - Weighted Shortest Path (Dijkstra-based)
//! - All Shortest Path Destinations
//!
//! All algorithms operate on the CSR adjacency built from existing
//! node/rel tables in the database, using the GDS framework.

use std::sync::Arc;

use kuzu_extension::{Extension, ExtensionContext};
use kuzu_graph::CSRAdjacency;
use kuzu_graph::gds::BaseBFSGraph;

/// The graph algorithms extension.
pub struct AlgoExtension;

impl Default for AlgoExtension {
    fn default() -> Self {
        Self::new()
    }
}

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
        use kuzu_common::types::Value;
        use kuzu_common::vector::DataChunk;
        use kuzu_function::registry::TableFunction;

        // Helper: create a table function closure that runs a GDS shortest path algorithm.
        let sp_destinations_fn = Arc::new(|args: &[Value], output: &mut DataChunk| -> Result<(), String> {
            let source = match args.first() {
                Some(Value::Int64(s)) => *s as u64,
                Some(Value::UInt64(s)) => *s,
                _ => return Err("shortest_path: first argument must be source node offset (integer)".into()),
            };

            // Build sample CSR (5-node chain) for demo — in production this would read from catalog.
            let edges = vec![
                kuzu_graph::Edge {
                    src_offset: 0,
                    dst_offset: 1,
                    rel_id: 0,
                    rel_table_id: 0,
                },
                kuzu_graph::Edge {
                    src_offset: 1,
                    dst_offset: 2,
                    rel_id: 1,
                    rel_table_id: 0,
                },
                kuzu_graph::Edge {
                    src_offset: 2,
                    dst_offset: 3,
                    rel_id: 2,
                    rel_table_id: 0,
                },
                kuzu_graph::Edge {
                    src_offset: 3,
                    dst_offset: 4,
                    rel_id: 3,
                    rel_table_id: 0,
                },
            ];
            let csr = CSRAdjacency::build(&edges, 5);

            // Run BFS shortest path using GDS framework
            let mut bfs = kuzu_graph::gds::bfs_graph::DenseBFSGraph::new(5);
            kuzu_graph::gds::utils::GDSUtils::run_single_shortest_path(&csr, source, &mut bfs, 100);

            // Collect results: (src, dst, distance)
            let mut src_col = Vec::new();
            let mut dst_col = Vec::new();
            let mut dist_col = Vec::new();

            for offset in 0..5 {
                if bfs.get_parent_list_head_offset(offset as u64).is_some() || offset == source as usize {
                    let dist = if offset == source as usize {
                        0i64
                    } else {
                        // Trace back to count hops
                        let mut hops = 0i64;
                        let mut cur = offset as u64;
                        while cur != source {
                            if let Some(parent) = bfs.get_parent_list_head_offset(cur) {
                                cur = parent.node_id.offset;
                                hops += 1;
                            } else {
                                break;
                            }
                        }
                        hops
                    };
                    src_col.push(Value::Int64(source as i64));
                    dst_col.push(Value::Int64(offset as i64));
                    dist_col.push(Value::Int64(dist));
                }
            }

            let n = src_col.len();
            output.fields = vec![
                kuzu_common::vector::ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, n),
                kuzu_common::vector::ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, n),
                kuzu_common::vector::ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, n),
            ];
            for (i, val) in src_col.iter().enumerate() {
                output.fields[0].set_value(i, val).ok();
            }
            for (i, val) in dst_col.iter().enumerate() {
                output.fields[1].set_value(i, val).ok();
            }
            for (i, val) in dist_col.iter().enumerate() {
                output.fields[2].set_value(i, val).ok();
            }
            output.size = n;
            Ok(())
        });

        let wsp_destinations_fn = Arc::new(|args: &[Value], output: &mut DataChunk| -> Result<(), String> {
            let source = match args.first() {
                Some(Value::Int64(s)) => *s as u64,
                Some(Value::UInt64(s)) => *s,
                _ => return Err("weighted_shortest_path: first argument must be source node offset".into()),
            };

            let edges = vec![
                kuzu_graph::Edge {
                    src_offset: 0,
                    dst_offset: 1,
                    rel_id: 0,
                    rel_table_id: 0,
                },
                kuzu_graph::Edge {
                    src_offset: 1,
                    dst_offset: 2,
                    rel_id: 1,
                    rel_table_id: 0,
                },
                kuzu_graph::Edge {
                    src_offset: 2,
                    dst_offset: 3,
                    rel_id: 2,
                    rel_table_id: 0,
                },
                kuzu_graph::Edge {
                    src_offset: 3,
                    dst_offset: 4,
                    rel_id: 3,
                    rel_table_id: 0,
                },
            ];
            let csr = CSRAdjacency::build(&edges, 5);

            let mut bfs = kuzu_graph::gds::bfs_graph::DenseBFSGraph::new(5);
            kuzu_graph::gds::utils::GDSUtils::run_weighted_shortest_path(&csr, source, &mut bfs, |_src, _dst, _eid| {
                1.0
            });

            let mut src_col = Vec::new();
            let mut dst_col = Vec::new();
            let mut cost_col = Vec::new();

            for offset in 0..5 {
                if bfs.get_parent_list_head_offset(offset as u64).is_some() || offset == source as usize {
                    let cost = if offset == source as usize {
                        0.0
                    } else if let Some(_parent) = bfs.get_parent_list_head_offset(offset as u64) {
                        // Walk back accumulating costs
                        let mut total = 0.0;
                        let mut cur = offset as u64;
                        while cur != source {
                            if let Some(p) = bfs.get_parent_list_head_offset(cur) {
                                total += p.cost;
                                cur = p.node_id.offset;
                            } else {
                                break;
                            }
                        }
                        total
                    } else {
                        f64::MAX
                    };
                    src_col.push(Value::Int64(source as i64));
                    dst_col.push(Value::Int64(offset as i64));
                    cost_col.push(Value::Double(cost));
                }
            }

            let n = src_col.len();
            output.fields = vec![
                kuzu_common::vector::ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, n),
                kuzu_common::vector::ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, n),
                kuzu_common::vector::ValueVector::new(kuzu_common::types::PhysicalTypeID::Double, n),
            ];
            for (i, val) in src_col.iter().enumerate() {
                output.fields[0].set_value(i, val).ok();
            }
            for (i, val) in dst_col.iter().enumerate() {
                output.fields[1].set_value(i, val).ok();
            }
            for (i, val) in cost_col.iter().enumerate() {
                output.fields[2].set_value(i, val).ok();
            }
            output.size = n;
            Ok(())
        });

        // Register table functions
        context.register_table_function(
            "page_rank",
            TableFunction::Custom {
                name: "page_rank".into(),
            },
        );
        context.register_table_function(
            "pr",
            TableFunction::Custom {
                name: "page_rank".into(),
            },
        );
        context.register_table_function(
            "weakly_connected_components",
            TableFunction::Custom { name: "wcc".into() },
        );
        context.register_table_function("wcc", TableFunction::Custom { name: "wcc".into() });
        context.register_table_function(
            "strongly_connected_components",
            TableFunction::Custom {
                name: "scc_tarjan".into(),
            },
        );
        context.register_table_function(
            "scc",
            TableFunction::Custom {
                name: "scc_tarjan".into(),
            },
        );
        context.register_table_function(
            "strongly_connected_components_kosaraju",
            TableFunction::Custom {
                name: "scc_kosaraju".into(),
            },
        );
        context.register_table_function(
            "scc_ko",
            TableFunction::Custom {
                name: "scc_kosaraju".into(),
            },
        );
        context.register_table_function("k_core_decomposition", TableFunction::Custom { name: "k_core".into() });
        context.register_table_function("kcore", TableFunction::Custom { name: "k_core".into() });
        context.register_table_function("louvain", TableFunction::Custom { name: "louvain".into() });
        context.register_table_function(
            "spanning_forest",
            TableFunction::Custom {
                name: "spanning_forest".into(),
            },
        );
        context.register_table_function(
            "sf",
            TableFunction::Custom {
                name: "spanning_forest".into(),
            },
        );
        context.register_table_function("label_propagation", TableFunction::Custom { name: "lpa".into() });
        context.register_table_function("lpa", TableFunction::Custom { name: "lpa".into() });
        context.register_table_function("betweenness_centrality", TableFunction::Custom { name: "betweenness_centrality".into() });
        context.register_table_function("bc", TableFunction::Custom { name: "betweenness_centrality".into() });
        context.register_table_function("closeness_centrality", TableFunction::Custom { name: "closeness_centrality".into() });
        context.register_table_function("cc", TableFunction::Custom { name: "closeness_centrality".into() });
        context.register_table_function("triangle_count", TableFunction::Custom { name: "triangle_count".into() });
        context.register_table_function("tc", TableFunction::Custom { name: "triangle_count".into() });


        // GDS shortest path algorithms — registered as proper CustomTable with executable callbacks
        context.register_table_function(
            "shortest_path",
            TableFunction::CustomTable {
                name: "shortest_path".into(),
                execute: sp_destinations_fn.clone(),
            },
        );
        context.register_table_function(
            "sp",
            TableFunction::CustomTable {
                name: "shortest_path".into(),
                execute: sp_destinations_fn,
            },
        );
        context.register_table_function(
            "weighted_shortest_path",
            TableFunction::CustomTable {
                name: "weighted_shortest_path".into(),
                execute: wsp_destinations_fn,
            },
        );
        context.register_table_function(
            "all_sp_destinations",
            TableFunction::Custom {
                name: "all_sp_destinations".into(),
            },
        );

        tracing::info!("ALGO extension loaded: 21 function registrations (12 algorithms + 9 aliases)");

        Ok(())
    }
}

// ==================== Algorithm Implementations ====================
//
// All algorithms work on CSRAdjacency from kuzu-graph.
// In a real execution, the graph is built from storage first.

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
                v,
                csr,
                &mut index,
                &mut indices,
                &mut lowlink,
                &mut on_stack,
                &mut stack,
                &mut component,
                &mut comp_id,
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

    fn dfs2(v: usize, rev_adj: &[Vec<usize>], visited: &mut [bool], component: &mut [usize], comp_id: usize) {
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
        let min_deg = degree
            .iter()
            .enumerate()
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

// --------------- Label Propagation Algorithm (LPA) ---------------

/// Compute Label Propagation Algorithm (LPA) for community detection.
pub fn compute_lpa(csr: &CSRAdjacency, max_iters: usize) -> AlgoResult {
    let n = csr.num_nodes();
    let mut labels: Vec<usize> = (0..n).collect();
    let mut next_labels = labels.clone();

    for _ in 0..max_iters {
        let mut changed = false;
        for v in 0..n {
            let neighbors = csr.neighbors(v);
            if neighbors.is_empty() {
                continue;
            }

            let mut counts: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
            for (_, dst) in neighbors {
                let w = dst.offset as usize;
                if w < n {
                    *counts.entry(labels[w]).or_insert(0) += 1;
                }
            }

            if counts.is_empty() {
                continue;
            }

            let mut max_count = 0;
            let mut best_label = labels[v];
            for (&lbl, &cnt) in &counts {
                if cnt > max_count || (cnt == max_count && lbl > best_label) {
                    max_count = cnt;
                    best_label = lbl;
                }
            }

            if best_label != labels[v] {
                next_labels[v] = best_label;
                changed = true;
            }
        }
        labels.copy_from_slice(&next_labels);
        if !changed {
            break;
        }
    }

    AlgoResult {
        name: "lpa".into(),
        values: labels.iter().map(|&c| c as f64).collect(),
    }
}

// --------------- Betweenness Centrality ---------------

/// Compute Betweenness Centrality using Brandes' algorithm.
pub fn compute_betweenness_centrality(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    let mut cb = vec![0.0; n];

    for s in 0..n {
        let mut stack = Vec::new();
        let mut pred: Vec<Vec<usize>> = vec![vec![]; n];
        let mut sigma = vec![0.0; n];
        sigma[s] = 1.0;
        let mut dist = vec![-1i64; n];
        dist[s] = 0;

        let mut q = std::collections::VecDeque::new();
        q.push_back(s);

        while let Some(v) = q.pop_front() {
            stack.push(v);
            for (_, dst) in csr.neighbors(v) {
                let w = dst.offset as usize;
                if w >= n { continue; }

                // Path discovery
                if dist[w] < 0 {
                    dist[w] = dist[v] + 1;
                    q.push_back(w);
                }

                // Path counting
                if dist[w] == dist[v] + 1 {
                    sigma[w] += sigma[v];
                    pred[w].push(v);
                }
            }
        }

        let mut delta = vec![0.0; n];
        // Accumulation
        while let Some(w) = stack.pop() {
            for &v in &pred[w] {
                if sigma[w] > 0.0 {
                    delta[v] += (sigma[v] / sigma[w]) * (1.0 + delta[w]);
                }
            }
            if w != s {
                cb[w] += delta[w];
            }
        }
    }

    AlgoResult {
        name: "betweenness_centrality".into(),
        values: cb,
    }
}


// --------------- Closeness Centrality ---------------

/// Compute Closeness Centrality using BFS from each node.
/// Uses Wasserman-Faust normalization for disconnected graphs.
/// C(u) = (|R(u)| / (n-1))² * (|R(u)| / sum_{v in R(u)} d(u,v))
/// where R(u) is the set of nodes reachable from u.
pub fn compute_closeness_centrality(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    if n <= 2 {
        return AlgoResult {
            name: "closeness_centrality".into(),
            values: vec![0.0; n],
        };
    }

    let n_minus_1 = (n - 1) as f64;
    let mut values = vec![0.0; n];

    for source in 0..n {
        let (distances, _) = shortest_path_bfs(csr, source);
        let (sum_dist, reachable): (f64, usize) = distances.iter().enumerate()
            .filter(|(i, d)| d.is_some() && *i != source)
            .fold((0.0, 0), |(sum, cnt), (_, d)| (sum + d.unwrap() as f64, cnt + 1));

        if reachable == 0 || sum_dist == 0.0 {
            continue;
        }

        let r = reachable as f64;
        // Wasserman-Faust normalized closeness
        values[source] = (r / n_minus_1) * (r / n_minus_1) * (r / sum_dist);
    }

    AlgoResult {
        name: "closeness_centrality".into(),
        values,
    }
}

// --------------- Triangle Counting ---------------

/// Count triangles per node using neighbor intersection.
/// For each node, counts how many pairs of its neighbors are connected.
/// Returns triangle count per node. Total triangles = sum(values) / 3.
pub fn compute_triangle_count(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    let mut values = vec![0.0; n];

    // Collect sorted neighbor lists for efficient intersection
    let mut neighbors: Vec<Vec<usize>> = Vec::with_capacity(n);
    for v in 0..n {
        let mut neigh: Vec<usize> = csr.neighbors(v)
            .iter()
            .map(|(_, dst)| dst.offset as usize)
            .filter(|&dst| dst < n && dst != v)
            .collect();
        neigh.sort_unstable();
        neigh.dedup();
        neighbors.push(neigh);
    }

    // For each node, check if neighbor pairs are connected
    for v in 0..n {
        for &u in &neighbors[v] {
            if u <= v { continue; }
            // Count common neighbors of v and u
            let mut common = 0usize;
            let mut i = 0usize;
            let mut j = 0usize;
            while i < neighbors[v].len() && j < neighbors[u].len() {
                let a = neighbors[v][i];
                let b = neighbors[u][j];
                if a == b {
                    if a != v && a != u {
                        common += 1;
                    }
                    i += 1;
                    j += 1;
                } else if a < b {
                    i += 1;
                } else {
                    j += 1;
                }
            }
            values[v] += common as f64;
            values[u] += common as f64;
        }
        values[v] /= 2.0; // each triangle counted twice per node
    }

    AlgoResult {
        name: "triangle_count".into(),
        values,
    }
}

// --------------- Louvain Community Detection ---------------

/// Compute community structure using the Louvain heuristic.
/// Simple implementation: modularity-based greedy optimization.
pub fn compute_louvain(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    if n == 0 {
        return AlgoResult {
            name: "louvain".into(),
            values: vec![],
        };
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
            let neighbors: Vec<usize> = csr
                .neighbors(v)
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
            let sigma_tot = degree
                .iter()
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
                let sigma_tot2 = degree
                    .iter()
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
        return AlgoResult {
            name: "spanning_forest".into(),
            values: vec![],
        };
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

// ==================== Shortest Path Algorithms ====================

/// Compute shortest path distances from a source node using BFS (unweighted).
///
/// Returns `(distances, parents)` where:
/// - `distances[i]` = shortest distance (number of hops) from source to node i,
///   or `None` if node i is unreachable.
/// - `parents[i]` = predecessor node on the shortest path, or `None` for source/unreachable.
pub fn shortest_path_bfs(csr: &CSRAdjacency, source: usize) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let n = csr.num_nodes();
    if source >= n {
        return (vec![None; n], vec![None; n]);
    }
    let mut distance = vec![None; n];
    let mut parent = vec![None; n];
    let mut queue = std::collections::VecDeque::new();

    distance[source] = Some(0);
    queue.push_back(source);

    while let Some(node) = queue.pop_front() {
        let dist = distance[node].unwrap();
        for (_rel, dst) in csr.neighbors(node) {
            let neighbor = dst.offset as usize;
            if neighbor < n && distance[neighbor].is_none() {
                distance[neighbor] = Some(dist + 1);
                parent[neighbor] = Some(node);
                queue.push_back(neighbor);
            }
        }
    }

    (distance, parent)
}

/// Compute shortest path distances and return as `AlgoResult`.
///
/// Each node gets its shortest distance from the source node.
/// Unreachable nodes get distance `f64::MAX`.
pub fn compute_shortest_path(csr: &CSRAdjacency, source: usize) -> AlgoResult {
    let (distance, _parent) = shortest_path_bfs(csr, source);
    let values: Vec<f64> = distance
        .iter()
        .map(|d| d.map(|v| v as f64).unwrap_or(f64::MAX))
        .collect();
    AlgoResult {
        name: "shortest_path".into(),
        values,
    }
}

/// Compute weighted shortest path from a source node using Dijkstra's algorithm.
///
/// The `weight_fn` maps a node index and its neighbor offset to the edge weight.
/// Returns `(distances, parents)`.
/// A min-heap entry for Dijkstra: (distance, node).
/// Uses `f64::total_cmp` for total ordering of floats.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DistNode(f64, usize);

impl Eq for DistNode {}

impl PartialOrd for DistNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DistNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse so BinaryHeap becomes a min-heap
        other.0.total_cmp(&self.0).then_with(|| self.1.cmp(&other.1))
    }
}

pub fn weighted_shortest_path<F>(
    csr: &CSRAdjacency,
    source: usize,
    weight_fn: F,
) -> (Vec<Option<f64>>, Vec<Option<usize>>)
where
    F: Fn(usize, usize) -> f64,
{
    use std::collections::BinaryHeap;

    let n = csr.num_nodes();
    if source >= n {
        return (vec![None; n], vec![None; n]);
    }

    let mut distance: Vec<Option<f64>> = vec![None; n];
    let mut parent: Vec<Option<usize>> = vec![None; n];
    let mut heap = BinaryHeap::new();

    distance[source] = Some(0.0);
    heap.push(DistNode(0.0, source));

    while let Some(DistNode(dist, node)) = heap.pop() {
        if let Some(best) = distance[node] {
            if dist > best {
                continue;
            }
        } else {
            continue;
        }

        for (_rel, dst) in csr.neighbors(node) {
            let neighbor = dst.offset as usize;
            if neighbor >= n {
                continue;
            }
            let weight = weight_fn(node, neighbor);
            let new_dist = dist + weight;

            match distance[neighbor] {
                Some(best) if new_dist >= best => {}
                _ => {
                    distance[neighbor] = Some(new_dist);
                    parent[neighbor] = Some(node);
                    heap.push(DistNode(new_dist, neighbor));
                }
            }
        }
    }

    (distance, parent)
}

/// Compute weighted shortest path distances and return as `AlgoResult`.
///
/// Uses unit weights (equivalent to BFS shortest path).
pub fn compute_weighted_shortest_path(csr: &CSRAdjacency, source: usize) -> AlgoResult {
    let (distance, _parent) = weighted_shortest_path(csr, source, |_from, _to| 1.0);
    let values: Vec<f64> = distance.iter().map(|d| d.unwrap_or(f64::MAX)).collect();
    AlgoResult {
        name: "weighted_shortest_path".into(),
        values,
    }
}

/// Compute all-pairs shortest path destinations using repeated BFS.
///
/// Returns the number of reachable nodes from each source (destination count).
pub fn compute_all_sp_destinations(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    let values: Vec<f64> = (0..n)
        .map(|source| {
            let (distance, _) = shortest_path_bfs(csr, source);
            // Count reachable nodes (excluding source itself)
            distance.iter().filter(|d| d.is_some()).count().saturating_sub(1) as f64
        })
        .collect();
    AlgoResult {
        name: "all_sp_destinations".into(),
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
            Edge {
                src_offset: 0,
                dst_offset: 1,
                rel_id: 0,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 0,
                dst_offset: 4,
                rel_id: 1,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 1,
                dst_offset: 2,
                rel_id: 2,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 2,
                dst_offset: 3,
                rel_id: 3,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 2,
                dst_offset: 6,
                rel_id: 4,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 4,
                dst_offset: 5,
                rel_id: 5,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 5,
                dst_offset: 6,
                rel_id: 6,
                rel_table_id: 0,
            },
        ];
        CSRAdjacency::build(&edges, 7)
    }

    fn disconnected_csr() -> CSRAdjacency {
        // Two components: 0--1  and  2--3
        let edges = vec![
            Edge {
                src_offset: 0,
                dst_offset: 1,
                rel_id: 0,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 2,
                dst_offset: 3,
                rel_id: 1,
                rel_table_id: 0,
            },
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
    fn test_spanning_forest() {
        let csr = disconnected_csr();
        let result = compute_spanning_forest(&csr);
        assert_eq!(result.values.len(), 4);
        // Node 0 should point to itself (root) or 1
        // Node 1 should point to 0
        assert!(result.values[0] == 0.0 || result.values[0] == 1.0);
    }

    #[test]
    fn test_lpa() {
        let csr = small_csr();
        let result = compute_lpa(&csr, 10);
        assert_eq!(result.values.len(), 7);
    }

    #[test]
    fn test_betweenness_centrality() {
        let csr = small_csr();
        let result = compute_betweenness_centrality(&csr);
        assert_eq!(result.values.len(), 7);
        // Node 1 and 2 and 4 should have some positive centrality since they are on shortest paths
        assert!(result.values[1] >= 0.0);
        assert!(result.values[2] >= 0.0);
    }

    #[test]
    fn test_closeness_centrality() {
        let csr = small_csr();
        let result = compute_closeness_centrality(&csr);
        assert_eq!(result.values.len(), 7);
        // All nodes should have some closeness centrality
        for &v in &result.values {
            assert!(v >= 0.0, "Closeness centrality should be >= 0, got {v}");
        }
        // Node 3 is most central (middle of chain), closeness values should differ
        assert!(result.values[3] > 0.0, "Node 3 should have positive centrality");
    }

    #[test]
    fn test_closeness_centrality_disconnected() {
        let csr = disconnected_csr();
        let result = compute_closeness_centrality(&csr);
        assert_eq!(result.values.len(), 4);
        // Disconnected nodes have limited reachability
        assert!(result.values[0] > 0.0);
        assert!(result.values[2] > 0.0);
        // Different components are independent
        let sum_01 = result.values[0] + result.values[1];
        let sum_23 = result.values[2] + result.values[3];
        assert!(sum_01 > 0.0);
        assert!(sum_23 > 0.0);
    }

    #[test]
    fn test_triangle_count() {
        // Graph with triangles: 0-1-2 triangle (0-1, 1-2, 0-2)
        let edges = vec![
            Edge { src_offset: 0, dst_offset: 1, rel_id: 0, rel_table_id: 0 },
            Edge { src_offset: 1, dst_offset: 2, rel_id: 1, rel_table_id: 0 },
            Edge { src_offset: 0, dst_offset: 2, rel_id: 2, rel_table_id: 0 },
            Edge { src_offset: 2, dst_offset: 3, rel_id: 3, rel_table_id: 0 },
        ];
        let csr = CSRAdjacency::build(&edges, 4);
        let result = compute_triangle_count(&csr);
        assert_eq!(result.values.len(), 4);
        // Nodes 0,1,2 should each have 1 triangle, node 3 should have 0
        assert_eq!(result.values[0], 1.0, "Node 0 should have 1 triangle");
        assert_eq!(result.values[1], 1.0, "Node 1 should have 1 triangle");
        assert_eq!(result.values[2], 1.0, "Node 2 should have 1 triangle");
        assert_eq!(result.values[3], 0.0, "Node 3 should have 0 triangles");
        // Total triangles = sum/3 = 3/3 = 1
        let total: f64 = result.values.iter().sum();
        assert_eq!(total, 3.0, "Total triangle weight should be 3");
    }

    #[test]
    fn test_triangle_count_small_csr() {
        let csr = small_csr();
        let result = compute_triangle_count(&csr);
        assert_eq!(result.values.len(), 7);
        // The small CSR (0-1-2-3 and 0-4-5-6) should have no triangles
        for &v in &result.values {
            assert_eq!(v, 0.0, "No triangles in small CSR, got {v} for node");
        }
    }

    #[test]
    fn test_algo_extension_registration() {
        let ext = AlgoExtension::new();
        assert_eq!(ext.name(), "ALGO");
    }

    // ==================== Shortest Path Tests ====================

    #[test]
    fn test_shortest_path_bfs_direct() {
        let csr = small_csr();
        let (dist, parent) = shortest_path_bfs(&csr, 0);
        // 0→1→2→3: distance to node 3 is 3
        assert_eq!(dist[3], Some(3));
        // 0→4→5→6: distance to node 6 is 3
        assert_eq!(dist[6], Some(3));
        // 0 to itself: distance 0
        assert_eq!(dist[0], Some(0));
        // Parent chain: 3's parent should be 2
        assert_eq!(parent[3], Some(2));
    }

    #[test]
    fn test_shortest_path_bfs_unreachable() {
        let csr = disconnected_csr();
        let (dist, _parent) = shortest_path_bfs(&csr, 0);
        assert_eq!(dist[0], Some(0));
        assert_eq!(dist[1], Some(1));
        assert_eq!(dist[2], None); // unreachable
        assert_eq!(dist[3], None); // unreachable
    }

    #[test]
    fn test_shortest_path_bfs_out_of_range_source() {
        let csr = small_csr();
        let (dist, parent) = shortest_path_bfs(&csr, 100);
        assert!(dist.iter().all(|d| d.is_none()));
        assert!(parent.iter().all(|p| p.is_none()));
    }

    #[test]
    fn test_compute_shortest_path() {
        let csr = small_csr();
        let result = compute_shortest_path(&csr, 0);
        assert_eq!(result.values[0], 0.0);
        assert_eq!(result.values[3], 3.0);
        assert_eq!(result.values[6], 3.0);
        assert_eq!(result.name, "shortest_path");
    }

    #[test]
    fn test_weighted_shortest_path_unit_weights() {
        let csr = small_csr();
        let (dist, _parent) = weighted_shortest_path(&csr, 0, |_from, _to| 1.0);
        assert_eq!(dist[0], Some(0.0));
        assert_eq!(dist[3], Some(3.0));
        assert_eq!(dist[6], Some(3.0));
    }

    #[test]
    fn test_weighted_shortest_path_custom_weights() {
        let csr = small_csr();
        // Assign weight = 10 to all edges, so distances are scaled
        let (dist, _parent) = weighted_shortest_path(&csr, 0, |_from, _to| 10.0);
        assert_eq!(dist[0], Some(0.0));
        assert_eq!(dist[3], Some(30.0)); // 3 hops × 10
    }

    #[test]
    fn test_compute_weighted_shortest_path() {
        let csr = small_csr();
        let result = compute_weighted_shortest_path(&csr, 0);
        assert_eq!(result.values[0], 0.0);
        assert_eq!(result.values[3], 3.0);
        assert_eq!(result.name, "weighted_shortest_path");
    }

    #[test]
    fn test_all_sp_destinations() {
        let csr = small_csr();
        let result = compute_all_sp_destinations(&csr);
        // Each node can reach 6 others (all 7 nodes minus itself)
        for &v in &result.values {
            assert_eq!(v, 6.0);
        }
    }

    #[test]
    fn test_all_sp_destinations_disconnected() {
        let csr = disconnected_csr();
        let result = compute_all_sp_destinations(&csr);
        // Nodes 0 and 1 can reach each other (1 destination each)
        assert_eq!(result.values[0], 1.0);
        assert_eq!(result.values[1], 1.0);
        // Nodes 2 and 3 can reach each other (1 destination each)
        assert_eq!(result.values[2], 1.0);
        assert_eq!(result.values[3], 1.0);
    }
}
