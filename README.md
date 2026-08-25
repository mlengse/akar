# Akar

**Pure Rust embedded graph database for AI agent memory.** Built for speed. Concurrent
multi-writer support. GPLv3 licensed.

Akar is a from-scratch Rust reimplementation of [KuzuDB](https://github.com/kuzudb/kuzu),
an embedded property graph database optimized for complex analytical workloads on very
large graphs. Originally forked from the Vela-Engineering/Kuzu project, Akar is now a
**standalone pure Rust codebase** - zero C++ dependencies, zero FFI.

[![GPLv3 License](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)

---

## Why Akar

AI agents need memory that captures relationships, not just documents. When an agent
traces a chain like `Founder -> Company -> Round -> Outcome`, that is a multi-hop graph
traversal. Akar handles exactly this pattern as an embedded, in-process database -
requiring **zero infrastructure** (no server, no Docker, no connection pool).

Performance is validated against two independent C++ implementations of the same
architecture (KuzuDB): **3-way parity** on the hot path
(`MATCH (p) WHERE p.age > 30 RETURN COUNT(p)`, 10k rows): Rust **397 us** ~ Kuzu C++
**400 us** ~ LadybugDB C++ **374 us**. See [`BENCHMARK_COMPARISON.md`](akar-core/BENCHMARK_COMPARISON.md).

> **Note:** Akar has **not** been benchmarked directly against Neo4j or any vector database.
> Verified comparisons are limited to the Kuzu C++ (Vela) and LadybugDB C++ implementations
> on identical 10k-row datasets.

## Quick Start

```rust
use akar_main::database::{Database, SystemConfig};
use akar_main::connection::Connection;
use std::sync::Arc;

let db = Arc::new(Database::new("./agent_memory", SystemConfig::default())?);
let conn = Connection::new(&db);

// Create schema
conn.query("CREATE NODE TABLE Entity(name STRING, type STRING, PRIMARY KEY(name))")?;
conn.query("CREATE REL TABLE RELATES_TO(FROM Entity TO Entity, relation STRING)")?;

// Add knowledge
conn.query("CREATE (:Entity {name: 'Acme AI', type: 'company'})")?;
conn.query("CREATE (:Entity {name: 'Jane Smith', type: 'founder'})")?;

// Query: who founded what?
let result = conn.query("
    MATCH (f:Entity)-[r:RELATES_TO {relation: 'founded'}]->(c:Entity)
    RETURN f.name, c.name
")?;
```

No server. No Docker. Just `cargo add akar-main` and query.

## Core Features

- **Pure Rust** - zero C++ dependencies, zero FFI, safe by default
- **Property Graph Model** with openCypher query language
- **Embedded, in-process** execution with sub-millisecond latency
- **Concurrent multi-writer support** for multi-agent architectures
- **Vector search** and **full-text search** built in
- **Columnar disk-based storage** with CSR adjacency indices
- **Vectorized query processor** with Arrow compute kernels
- **Worst-case optimal join algorithms** for complex many-to-many traversals
- **Multi-core parallelism** across all available CPU cores
- **Serializable ACID transactions**
- **WebAssembly bindings** for browser execution

## Architecture

Akar is a **complete from-scratch Rust reimplementation**. The Rust workspace (`akar-core/`)
contains 35 crates and ~106K lines of pure Rust code (git-tracked, incl. tests):

| Crate | Purpose |
|-------|---------|
| `akar-parser` | Cypher query parser (pest-based) |
| `akar-binder` | Semantic analysis and type resolution |
| `akar-optimizer` | 24 query optimization passes |
| `akar-processor` | Physical operators (scan, filter, join, aggregate, sort) |
| `akar-storage` | Columnar disk storage, WAL, buffer manager, CSR adjacency |
| `akar-function` | 259 SQL/Cypher functions (244 scalar + 14 aggregate + 1 table) |
| `akar-algo` | 17 graph algorithms (PageRank, SCC, Louvain, node2vec, etc.) |
| `akar-fts` | Full-text search (BM25) |
| `akar-vector` | Vector similarity search |
| `akar-server` | Embedded TCP server mode (multi-process access) |
| `akar-c` | C FFI API (`extern "C"`) |
| `akar-cli` | Interactive CLI shell |
| `akar-wasm` | WebAssembly bindings |

**Test suite:** **1,889 tests, 0 failed** (gate `test [akar-core]`,
2026-08-25). **24 optimizer passes**, **37 logical types**, **59 logical
operators**, **50 physical operator structs**.

## Benchmarks

Performance parity with the C++ implementations of the same architecture (KuzuDB/Vela and
LadybugDB) has been verified on the hot path. Large-scale benchmarks (100K/1M rows) confirm
near-linear scaling:

| Scale | Scan | Filter | COUNT | Filter+COUNT |
|---|---|---|---|---|
| 10K rows | **3.0 ms** | **2.9 ms** | **2.8 ms** | **3.2 ms** |
| 100K rows | **23.4 ms** | **22.7 ms** | **23.8 ms** | **23.7 ms** |
| 1M rows | **222 ms** | **235 ms** | **212 ms** | **237 ms** |

**3-way C++ parity** (10K, `MATCH (p) WHERE p.age > 30 RETURN COUNT(p)`):
- Rust: **397 us** | Vela C++: **400 us** | LadybugDB C++: **374 us**

Repeated queries benefit from the **plan cache** (LRU at the connection level): identical
statements skip parse/bind/plan/optimize entirely, which is significant for
planning-dominated workloads (complex plans on small data).

Run benchmarks locally:

```bash
cargo bench -p akar-main                          # 10K benchmarks
cargo bench -p akar-main --bench ladybug_suite -- "ladybug_100k"  # 100K
cargo bench -p akar-main --bench ladybug_suite -- "ladybug_1m"    # 1M
```

## Extensions

Akar bundles commonly used extensions as compile-time cargo features (`algo-extension`,
`fts-extension`, `json-extension`, `vector-extension`, plus httpfs, duckdb, sqlite,
postgres, neo4j, delta, iceberg, azure, unity-catalog, llm). No manual installation needed.

## Documentation

- **Research paper:** [Kuzu GDBMS, CIDR 2023](https://www.cidrdb.org/cidr2023/papers/p48-jin.pdf) (original architecture)

## Origins

Akar is a from-scratch Rust reimplementation of [KuzuDB](https://github.com/kuzudb/kuzu),
originally developed by Kuzu Inc. at the University of Waterloo. The original project was
archived in October 2025. The engineering quality of the original codebase is exceptional,
grounded in serious database research including worst-case optimal joins and factorized
execution. Akar builds on that foundation.

## Contributing

We welcome contributions. Priority areas:

1. **Bug fixes and stability** - ensuring core functionality is rock-solid
2. **Performance optimization** - query execution, storage, concurrency
3. **CI/CD and testing** - cross-platform automated testing
4. **Documentation** - tutorials, examples, API reference
5. **Extension ecosystem** - new functions and integrations

## License

GNU General Public License v3.0. See [LICENSE](LICENSE).

Copyright (c) 2026 Anjang Kusuma Netra
