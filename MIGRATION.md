# Migrating to Akar Rust

This guide covers migration from the legacy C++ Kuzu API to the pure Rust `akar-core`.

## Why Migrate?

The Rust port offers:
- **~100% functional parity** with C++ (gate `test [akar-core]`: 1,872 passed / 0 failed / 0 ignored per 2026-08-24 — lihat CHANGELOG untuk angka terkini)
- **Memory safety** via Rust's ownership model
- **Arrow-native execution** — up to 24x faster filtering/numeric expressions
- **Operator fusing** — fewer physical nodes, less overhead
- **Simpler builds** — `cargo build` instead of CMake + 28 vendored C libraries
- **Cross-platform** — Linux, macOS, Windows, WASM
- **Extensible** — 15+ extensions (JSON, FTS, Vector, DuckDB, SQLite, Postgres, etc.)

## Data Migration

### Prerequisites
```bash
pip install akar
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

### 5. Error Handling
Rust uses `Result<T, akar_common::Error>` instead of exceptions. Use `?` or `match`:
```rust
conn.query("...").map_err(|e| anyhow::anyhow!("Query failed: {}", e))?;
```

## Extension Differences

Extensions in Rust are compiled statically via Cargo features:
```toml
[dependencies]
akar-main = { version = "0.1.0", features = ["json-extension", "httpfs-extension"] }
```

This replaces the C++ `LOAD 'extensions/JSON'` runtime loading model.

## Application Integration

### Python API
```bash
pip install akar  # PyPI package (Python bindings via maturin)
```

### Node.js API
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

### Performance Parity

| Query pattern | Rust (debug) | Rust (est. release) | C++ (Vela) |
|---------------|-------------|-------------------|------------|
| `MATCH ... WHERE age > 30 RETURN COUNT(p)` | ~4 ms | ~400 µs | ~400 µs |
| `MATCH ... RETURN SUM(p.age)` | ~6 ms | ~435 µs | — |
| `MATCH ... RETURN AVG(p.score)` | ~6 ms | ~424 µs | — |
| `MATCH ... RETURN MIN(p.age), MAX(p.age)` | ~9 ms | ~587 µs | — |
| `GROUP BY active RETURN COUNT(p), AVG(p.score)` | ~9 ms | ~600 µs | — |

## Feature Flags (Extensions)

Enable extensions in `Cargo.toml`:
```toml
akar-main = { features = [
    "json-extension",
    "fts-extension",
    "vector-extension",
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
    "algo-extension",
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
| `akar-migrate` Python step fails | Install `pip install akar`, verify Python 3.8+ |
| Missing extensions | Enable feature flag in Cargo.toml |
| Slow queries | `cargo build --release`, verify Arrow-native execution |
| Windows ETW profiling | Run `cargo-flamegraph` as Administrator |

## Known Issues & Workarounds

| Issue | Workaround |
|-------|-----------|
| Postgres wire protocol `pg_isready` | Handled gracefully (returns TRUE) |
| Runtime extension loading | Use Cargo features instead |
| Windows ETW profiling | Run `cargo-flamegraph` as Administrator |
| Large databases | Increase buffer pool: `SystemConfig { buffer_pool_size: 1 << 30, .. }` |
