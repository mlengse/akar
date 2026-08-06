# Akar Migration Guide

Welcome to **Akar**! This guide details how to transition your application from the legacy C++ implementation (KuzuDB Vela/LadybugDB) to the pure Rust Akar database.

As of August 2026, the Rust port has achieved **~100% functional parity** with the C++ version — **1,346 tests** (1,345 passing + 1 failing `test_count_variable` = P48.14), all 15 GDS algorithms, **25 optimizer passes**, and 33 parser statement variants (+ 10 clause sub-variants) are implemented.

## 1. Quick Migration (Data)

The `akar-migrate` CLI tool handles the full C++ → Rust data migration:

```bash
# Install Python deps (required for reading C++ DB)
pip install kuzu

# Run migration
cargo run --bin akar-migrate -- --from /path/to/legacy/cpp-db --to /path/to/new/rust-db
```

This exports schema + data as Parquet, creates the Rust database, and imports everything.

### Skip extraction (if you already have Parquet files)
```bash
cargo run --bin akar-migrate -- --from /path/to/export-dir --to /path/to/new/rust-db --skip-extract
```

## 2. Application Integration

### Rust API

Update your `Cargo.toml`:
```toml
[dependencies]
akar = { git = "https://github.com/anjangkusumanetra/akar", branch = "main" }
```

### Basic Connection
```rust
use akar::{Database, Connection};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::new("path/to/new_db")?;
    let conn = Connection::new(&db)?;
    let result = conn.query("MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name")?;
    for row in result {
        println!("{} knows {}", row.get("a.name")?, row.get("b.name")?);
    }
    Ok(())
}
```

### Python API
```bash
pip install akar-rust  # or build from source with `make python`
```

### Node.js API
```bash
npm install @vela-engineering/kuzu  # or use the akar-wasm bindings (AkarDatabase wrapper)
```

### CLI
```bash
cargo install akar-cli
akar-cli /path/to/db
```

Or download the prebuilt binary from [GitHub Releases](https://github.com/anjangkusumanetra/akar/releases).

## 3. Architecture Changes

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

## 4. Feature Flags (Extensions)

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

## 5. Performance Tuning

```bash
# Run benchmark suite
cargo bench --workspace

# Individual crate benchmarks
cargo bench -p akar-processor
cargo bench -p akar-main

# C++ comparison (requires CMake)
make benchmark
./build/release/tools/benchmark/kuzu_benchmark --dataset=... --benchmark=...
```

**Status:** Rust is at parity with C++ for scalar aggregates (397 µs vs 400 µs for filter+count). GROUP BY queries also at parity with Arrow compute fast paths.

## 6. Known Issues & Workarounds

| Issue | Workaround |
|-------|-----------|
| Postgres wire protocol `pg_isready` | Handled gracefully (returns TRUE) |
| Runtime extension loading | Use Cargo features instead |
| Windows ETW profiling | Run `cargo-flamegraph` as Administrator |
| Large databases | Increase buffer pool: `SystemConfig { buffer_pool_size: 1 << 30, .. }` |
