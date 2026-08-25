# Akar Core

The `akar-core/` workspace contains all 35 Rust crates of **Akar**, a pure-Rust embedded
graph database (KuzuDB reimplementation). The public overview, quick start, benchmarks,
and contributing guidance live in the repository-wide **[`../README.md`](../README.md)**.

## Highlights

- **Test suite:** 1,889 total / 0 ignored / 1,889 passed / 0 failed (gate `test [akar-core]`, 2026-08-25)
- **Query pipeline:** parser → binder → planner → optimizer (24 passes) → processor
- **Storage:** columnar disk storage, WAL + replayer, buffer manager, CSR adjacency, ART/HNSW indexes
- **Extensions:** JSON, FTS, Vector, DuckDB, SQLite, Postgres, Neo4j, HTTPFS, Delta, Iceberg, Azure, Unity Catalog, LLM, algo, server
- **GDS:** 17 graph algorithms (BFS, Dijkstra, PageRank, WCC, SCC, K-Core, Louvain, node2vec, ...)
- **Public API:** `akar-main` (Database/Connection/QueryResult), `akar-c` FFI, `akar-wasm`, `akar-cli`

## Documentation

| Document | Location |
|----------|----------|
| Repository specification (types, functions, testing, benchmarks) | [`../SPEC.md`](../SPEC.md) |
| Migration from C++ KuzuDB | [`../MIGRATION.md`](../MIGRATION.md) |
| Release process & crates.io publishing | [`RELEASE.md`](RELEASE.md) |
| Benchmark comparison vs C++ | [`BENCHMARK_COMPARISON.md`](BENCHMARK_COMPARISON.md) |
| Per-crate READMEs | `akar-core/*/README.md` |

## License

GPLv3 — see [LICENSE](../LICENSE).
