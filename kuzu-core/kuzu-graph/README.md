# Kuzu Graph

CSR adjacency graph and graph algorithms.

**Structures:**
- `Graph` / `GraphEntry` — metadata storage
- `CSRAdjacency` — Compressed Sparse Row format
- `Edge` — source, destination, relationship ID
- `OnDiskGraph` — storage-backed graph

**Algorithms:**
- BFS (breadth-first search with distances + parents)
- PageRank (power iteration with configurable damping)
- WCC (Weakly Connected Components via Union-Find)
- Shortest Path (BFS-based)
- Reachable Within (BFS with max distance)
- Degree Centrality

**Tests:** 16
