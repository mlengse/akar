# Kuzu Graph

CSR adjacency graph and graph algorithms.

**Structures:**
- `Graph` / `GraphEntry` — metadata storage
- `CSRAdjacency` — Compressed Sparse Row format
- `Edge` — source, destination, relationship ID
- `OnDiskGraph` — storage-backed graph

**Algorithms:**
- BFS (breadth-first search with distances + parents)
- PageRank (power iteration with configurable damping factor)
- WCC (Weakly Connected Components via Union-Find with path compression)
- SCC (Strongly Connected Components via Tarjan's algorithm)
- SCC-Kosaraju (alternative SCC via Kosaraju's algorithm)
- K-Core (k-core decomposition)
- Louvain (community detection via modularity optimization)
- Spanning Forest (minimum spanning forest)
- Shortest Path (BFS-based)
- Reachable Within (BFS with max distance)
- Degree Centrality

**Tests:** 16
