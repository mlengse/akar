# Akar Vector Similarity Search Extension

HNSW (Hierarchical Navigable Small World) approximate nearest neighbor search for the Akar database engine. Pure Rust implementation with SIMD-accelerated distance kernels.

## Core Types

**`HnswIndex`** (`hnsw.rs`) — The HNSW index with lazy graph building:

- **Flat store → Graph transition**: Starts as a flat vector store (exact scan). Graph links are built only when corpus exceeds `EXACT_SEARCH_MAX_NODES` (default 50,000).
- **`EXACT_SEARCH_MAX_NODES = 50_000`** — Vector count at or below which queries scan exactly instead of walking the graph. Can be overridden via `with_exact_threshold(metric, threshold)`.
- **`with_exact_threshold(metric, exact_threshold)`** — Create an index with explicit exact-scan threshold. A threshold of `0` forces the graph path from the first insert (used by graph-level tests).
- **Lazy graph linking** — Graph is built on-the-fly when crossing the threshold; insertion order is maintained in flat mode and consumed deterministically to link the graph.

**`HnswConfig`** — Structural and operational configuration:
- `M` (max connections per layer, default 16)
- `ef_construction` (construction-time search width, default 200)
- `ef_search` (query-time search width, default 50)
- `ml` (level normalization factor, default 1/ln(M))
- `seed` (RNG seed for deterministic level generation)

**Tombstones** — Deleted node IDs stored in `AHashSet<usize>`; ignored during search and never linked into the graph when the flat store is relinked at the threshold.

**Distance metrics** — `Cosine`, `Euclidean`, `L1`, `L2Squared`, `DotProduct` with SIMD kernels (`distance.rs`) and scalar fallback.

## Storage Integration

**`VectorIndexTable`** (`akar-storage/src/vector_index.rs`) — Persists `HnswIndex` to disk via BufferManager:
- Page-based serialization with `flush()` / `load()` round-trips
- `precision` field tracks `VectorPrecision::F32` (current) vs `F64` (legacy, must rebuild or fail loudly)
- Registered with BufferManager via `register_file()`

**Row-scoped maintenance** — `TableCatalog::refresh_vector_index_rows(&[(table_id, row_id)])` re-reads only the named rows (O(scope) cost). Used automatically by bulk write paths (`akar-main/src/bulk.rs`) after commit.

## Benchmarks

`examples/bench_021.rs` — Vector threshold & `vector_write_cost` benchmarking.

## Tests

53 tests covering: exact scan vs graph path, threshold crossing, tombstone filtering, recall against brute-force, level distribution, config compatibility, persistence round-trips, and row-scoped maintenance.