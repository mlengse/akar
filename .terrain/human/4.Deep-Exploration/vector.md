# Deep Exploration — akar-vector

The vector crate implements Akar's ANN index: a Hierarchical Navigable Small World (HNSW) graph with SIMD-accelerated distance metrics, plus the physical `VectorSimilarityScan` operator that reads the HNSW graph directly from SQL. Semantic recall for agent memory depends on this crate.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `HNSW` index | Navigable small-world graph: M=16, M_MAX=32, EF_CONSTRUCTION=200, EF_SEARCH=50 | `akar-core/akar-vector/src/` |
| `DistanceMetric` enum | Cosine=0, Euclidean=1, L1=2, L2Squared=3, DotProduct=4 | `akar-core/akar-vector/src/` |
| distance SIMD kernels | SIMD dot / L2 / cosine projections | `akar-core/akar-vector/src/` |
| `VectorSimilarityScan` (physical) | K-NN retrieval from SQL-visible HNSW graph | `akar-core/akar-processor/src/processor/vector_similarity_scan.rs` |
| registered functions | `vector_similarity_scan` (table), `cosine_similarity`, `euclidean_distance`, `dot_product`, `l2_distance` (scalars) | `akar-core/akar-vector/src/lib.rs` |

## Design Decisions

- **HNSW graph read from SQL.** Since P71.4, the pass optimizer rewrites `MATCH ... WHERE cosine_similarity(n.col, q) >/= thr ORDER BY cos DESC LIMIT k` into `[VectorSimilarityScan(HNSW), Filter(cos>thr), OrderBy, Projection, Limit]`. The graph is maintained by `TableCatalog::refresh_vector_indexes_for_tables` and queried at scan time — not precomputed and post-filtered. This supersedes the old write-only/no-op path.
- **Explicit scan function too.** `CALL vector_similarity_scan(table, column, query_vector, k)` gives users the raw index probe (P71.1–P71.3). Defaults M=16 / EF_SEARCH=50 / EF_CONSTRUCTION=200 balance recall vs latency for memory-scale graphs.
- **Distance metric is a first-class column type.** Storing which metric an index uses lets the physical scan pick the matching SIMD kernel and threshold semantics.

## Why It Matters

This is the recall bottleneck of the AI-memory layer. Latency for the HNSW traversal is what `granularity of recall` benchmarks in the dream/orchestration cycle measure; quality (recall@k) is what determines whether a memory surfaces. Because the scan is SQL-visible and fused into the plan, vector search composes with graph patterns and FTS in one query — enabling the hybrid recall that `akar-search` builds on.