# Migrating to Akar Rust (v1.0.0)

This guide covers migration from the legacy C++ Kuzu API to the pure Rust `akar-core`.

## Why Migrate?

The Rust port offers:
- **~100% functional parity** with C++ (1,343 workspace tests: 1,342 passing, 1 failing `test_count_variable` = P48.14)
- **Memory safety** via Rust's ownership model
- **Arrow-native execution** — up to 24x faster filtering/numeric expressions
- **Operator fusing** — fewer physical nodes, less overhead
- **Simpler builds** — `cargo build` instead of CMake + 28 vendored C libraries
- **Cross-platform** — Linux, macOS, Windows, WASM
- **Extensible** — 15+ extensions (JSON, FTS, Vector, DuckDB, SQLite, Postgres, etc.)

## Data Migration

### Prerequisites
```bash
pip install kuzu  # or ladybug (C++ fork)
```

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
let config = SystemConfig::default();
let db = Database::new("path/to/db", config)?;
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
    for row in chunk.iter_rows() {
        if let Some(val) = chunk.fields[0].get_value(row) {
            println!("{:?}", val);
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
akar-main = { git = "...", features = ["json-extension", "httpfs-extension"] }
```

This replaces the C++ `LOAD 'extensions/JSON'` runtime loading model.

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
| `akar-migrate` Python step fails | Install `pip install kuzu`, verify Python 3.8+ |
| Missing extensions | Enable feature flag in Cargo.toml |
| Slow queries | `cargo build --release`, verify Arrow-native execution |
| Windows ETW profiling | Run `cargo-flamegraph` as Administrator |
