# Deep Exploration — akar-main

`akar-main` is the wrapper/facade over the whole engine: the public API surface (`Database`, `Connection`), the bootstrap that wires catalog + storage + transaction + memory manager + extensions into a working database, and the five-stage query orchestration (parse → bind → plan → optimize → execute). Both embedded usage (`Database::new`) and host bindings (CLI/Python/Wasm/C/server) go through this crate.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `Database` | Owns storage, catalog, txn manager, memory manager, extension registry; `Database::new` | `akar-core/akar-main/src/database.rs:531-664` |
| `register_builtin_extensions` | Registers all extension crates under feature flags | `akar-core/akar-main/src/database.rs:667-773` |
| `Connection` | Per-thread session: TxnResources, plan cache (capacity 100) | `akar-core/akar-main/src/connection/mod.rs` |
| `query` pipeline | parse → bind → plan → optimize → execute | `akar-core/akar-main/src/connection/query.rs:21-114` |
| prepared statement path | prepare → substitute → execute with param binding | `akar-core/akar-main/src/connection/query.rs:317-448` |
| `SystemConfig` | buffer_pool_size, checkpoint_threshold, enable_external_database, home_directory, specific_storage_version | `akar-core/akar-main/src/database.rs` |

## Design Decisions

- **Facade owns dependencies, not composition.** `Database` holds the `StorageManager`, `ExtensionRegistry`, transaction, and memory manager. Host bindings only ever see `Database` + `Connection`, which is why CLI/Python/C/Wasm/server are thin. (This is the "akar-main = wrapper" pattern from the crate's README.)
- **Plan cache keyed by query text + catalog version.** Repeated identical statements (agent loops over the same MATCH) reuse cached physical plans instead of re-optimizing. The cache is bounded (100) to avoid unbounded growth.
- **Features gate extensions.** `register_builtin_extensions` conditionally includes each extension crate (json, duckdb, sqlite, ...) behind Cargo features, so release builds without `--all-features` ship smaller binaries (and skip compiling C++ duckdb/sqlite, matching the `check [akar-core]` config).

## Why It Matters

Everything users touch starts here: `Database::new(path)` opens/creates a DB, `Connection::query(sql)` runs it, `LOAD EXTENSION` routes through standalone_call. The five-stage orchestration in `query.rs` is the single funnel where all correctness mechanisms — resolution, planning, optimization, execution — meet, making it the natural place for feature-level integration tests (network `test_httpfs` runs from this crate).