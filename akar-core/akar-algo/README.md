# Akar Graph Algorithm Extension

Graph algorithm functions callable from Cypher queries.

**Algorithms (11):**
- `page_rank` — power iteration with configurable damping factor
- `wcc` — Weakly Connected Components (Union-Find)
- `scc` — Strongly Connected Components (Tarjan's)
- `scc_kosaraju` — SCC via Kosaraju's algorithm
- `k_core_decomposition` — k-core decomposition
- `louvain` — community detection via modularity optimization
- `spanning_forest` — minimum spanning forest
- `label_propagation` — Label Propagation Algorithm
- `betweenness_centrality` — node betweenness centrality
- `closeness_centrality` — node closeness centrality
- `triangle_count` — per-node triangle counting

**Usage pattern:**
```cypher
LOAD EXTENSION '.../libalgo.Akar_extension';
CALL PROJECT_GRAPH('g', ['Person'], ['knows']);
CALL page_rank('g') RETURN node.id, rank ORDER BY rank DESC LIMIT 10;
CALL DROP_PROJECTED_GRAPH('g');
```

**Tests:** 34
