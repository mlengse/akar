# Kuzu Rust Migration Guide

Welcome to **Kuzu Rust**! This guide details how to transition your application from the legacy C++ implementation (Vela/LadybugDB) to the new, memory-safe, Arrow-native Rust implementation.

As of July 2026, the Rust port has achieved **~100% functional parity** with the C++ version, including all 15 GDS algorithms, the full suite of 22 optimizer passes, and complex parser/binder statements. 

## 1. Core Differences & Architecture Changes

Before migrating, please note the following structural changes:
- **Embedded Only:** Kuzu Rust remains an embedded database. However, connection pooling and thread management are now inherently safe via Rust's `Send`/`Sync` models.
- **Operator Fusing:** In the C++ version, physical operators were highly split (e.g., `HASH_JOIN_BUILD` and `HASH_JOIN_PROBE` were separate physical nodes). In the Rust port, these are often **fused** into single, highly-optimized physical operators.
- **Arrow-Native Execution:** The underlying storage vectors (`ValueVector`) are being transitioned to native Apache Arrow arrays (`ArrayRef`), providing massive speedups (up to 24x) for filtering and numeric expressions.
- **Dependency Reductions:** String and Blob processing are now handled via pure-Rust crates (`base64`, `regex`, `sha2`, `md-5`) rather than massive C++ libraries.

## 2. Using the Read-Only Migration Utility

If you have existing C++ database files, we provide a read-only CLI utility in `kuzu-migrate` to read your legacy data and optionally export it to CSV/Parquet for loading into the Rust version.

### Step 2.1: Exporting Legacy Data
Compile and run the migration utility against your old C++ database directory:
```bash
cargo run -p kuzu-migrate -- --db-path /path/to/legacy/db --export-dir /path/to/export
```
This tool will iterate over all node and relationship tables and write them as Arrow IPC or Parquet files.

### Step 2.2: Importing into Kuzu Rust
Use the native `COPY` statement in the new Rust shell to ingest the exported data:
```cypher
CREATE NODE TABLE User(id INT64, name STRING, PRIMARY KEY (id));
COPY User FROM '/path/to/export/user.parquet';
```

## 3. Application Integration (Rust API)

If you are calling Kuzu from a Rust application, update your `Cargo.toml`:
```toml
[dependencies]
kuzu = { git = "https://github.com/kuzudb/kuzu-rust", branch = "main" }
```

### Basic Connection Example
```rust
use kuzu::{Database, Connection};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialize the database
    let db = Database::new("path/to/new_db")?;
    
    // 2. Open a connection
    let conn = Connection::new(&db)?;
    
    // 3. Execute a query
    let result = conn.query("MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name")?;
    
    // 4. Iterate over results
    for row in result {
        println!("{} knows {}", row.get("a.name")?, row.get("b.name")?);
    }
    
    Ok(())
}
```

## 4. Known Workarounds & Driver Compatibility

- **Postgres Compatibility Pings:** If you are using an ORM or driver that expects a Postgres wire protocol and attempts to call `pg_isready()`, Kuzu Rust handles this gracefully by returning a static `TRUE`.
- **Extension Loading:** The `LOAD EXTENSION` syntax is parsed, but extensions in Rust are compiled statically via Cargo features (e.g., `features = ["json-extension", "httpfs-extension"]`) rather than `.so`/`.dll` runtime loading.

## 5. Performance Tuning (Profiling)

If you experience latency regressions compared to C++, note that the transition to Arrow-native `ValueVector` is ongoing. To run benchmarks locally:
```bash
cargo bench -p kuzu-processor
cargo bench -p kuzu-main
```
*(Note for Windows users: Low-level ETW profiling via `cargo-flamegraph` may require execution from an elevated Administrator terminal).*
