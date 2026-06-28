# Kuzu Core — Rust Port

A from-scratch Rust port of [Kuzu](https://github.com/kuzudb/kuzu), an embedded property graph database management system (GDBMS) with openCypher query support.

## Architecture

```
kuzu-core/
├── kuzu-common/       # Type system, vectors, serialization, memory, task system
├── kuzu-storage/      # Buffer manager, page management, WAL, compression, indexing
├── kuzu-transaction/  # MVCC transaction manager with timestamp ordering
├── kuzu-catalog/      # System catalog (schemas, tables, types)
├── kuzu-parser/       # PEG grammar (pest.rs) for Cypher subset
├── kuzu-binder/       # Semantic analysis, symbol resolution, type inference
├── kuzu-planner/      # Logical query plan construction
├── kuzu-optimizer/    # Optimization passes (filter push-down, projection push-down, etc.)
├── kuzu-processor/    # Physical operator execution pipeline
├── kuzu-function/     # Built-in function registry and evaluation
├── kuzu-graph/        # CSR adjacency, graph algorithms (BFS, PageRank, WCC)
├── kuzu-extension/    # Extension framework trait + registry
├── kuzu-json/         # JSON extension (extract, validate, type, structure)
├── kuzu-fts/          # Full-Text Search extension (stemmer, tokenizer, BM25)
├── kuzu-main/         # Public API: Database, Connection, QueryResult
└── kuzu-cli/          # Interactive Cypher shell (REPL)
```

## Quick Start

```bash
cargo build --release
cargo run --bin kuzu-cli
```

Or with a persistent database:

```bash
cargo run --bin kuzu-cli -- /path/to/db
```

## Query Pipeline

```
Cypher text
    │
    ▼
┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌────────────┐
│  Parser   │───▶│  Binder  │───▶│  Planner │───▶│Optimizer │───▶│  Processor │
│ (pest.rs) │    │(Catalog) │    │ (logical)│    │(8 passes)│    │ (physical) │
└──────────┘    └──────────┘    └──────────┘    └──────────┘    └────────────┘
                                                                    │
                                                                    ▼
                                                              DataChunks
```

## Status

| Phase | Component | Tests | Status |
|-------|-----------|-------|--------|
| 1 | Common & Types | 25 | ✅ |
| 2 | Storage Engine | 15 | ✅ |
| 3 | Transaction Manager | 11 | ✅ |
| 4 | Catalog | 14 | ✅ |
| 5 | Parser (Cypher PEG) | 12 | ✅ |
| 6 | Binder | 13 | ✅ |
| 7 | Planner | 6 | ✅ |
| 8 | Optimizer | 8 passes | ✅ |
| 9 | Functions | 30 | ✅ |
| 10 | Processor | 9 | ✅ |
| 11 | Graph Algorithms | 16 | ✅ |
| 12 | Main API | 17 | ✅ |
| 13 | Extensions (JSON, FTS) | 26 | ✅ |
| 14 | CLI, Docs, Cleanup | — | ✅ |
| **Total** | | **203** | **✅** |

## Building

### Prerequisites

- Rust 1.80+ (MSRV)
- Cargo

### Native (Windows/MinGW)

```bash
cargo build --target x86_64-pc-windows-gnu
cargo test --target x86_64-pc-windows-gnu --workspace
```

### WASM

```bash
rustup target add wasm32-unknown-unknown
cargo check --target wasm32-unknown-unknown --workspace
```

## License

MIT — see [LICENSE](../LICENSE).
