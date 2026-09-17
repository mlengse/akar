# Deep Exploration — akar-graph

The graph crate gives Akar its graph analytics heart: compressed sparse row (CSR) adjacency structures, graph algorithm kernels, and a Graph Data Science (GDS) framework through which algorithms run as table functions. Combined with `akar-algo`, it powers community detection, centrality, and traversal used by agent memory consolidation and memory graph analysis.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `Graph` / `GraphEntry` | Graph representation loaded from node/rel tables | `akar-core/akar-graph/src/lib.rs` |
| `CSRAdjacency` | Compressed Sparse Row adjacency (label-keyed) | `akar-core/akar-graph/src/`, also `akar-storage/src/csr.rs` |
| `Edge` / `OnDiskGraph` | Edge traversal + disk-resident graph support | `akar-core/akar-graph/src/` |
| algorithm kernels | BFS, PageRank, WCC, SCC, SCC-Kosaraju, K-Core, Louvain, Spanning Forest, Shortest Path, Reachable Within, Degree Centrality | `akar-core/akar-graph/src/` |
| GDS framework | Table-function registration + ProjectGraph for algorithms | `akar-core/akar-graph/src/` |

## Design Decisions

- **CSR adjacency for dense relational traversal.** A graph is loaded once into `Graph`/`GraphEntry` and traversed through CSR offsets, giving cache-friendly sequential access. The README notes the CSR structure used to be a stub (ADR-004 historical note) — it is now active and read from SQL.
- **Algorithms as table functions, not internal calls.** `CALL page_rank(graph = $g)` runs the same kernel that internal traversal uses, exposed to users. This keeps `akar-graph` kernels decoupled from `akar-algo`'s SQL surface (`akar-core/akar-algo/src/lib.rs` registers the functions).
- **On-disk graph support.** `OnDiskGraph` avoids loading the whole graph into memory — important for large memory graphs that consolidate only a slice.

## Why It Matters

Graph semantics are the difference between a KV store and a graph DB. Agent memories are graphs (facts → nodes, relations → edges), so traversal, reachability, and community detection are primary workloads. The GDS framework means algorithms work over relational storage through one interface used by `akar-dream`'s graph phase and Python's `graph_operation`.