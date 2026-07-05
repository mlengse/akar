# GDS Framework + Shortest Path — DONE ✅

## What was built

### GDS Framework (`kuzu-core/kuzu-graph/src/gds/`)
7 files created:

1. **`mod.rs`** — Module declarations + public re-exports
2. **`frontier.rs`** — Frontier trait, SparseFrontier, DenseFrontier, FrontierPair trait, SPFrontierPair, DenseSparseDynamicFrontierPair, DenseFrontierPair. All with iteration-based frontier tracking.
3. **`compute.rs`** — EdgeCompute trait, VertexCompute trait, DefaultEdgeCompute
4. **`bfs_graph.rs`** — ParentList (linked list for path tracking), BaseBFSGraph trait, DenseBFSGraph, SparseBFSGraph, BFSGraphManager
5. **`output_writer.rs`** — RJOutputWriter trait, PathsOutputWriterInfo, PathsOutputWriter (DFS-based path enumeration), SPPathsOutputWriter
6. **`utils.rs`** — GDSUtils with `run_single_shortest_path`, `run_all_shortest_paths`, `run_weighted_shortest_path`, `run_all_weighted_shortest_paths`, `run_edge_compute`

### Algorithm Integration (`kuzu-core/kuzu-algo/src/lib.rs`)
- Shortest path algorithms registered as `CustomTable` table functions with executable closures
- GDS-based BFS + Dijkstra implementations now callable

### Key Design Decisions
- Dense frontiers used primarily (simpler than adaptive sparse↔dense)
- Sequential BFS (avoids &mut issues with rayon closures)
- ParentList uses Box-based linked list (not raw pointers like C++)
- Trait-based dispatch for algorithm extensibility

### Test Results
- 19 tests in kuzu-algo: all pass
- 31 tests in kuzu-graph (incl. 13 new GDS tests): all pass
- Full workspace: 691+ tests, 0 failures
