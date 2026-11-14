# Akar Graph Algorithm Extension

Graph algorithm functions callable from Cypher queries. **Pure Rust crate embedded statically** — no dynamic loading required.

## Algorithms (18)

- `page_rank` — Power iteration with configurable damping factor
- `wcc` — Weakly Connected Components (Union-Find, iterative find for deep chains)
- `scc` — Strongly Connected Components (Tarjan's)
- `scc_kosaraju` — SCC via Kosaraju's algorithm
- `k_core_decomposition` — k-core decomposition
- `louvain` — Community detection via modularity optimization
- `spanning_forest` — Minimum spanning forest
- `label_propagation` — Label Propagation Algorithm
- `betweenness_centrality` — Node betweenness centrality
- `closeness_centrality` — Node closeness centrality
- `triangle_count` — Per-node triangle counting
- `bfs` — Breadth-first search with path tracking
- `single_shortest_path` — Single-source shortest path (BFS-based)
- `all_shortest_paths` — All shortest paths from a source
- `dijkstra` — Dijkstra's algorithm for weighted graphs
- `astar` — A* search with heuristic
- `community_detection` — Generic community detection framework
- `graph_embedding` — Graph embedding utilities (Node2Vec, etc.)

## Usage Pattern

```cypher
CALL PROJECT_GRAPH('g', ['Person'], ['knows']);
CALL page_rank('g') RETURN node.id, rank ORDER BY rank DESC LIMIT 10;
CALL DROP_PROJECTED_GRAPH('g');
```

Graph projection is explicit via `PROJECT_GRAPH` / `DROP_PROJECTED_GRAPH` — no C++ `LOAD EXTENSION` required. The algorithms operate on the projected CSR adjacency structure.

## BFS Graph Internals

- **DenseBFSGraph** — Array-based parent storage for dense frontiers
- **SparseBFSGraph** — HashMap-based parent storage for sparse frontiers
- **BFSGraphManager** — Manages sparse↔dense switching based on `GDSDensityState`

## Tests

53 tests covering: empty graph handling, deep chain (200k nodes) stack-safety for Union-Find, weighted/unweighted path variants, and recall validation.