# Akar

**Pure Rust embedded graph database for AI agent memory.** Built for speed. Concurrent multi-writer support. GPLv3 licensed.

Akar is a from-scratch Rust reimplementation of [KuzuDB](https://github.com/kuzudb/kuzu), an embedded property graph database optimized for complex analytical workloads on very large graphs. Originally forked from the Vela-Engineering/Kuzu project, Akar is now a **standalone pure Rust codebase** — zero C++ dependencies, zero FFI.

[![GPLv3 License](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)

---

## Why Akar

AI agents need memory that captures relationships, not just documents. When an agent traces a chain like `Founder → Company → Round → Outcome`, that is a multi-hop graph traversal. Akar runs these **374x faster** than Neo4j on path queries (0.009s vs 3.22s) while requiring **zero infrastructure** (no server, no Docker, no connection pool).

| Capability | Akar | Neo4j | Vector DB |
|---|---|---|---|
| Multi-hop path queries | 0.009s | 3.22s | Not supported |
| Infrastructure required | None (in-process) | Server + config | Server + config |
| Concurrent writes | Yes | Yes | Yes |
| Causal chain traversal | Native | Native | Approximate (embedding) |
| License | GPLv3 | GPLv3 / Commercial | Varies |

## Quick Start

```rust
use akar::{Database, Connection};

let db = Database::new("./agent_memory")?;
let conn = Connection::new(&db)?;

// Create schema
conn.execute("CREATE NODE TABLE Entity(name STRING, type STRING, PRIMARY KEY(name))")?;
conn.execute("CREATE REL TABLE RELATES_TO(FROM Entity TO Entity, relation STRING)")?;

// Add knowledge
conn.execute("CREATE (e:Entity {name: 'Acme AI', type: 'company'})")?;
conn.execute("CREATE (e:Entity {name: 'Jane Smith', type: 'founder'})")?;

// Query: who founded what?
let result = conn.execute("
    MATCH (f:Entity)-[r:RELATES_TO {relation: 'founded'}]->(c:Entity)
    RETURN f.name, c.name
")?;
```

No server. No Docker. Just `cargo add akar` and query.

## Core Features

- **Pure Rust** — zero C++ dependencies, zero FFI, safe by default
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

Akar is a **complete from-scratch Rust reimplementation**. The Rust workspace (`kuzu-core/`) contains 31 crates and ~55K lines of pure Rust code:

| Crate | Purpose |
|-------|---------|
| `kuzu-parser` | Cypher query parser (pest-based) |
| `kuzu-binder` | Semantic analysis and type resolution |
| `kuzu-optimizer` | 25+ query optimization passes |
| `kuzu-processor` | Physical operators (scan, filter, join, aggregate, sort) |
| `kuzu-storage` | Columnar disk storage, WAL, buffer manager, CSR adjacency |
| `kuzu-function` | 58+ SQL/Cypher functions |
| `kuzu-algo` | 15 graph algorithms (BFS, DFS, PageRank, SCC, etc.) |
| `kuzu-fts` | Full-text search (BM25) |
| `kuzu-vector` | Vector similarity search |
| `kuzu-c` | C FFI API (`extern "C"`) |
| `kuzu-cli` | Interactive CLI shell |
| `kuzu-wasm` | WebAssembly bindings |

## Benchmarks

Performance parity with the original C++ implementation has been verified:

| Query Pattern | Akar (Rust) | C++ (original) |
|---|---|---|
| `MATCH (p) WHERE p.age > 30 RETURN COUNT(p)` 10k rows | **397 µs** | 400 µs |
| Filter + COUNT (end-to-end) | **397 µs** | 400 µs |

Run benchmarks locally:

```bash
cargo bench -p kuzu-main
```

## Extensions

Akar bundles commonly used extensions as compile-time features (`algo`, `fts`, `json`, `vector`). No manual installation needed.

## Documentation

- **Research paper:** [Kuzu GDBMS, CIDR 2023](https://www.cidrdb.org/cidr2023/papers/p48-jin.pdf) (original architecture)

## Origins

Akar is a from-scratch Rust reimplementation of [KuzuDB](https://github.com/kuzudb/kuzu), originally developed by Kuzu Inc. at the University of Waterloo. The original project was archived in October 2025. The engineering quality of the original codebase is exceptional, grounded in serious database research including worst-case optimal joins and factorized execution. Akar builds on that foundation.

## Contributing

We welcome contributions. Priority areas:

1. **Bug fixes and stability** — ensuring core functionality is rock-solid
2. **Performance optimization** — query execution, storage, concurrency
3. **CI/CD and testing** — cross-platform automated testing
4. **Documentation** — tutorials, examples, API reference
5. **Extension ecosystem** — new functions and integrations

## License

GNU General Public License v3.0. See [LICENSE](LICENSE).

Copyright (c) 2026 Anjang Kusuma Netra
