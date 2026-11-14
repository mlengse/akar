# Migrating to Akar Rust

This guide covers migration from the legacy C++ Kuzu API to the pure Rust `akar-core`.

## Why Migrate?

The Rust port offers:
- **~100% functional parity** with C++ (gate `test [akar-core]`: 2,315+ passed / 0 failed / 0 ignored per 2026-11-14 — lihat CHANGELOG untuk angka terkini)
- **Memory safety** via Rust's ownership model
- **Arrow-native execution** — up to 24x faster filtering/numeric expressions
- **Operator fusing** — fewer physical nodes, less overhead
- **Simpler builds** — `cargo build` instead of CMake + 28 vendored C libraries
- **Cross-platform** — Linux, macOS, Windows, WASM
- **Extensible** — 15+ extensions (JSON, FTS, Vector, DuckDB, SQLite, Postgres, etc.)

## Data Migration

### Prerequisites

```bash
# PyPI akar is FROZEN at 0.2.5 (Constitution Principle VI)
# Use Rust crate embedding (akar-main) for active development
pip install akar==0.2.5
```

The legacy C++ engine is read via its Python bindings (`kuzu` / `ladybug`) only by `akar-migrate` on the machine that still has the old database.

### Step 1: Export legacy C++ database

```bash
cargo run --bin akar-migrate -- --from /path/to/legacy/cpp-db --to /path/to/new/rust-db
```

This tool:
1. Reads the C++ database via Python bindings
2. Exports schema and data as Parquet files
3. Creates the Rust database with matching DDL
4. Imports all data via `COPY FROM '...parquet'`
5. Cleans up temporary files

### Step 2: Verify migration

```bash
cargo run --bin akar-cli -- /path/to/new/rust-db
```

Then run sample queries to verify data integrity.

### Manual export (skip Python extraction)

If you already have exported Parquet files and `schema.json`:

```bash
cargo run --bin akar-migrate -- --from /path/to/schema-dir --to /path/to/new/rust-db --skip-extract
```

## API Migration

### 1. Database Initialization

**C++:**
```cpp
auto systemConfig = kuzu::main::SystemConfig();
auto db = std::make_unique<kuzu::main::Database>("path/to/db", systemConfig);
```

**Rust:**
```rust
use akar_main::{Database, SystemConfig};
use std::sync::Arc;
let config = SystemConfig::default();
let db = Arc::new(Database::new("path/to/db", config)?);
```

### 2. Connection Management

**C++:**
```cpp
auto conn = std::make_unique<kuzu::main::Connection>(db.get());
```

**Rust:**
```rust
use akar_main::Connection;
let conn = Connection::new(&db);
```

**New in v0.3.4:** Connection pooling
```rust
use akar_main::{Database, ConnectionPool};
let pool = ConnectionPool::new(db.clone(), 16); // 16 connections
let conn = pool.get()?;
```

### 3. Query Execution & Results

**C++:**
```cpp
auto result = conn->query("MATCH (a:person) RETURN a.name");
while (result->hasNext()) {
    auto tuple = result->getNext();
    std::cout << tuple->getValue(0)->getValue<std::string>() << std::endl;
}
```

**Rust:**
```rust
let result = conn.query("MATCH (a:person) RETURN a.name")?;
for chunk in &result.chunks {
    for field_idx in 0..chunk.fields.len() {
        for row in 0..chunk.size {
            if let Some(val) = chunk.get_value(field_idx, row) {
                println!("{:?}", val);
            }
        }
    }
}
```

### 4. Prepared Statements & Parameters

**C++:**
```cpp
auto preparedStatement = conn->prepare("MATCH (a:person {name: $name}) RETURN a.age");
auto result = conn->execute(preparedStatement.get(), std::make_pair("name", "Alice"));
```

**Rust:**
```rust
let stmt = conn.prepare("MATCH (a:person {name: $name}) RETURN a.age")?;
let result = conn.execute(&stmt, vec![("name", "Alice".into())])?;
```

### 5. Bulk Operations (v0.3.4)

**Rust (new):**
```rust
use akar_main::bulk::BulkInserter;
let mut inserter = BulkInserter::new(&conn, "Person");
for person in people {
    inserter.add_row(vec![("name", person.name.into()), ("age", person.age.into())])?;
}
inserter.flush()?;
```

### 6. Error Handling

Rust uses `Result<T, akar_common::Error>` instead of exceptions. Use `?` or `match`:

```rust
conn.query("...").map_err(|e| anyhow::anyhow!("Query failed: {}", e))?;
```

## Extension Differences

Extensions in Rust are compiled statically via Cargo features:

```toml
[dependencies]
akar-main = { version = "0.3.4", features = ["json-extension", "fts-extension", "vector-extension"] }
```

This replaces the C++ `LOAD 'extensions/JSON'` runtime loading model.

**Extension list (15):**
- `json-extension` — `json_extract`, `json_valid`, `json_type`, `json_structure`, `json_contains`
- `fts-extension` — Stemmer, tokenizer, TF-IDF, BM25, stop words
- `vector-extension` — HNSW index, vector similarity search
- `algo-extension` — PageRank, WCC, SCC, K-Core, Louvain, spanning forest
- `httpfs-extension` — HTTP/HTTPS/S3 file reads
- `duckdb-extension` — `duckdb_query`, `duckdb_scan`
- `sqlite-extension` — `sqlite_query`, `sqlite_scan`
- `postgres-extension` — `sql_query`
- `neo4j-extension` — Bolt protocol integration
- `llm-extension` — LLM function integration
- `delta-extension` — `delta_scan`
- `iceberg-extension` — `iceberg_scan`, `iceberg_metadata`
- `azure-extension` — `azure_scan` (abfss:// URI)
- `unity-catalog-extension` — `uc_scan`
- `markdown-extension` — `read_markdown_wiki` (3 columns: `node`, `rel`, `metadata`)

## Application Integration

### Python API (Legacy / Frozen)

```bash
pip install akar==0.2.5  # PyPI package FROZEN at 0.2.5 (Constitution Principle VI)
```

**Note:** No new Python releases. Use Rust crate embedding (`akar-main`) for active development.

### Node.js / WASM API

Use the [`akar-wasm`](https://github.com/mlengse/akar) bindings (`AkarDatabase` wrapper) — Akar tidak menerbitkan package npm terpisah.

### CLI

```bash
cargo install akar-cli
akar-cli /path/to/db
```

Or download the prebuilt binary from [GitHub Releases](https://github.com/mlengse/akar/releases).

## Architecture Changes

| Aspect | C++ (Legacy) | Rust |
|--------|-------------|------|
| Build | CMake + 28 vendored libs | `cargo build` |
| Safety | Manual memory | Ownership + borrow checker |
| Storage | ValueVector | Native Arrow arrays |
| Operators | Split (BUILD/PROBE) | Fused single operators |
| Extensions | `.so`/`.dll` runtime loading | Static via Cargo features |
| Thread safety | Manual | `Send`/`Sync` guaranteed |
| Error handling | Exceptions | `Result<T, Error>` |
| Function lookup | Case-sensitive | **Case-insensitive** (`to_lowercase()`) |
| Optimizer passes | 17 | **26** (19 flat + 7 tree) |

### Performance Parity

| Query pattern | Rust (release) | C++ (Vela) |
|---------------|-------------|-------------------|
| `MATCH ... WHERE age > 30 RETURN COUNT(p)` | ~397 µs | ~400 µs |
| `MATCH ... RETURN SUM(p.age)` | ~435 µs | — |
| `MATCH ... RETURN AVG(p.score)` | ~424 µs | — |
| `MATCH ... RETURN MIN(p.age), MAX(p.age)` | ~587 µs | — |
| `GROUP BY active RETURN COUNT(p), AVG(p.score)` | ~600 µs | — |

## Feature Flags (Extensions)

Enable extensions in `Cargo.toml`:

```toml
akar-main = { version = "0.3.4", features = [
    "json-extension",
    "fts-extension",
    "vector-extension",
    "algo-extension",
    "httpfs-extension",
    "duckdb-extension",
    "sqlite-extension",
    "postgres-extension",
    "delta-extension",
    "iceberg-extension",
    "azure-extension",
    "unity-catalog-extension",
    "neo4j-extension",
    "llm-extension",
] }
```

## Performance

Run benchmarks to compare:

```bash
# Rust
cargo bench --workspace

# C++ (requires CMake build)
make benchmark
./build/release/tools/benchmark/kuzu_benchmark --dataset=... --benchmark=...
```

Current status: **Rust at parity with C++** (397 µs Rust vs 400 µs C++ for filter+count).

## Troubleshooting

| Issue | Solution |
|-------|----------|
| `akar-migrate` Python step fails | Install `pip install akar==0.2.5`, verify Python 3.8+ |
| Missing extensions | Enable feature flag in Cargo.toml |
| Slow queries | `cargo build --release`, verify Arrow-native execution |
| Windows ETW profiling | Run `cargo-flamegraph` as Administrator |
| Function not found | FunctionRegistry is case-insensitive — use exact casing as registered |

## Known Issues & Workarounds

| Issue | Workaround |
|-------|-----------|
| Postgres wire protocol `pg_isready` | Handled gracefully (returns TRUE) |
| Runtime extension loading | Use Cargo features instead |
| Windows ETW profiling | Run `cargo-flamegraph` as Administrator |
| Large databases | Increase buffer pool: `SystemConfig { buffer_pool_size: 1 << 30, .. }` |
| PyPI akar frozen at 0.2.5 | Embed `akar-main` via Rust; no new Python releases |
| `akar-server` deleted | Use `akar-main` embedded; no TCP server |
| `read_markdown_wiki` returns 3 columns | Update queries expecting 2 columns (`node`, `rel`) to handle `metadata` column |

## PyPI & Legacy Repositories

| Artifact | Status |
|----------|--------|
| PyPI `akar` | **FROZEN** at `0.2.5` (2026-08-24) — Constitution Principle VI |
| `kairos/` repo | **ARCHIVED** to `_archive/kairos_legacy_20260902_204025` — Constitution Principle I |
| `akar-server` binary | **DELETED** — use `akar-main` embedded; no TCP server |
| `sulur` Python package | **RETIRED** — pure-Rust `sulur-server` + `sulur-mcp` only |

See `.specify/memory/constitution.md` Principles I, IV, VI for governance.