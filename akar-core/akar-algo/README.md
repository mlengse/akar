# Kuzu Graph Algorithm Extension

Graph algorithm functions callable from Cypher queries.

**Algorithms (7):**
- `page_rank` — power iteration with configurable damping factor
- `wcc` — Weakly Connected Components (Union-Find)
- `scc` — Strongly Connected Components (Tarjan's)
- `scc_kosaraju` — SCC via Kosaraju's algorithm
- `k_core_decomposition` — k-core decomposition
- `louvain` — community detection via modularity optimization
- `spanning_forest` — minimum spanning forest

**Usage pattern:**
```cypher
LOAD EXTENSION '.../libalgo.kuzu_extension';
CALL PROJECT_GRAPH('g', ['Person'], ['knows']);
CALL page_rank('g') RETURN node.id, rank ORDER BY rank DESC LIMIT 10;
CALL DROP_PROJECTED_GRAPH('g');
```

**Tests:** 10
