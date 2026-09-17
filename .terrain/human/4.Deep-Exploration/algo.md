# Deep Exploration — akar-algo

`akar-algo` is the SQL layer over `akar-graph`: it registers 11 graph algorithms as callable table functions and provides the memory-optimized `Graph` struct used for algorithm execution. Community detection, centrality, and traversal are exposed as `CALL`, so agent memory graphs can be analyzed with the same Cypher you use for CRUD.

## Key Components

| Function | Purpose | Source |
|----------|---------|--------|
| `page_rank` | Web/importance ranking | `akar-core/akar-algo/src/` |
| `wcc` | Weakly connected components | `akar-core/akar-algo/src/` |
| `scc` / `scc_kosaraju` | Strongly connected components | `akar-core/akar-algo/src/` |
| `k_core_decomposition` | Core decomposition | `akar-core/akar-algo/src/` |
| `louvain` | Community detection | `akar-core/akar-algo/src/` |
| `spanning_forest` | Spanning forest | `akar-core/akar-algo/src/` |
| `label_propagation` | Scaling label propagation | `akar-core/akar-algo/src/` |
| `betweenness_centrality` / `closeness_centrality` | Centrality | `akar-core/akar-algo/src/` |
| `triangle_count` | Triangle counting | `akar-core/akar-algo/src/` |
| `load_graph` / `project_graph` | Graph loading & projection for algorithm calls | `akar-core/akar-algo/src/` |

Interface: `LOAD EXTENSION "akar-algo"; CALL load_graph('/gdir'); CALL project_graph('cities'); CALL page_rank(graph = $g);` — registered through `AlgorithmExtension` (`akar-core/akar-algo/src/lib.rs`), executed via `StandaloneCall` (`akar-core/akar-processor/src/processor/standalone_call.rs`).

## Design Decisions

- **Algorithms over projected graphs.** Pattern: load a graph from node/rel tables once, then run multiple algorithms against it (`CALL page_rank(graph = $g)`). This amortizes graph materialization across calls — important for consolidation jobs that run several algorithms on the same memory slice.
- **Memory-optimized Graph.** The local `Graph` in algo (distinct from `akar-graph::Graph`) emphasizes cache locality and avoids per-head allocations, tuned for scales where the whole graph fits in the buffer pool.

## Why It Matters

Graph algorithms are how Akar turns a bag of memories into structure: `louvain` finds memory clusters, `label_propagation` finds spans of influence, `page_rank` finds important anchors. `akar-dream`'s GRAPH phase (`akar-core/akar-dream/src/orchestrator.rs:37`) consumes these to decide consolidation candidates, and Python's `graph_operation` exposes them directly.