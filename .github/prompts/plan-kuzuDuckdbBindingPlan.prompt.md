# Plan: DuckDB Binding + 6 Extension Crates + Storage Cardinality + Previously Excluded Scope

## TL;DR

Complete the Kuzu Rust port by: (1) adding callback bridge to Extension trait so extensions can execute real Rust code, (2) implementing a real DuckDB Rust binding using `duckdb` crate v~1.105, (3) porting 6 remaining extensions using **native Rust crates** (rusqlite, tokio-postgres, deltalake, iceberg-rust, azure_storage, reqwest) — NOT via DuckDB delegation, (4) implementing storage-backed cardinality estimation + join order enumeration, (5) PreparedStatement support, (6) tools/rust_api integration review.

**Critical architecture bottleneck discovered:** The current `Extension` trait + function registry (`ScalarFunction`/`TableFunction`/`AggregateFunction` enums) is **purely declarative** — no callback/closure mechanism exists. Even existing extensions like `kuzu-algo` register functions as `TableFunction::Custom { name }` string tags with **no bridge to actual Rust execution**. Fixing this is required before any extension can actually execute queries.

---

## Phase A: Architecture — Callback Bridge (Blocking dependency for ALL extensions)

### Steps

**A1. Add `CustomScalar` variant to `ScalarFunction` enum** (`kuzu-function/src/registry.rs`)
- `CustomScalar { name: String, execute: Box<dyn Fn(&[Value]) -> Result<Value, String> + Send + Sync> }`
- Stores a closure: slice of input `Value`s → output `Value`

**A2. Add `CustomTable` variant to `TableFunction` enum**
- `CustomTable { name: String, execute: Box<dyn Fn(&[Value], &mut DataChunk) -> Result<(), String> + Send + Sync> }`
- Stores a closure: input args + mutable `DataChunk` to fill

**A3. Wire `CustomScalar` dispatch into `evaluate_scalar`** (`kuzu-function/src/scalar.rs`)
- Match arm calls the stored closure

**A4. Wire `CustomTable` dispatch into processor** (`kuzu-processor/src/`)
- `PhysicalScan` operator detects `TableFunction::CustomTable` and calls the closure

**A5. Refactor existing extensions to use callbacks** (optional cleanup)
- `kuzu-algo`: Wrap `compute_page_rank` etc. into `CustomTable` closures
- `kuzu-json`, `kuzu-fts`, `kuzu-httpfs`, `kuzu-duckdb`: Replace placeholder `UtilityOp::Coalesce` / `ScanJson` with real `CustomScalar`/`CustomTable` closures

**Dependency:** A1→A2→A3→A4 (sequential). A5 parallel with Phase B once A4 is done.

---

## Phase B: DuckDB Rust Binding (*depends on Phase A*)

**B1. Add `duckdb` crate dependency** to `kuzu-duckdb/Cargo.toml`
```toml
duckdb = { version = "~1.105", features = ["bundled"] }
```

**B2. Implement DuckDB connection manager** (`kuzu-duckdb/src/connection.rs`)
- `DuckDbManager`: holds `duckdb::Database` + `duckdb::Connection`
- Methods: `open(path, mode)`, `query(sql)`, `execute(sql)`
- 3 modes: Local (`Connection::open`), In-memory (`open_in_memory`), Remote HTTP/S3 (`INSTALL httpfs; LOAD httpfs;`)

**B3. Implement type converter** (`kuzu-duckdb/src/type_converter.rs`)
- DuckDB logical types → Kuzu `LogicalType`
- DuckDB values → Kuzu `Value`
- INT32→INT32, INT64→INT64, VARCHAR→STRING, DOUBLE→DOUBLE, BOOL→BOOL, etc.

**B4. Implement result converter** (`kuzu-duckdb/src/result_converter.rs`)
- `duckdb::DataChunkHandle` → Kuzu `DataChunk`
- Arrow `RecordBatch` as intermediary if needed

**B5. Replace placeholders with real callbacks** (`kuzu-duckdb/src/lib.rs`)
- `duckdb_query` → `CustomScalar` that executes SQL via DuckDB
- `duckdb_scan` → `CustomTable` that scans DuckDB table

**B6. Rewrite tests**: in-memory DuckDB round-trips

---

## Phase C: 6 Extension Crates (*depends on Phase A only; sequential after Phase B*)

**Strategy hybrid per extension berdasarkan kematangan native crate:**

| # | Extension | Approach | Crate | Functions | Estimasi |
|---|-----------|----------|-------|-----------|----------|
| **C1** | `kuzu-sqlite` | **Native** | `rusqlite` | Storage ext (ATTACH `.db`) | ~250 baris |
| **C2** | `kuzu-delta` | **DuckDB crate** | `duckdb` (bundled) | 1 table fn: `delta_scan(path)` | ~150 baris |
| **C3** | `kuzu-iceberg` | **DuckDB crate** | `duckdb` (bundled) | 3 table fn: `scan`, `metadata`, `snapshots` | ~200 baris |
| **C4** | `kuzu-azure` | **DuckDB crate** | `duckdb` (bundled) | 1 table fn: `azure_scan(path)` | ~150 baris |
| **C5** | `kuzu-postgres` | **Native** | `tokio-postgres` + `block_on` | Storage ext + `sql_query()` | ~500 baris |
| **C6** | `kuzu-unity-catalog` | **DuckDB crate** | `duckdb` (bundled) | Storage ext + token/endpoint | ~200 baris |

**Key design decisions per extension:**

**C1 (SQLite) — Native `rusqlite`:**
- `rusqlite::Connection::open(path)` → execute query → convert `rusqlite::Rows` → Kuzu `DataChunk`
- Storage extension: register ATTACH handler → `rusqlite::Connection` → enumerate tables
- `sqlite_all_varchar` option: force all columns to STRING type
- **No DuckDB dependency**

**C2 (Delta) — DuckDB crate:**
- Reuse `DuckDbManager` dari Phase B
- In-memory DuckDB → `INSTALL delta; LOAD delta;` → `SELECT * FROM delta_scan('path')`
- **Alasan**: `deltalake` native crate API masih berubah-ubah

**C3 (Iceberg) — DuckDB crate:**
- Reuse `DuckDbManager` dari Phase B
- In-memory DuckDB → `INSTALL iceberg; LOAD iceberg;`
- 3 functions via DuckDB SQL delegation
- Share scan helper module with Delta (C2)
- **Alasan**: `iceberg-rust` masih immature

**C4 (Azure) — DuckDB crate:**
- Reuse `DuckDbManager` dari Phase B
- In-memory DuckDB → `INSTALL azure; LOAD azure; INSTALL httpfs; LOAD httpfs;` → `CREATE SECRET`
- Support `az://` and `abfss://` URI schemes
- **Alasan**: Azure SDK for Rust masih developing

**C5 (PostgreSQL) — Native `tokio-postgres` + `block_on`:**
- `tokio-postgres` async → bungkus dengan `futures::executor::block_on()` (Kuzu sync runtime)
- Storage extension: attach PG → enumerate schemas/tables → catalog entries
- `sql_query(db_name, query)` → raw SQL ke PG → convert results
- **Most complex**: catalog binding + table enumeration + PG→Kuzu type mapping
- **Alasan**: `tokio-postgres` mature dan stabil. DuckDB `postgres` scanner juga dependency pada libpq yang sama.

**C6 (Unity Catalog) — DuckDB crate:**
- Reuse `DuckDbManager` dari Phase B
- In-memory DuckDB → `INSTALL uc_catalog; LOAD uc_catalog;` dari `core_nightly`
- `CREATE SECRET (TYPE UC, TOKEN '...', ENDPOINT '...')`
- `uc_token` and `uc_endpoint` config options
- **Alasan**: Tidak ada native Rust crate untuk Unity Catalog; DuckDB `uc_catalog` extension handle REST API

**Feature flags** di `kuzu-main/Cargo.toml`:
```toml
sqlite-extension = ["dep:kuzu-sqlite"]
delta-extension = ["dep:kuzu-delta"]
iceberg-extension = ["dep:kuzu-iceberg"]
azure-extension = ["dep:kuzu-azure"]
postgres-extension = ["dep:kuzu-postgres"]
unity-catalog-extension = ["dep:kuzu-unity-catalog"]
```

---

## Phase D: Storage Cardinality & Join Order Enumeration

### D1. Storage-engine-backed cardinality estimation
- **Current**: `CardinalityEstimation` pass uses static selectivity constants (PK→1, filter→0.01, etc.)
- **Upgrade**: Add `table_cardinality()` method to `kuzu-storage`'s `Table` trait via `StorageManager`
- Wire into optimizer: when estimating `ScanNode`/`ScanRel` cardinality, query actual table statistics from storage
- **Files**: `kuzu-storage/src/table.rs`, `kuzu-optimizer/src/passes.rs`, `kuzu-optimizer/src/cardinality_estimation.rs`
- **Risk**: Low — additive change, doesn't break existing estimates

### D2. Join order enumeration
- **Current**: `JoinOptimization` pass is a placeholder
- **Implement**: Greedy join order enumeration using cardinality estimates from D1
- Algorithm: Start with smallest relation, iteratively join with smallest-cost next relation
- **Files**: `kuzu-optimizer/src/passes.rs`, new `kuzu-optimizer/src/join_order.rs`
- **Risk**: Medium — requires D1 complete, non-trivial algorithm
- **Dependency**: D1 must be complete first

---

## Phase E: PreparedStatement + tools/rust_api

### E1. PreparedStatement
- `kuzu-main/src/prepared_statement.rs` (new)
- `kuzu-main/src/lib.rs` (extend `Connection` API)
- API: `connection.prepare(sql)` → `PreparedStatement`, `stmt.execute(params)`
- Implementation: store parsed/bound/planned query plan, accept parameter bindings at execution, re-plan if schema changed
- **Risk**: Low-Medium

### E2. tools/rust_api integration review
- `tools/rust_api/` (existing C++ FFI layer)
- Research & decide: (a) rewrite Rust API to call Rust native `kuzu-main`, or (b) keep as C++ FFI wrapper for backward compat
- **Recommendation**: Create thin `kuzu-ffi` crate if backward compat needed, otherwise deprecate
- **Risk**: Low — research/decisional

---

## Phase F: Cleanup & Integration

### F1. WASM guard
- `duckdb` crate cannot compile on wasm32
- Add `#[cfg(not(target_arch = "wasm32"))]` guards around all DuckDB-dependent extension registrations
- Gate `duckdb` dependency behind non-WASM cfg in `kuzu-duckdb/Cargo.toml`
- For native Rust extensions: `rusqlite` and `tokio-postgres` also don't support wasm32 → same cfg guards needed
- `deltalake` and `iceberg-rust` are pure Rust — may work on wasm32 (verify)

### F2. Integration tests
- Add end-to-end tests in `kuzu-main/tests/`:
  - DuckDB: create Kuzu DB, load DuckDB extension, attach DuckDB file, query
  - SQLite: create temp SQLite DB, attach from Kuzu, query
  - Delta/Iceberg/Azure/PG/UC: conditional tests (require external services or test data)
- Run: `cargo test --features "duckdb-extension,sqlite-extension,delta-extension,iceberg-extension,azure-extension,postgres-extension,unity-catalog-extension"`

### F3. Documentation
- Update `README.md` with supported extensions table
- Add extension-specific docs in each crate's `README.md`
- Document native crate choices and rationale

---

## Relevant Files

### Core Architecture (Phase A)
- `kuzu-core/kuzu-function/src/registry.rs` — add CustomScalar, CustomTable variants
- `kuzu-core/kuzu-function/src/scalar.rs` — dispatch CustomScalar
- `kuzu-core/kuzu-processor/src/` — dispatch CustomTable
- `kuzu-core/kuzu-extension/src/` — Extension trait (unchanged)

### DuckDB Binding (Phase B)
- `kuzu-core/kuzu-duckdb/Cargo.toml` — add `duckdb` dep
- `kuzu-core/kuzu-duckdb/src/lib.rs` — rewrite with real callbacks
- `kuzu-core/kuzu-duckdb/src/connection.rs` — new, DuckDbManager
- `kuzu-core/kuzu-duckdb/src/type_converter.rs` — new
- `kuzu-core/kuzu-duckdb/src/result_converter.rs` — new

### New Extension Crates (Phase C)
- `kuzu-core/kuzu-sqlite/Cargo.toml` + `src/lib.rs`
- `kuzu-core/kuzu-delta/Cargo.toml` + `src/lib.rs`
- `kuzu-core/kuzu-iceberg/Cargo.toml` + `src/lib.rs`
- `kuzu-core/kuzu-azure/Cargo.toml` + `src/lib.rs`
- `kuzu-core/kuzu-postgres/Cargo.toml` + `src/lib.rs`
- `kuzu-core/kuzu-unity-catalog/Cargo.toml` + `src/lib.rs`
- `kuzu-core/kuzu-main/Cargo.toml` — add 6 feature flags + optional deps
- `kuzu-core/kuzu-main/src/extensions.rs` — wire registrations

### Storage & Optimizer (Phase D)
- `kuzu-core/kuzu-storage/src/table.rs` — add table_cardinality()
- `kuzu-core/kuzu-storage/src/stats.rs` — existing table statistics
- `kuzu-core/kuzu-optimizer/src/passes.rs` — JoinOptimization rewrite
- `kuzu-core/kuzu-optimizer/src/join_order.rs` — new (greedy enumeration)
- `kuzu-core/kuzu-optimizer/src/cardinality_estimation.rs` — wire storage stats

### PreparedStatement & API (Phase E)
- `kuzu-core/kuzu-main/src/prepared_statement.rs` — new
- `kuzu-core/kuzu-main/src/lib.rs` — extend Connection

---

## Dependency Graph

```
Phase A (Callback Bridge)
├── Phase B (DuckDB Binding)
├── Phase C1–C6 (6 extensions) — parallel
│
Phase D1 (Storage Cardinality)
└── Phase D2 (Join Order Enumeration) — depends on D1
│
Phase E1 (PreparedStatement) — independent
Phase E2 (tools/rust_api) — independent
│
Phase F (Cleanup & Integration) — depends on all above
```

---

## Verification

1. **Phase A**: `cargo test -p kuzu-function -p kuzu-processor` — existing + new callback tests pass
2. **Phase B**: `cargo test -p kuzu-duckdb` — DuckDB in-memory queries work; type conversion round-trips
3. **Phase C**: `cargo test --features "sqlite-extension,delta-extension,iceberg-extension,azure-extension,postgres-extension,unity-catalog-extension"` — all extension tests pass
4. **Phase D1**: `cargo test -p kuzu-optimizer` — cardinality estimates use real table stats (test with populated DB)
5. **Phase D2**: `cargo test -p kuzu-optimizer` — join order changes based on cardinality (multi-relation query)
6. **Phase E1**: `cargo test -p kuzu-main` — PreparedStatement round-trips with params
7. **Phase F**: `cargo test --target wasm32-unknown-unknown` succeeds (native-code extensions excluded); full integration test with `--features "all-extensions"`

---

## Decisions

- **Callback bridge**: `Box<dyn Fn(...)>` closures di enum variants (approach paling sederhana, paling minimal invasive ke processor)
- **DuckDB crate (`bundled`)** → untuk `kuzu-duckdb` (Phase B) + Delta, Iceberg, Azure, Unity Catalog (Phase C2–C4, C6)
- **Native Rust crate** → untuk SQLite (`rusqlite`, C1) dan Postgres (`tokio-postgres` + `block_on`, C5)
- **WASM exclusion**: DuckDB crate, `rusqlite`, `tokio-postgres` tidak support wasm32 → cfg-gated. Extension berbasis DuckDB crate juga cfg-gated.
- **Implementation order**: A → B → C (sequential phases)
- **Scope excluded (confirmed)**: C++ code removal, CI/CD setup (out of scope per original plan)

---

## Further Considerations

1. **`tokio-postgres` + `block_on` untuk Postgres**: Kuzu runtime adalah sync (tidak punya tokio runtime). Setiap operasi PG perlu dibungkus:
   ```rust
   let rt = tokio::runtime::Runtime::new().unwrap();
   let client = rt.block_on(tokio_postgres::connect(conn_str, NoTls)).unwrap();
   let rows = rt.block_on(client.query(sql, &[])).unwrap();
   ```
   Perlu `tokio` dengan feature `rt` dan `rt-multi-thread`. `tokio-postgres` dengan feature `runtime` dan `blocking` juga. Biaya: setiap call buat runtime baru atau cache runtime. **Rekomendasi**: Cache `Runtime` sekali di `PostgresConnector`.

2. **Shared DuckDB helper module**: C2 (Delta), C3 (Iceberg), C4 (Azure), C6 (Unity Catalog) semuanya pakai pattern yang sama: in-memory DuckDB → install extension → delegate. Buat shared utility `duckdb_attach_helper` di `kuzu-duckdb/src/attach_helper.rs` untuk menghindari duplikasi. Fungsi: `attach_via_duckdb(name, install_sql, attach_sql) → DuckDbManager`.

3. **Extension dependency on kuzu-duckdb**: C2–C4 dan C6 perlu depend pada `kuzu-duckdb` (untuk `DuckDbManager` reusable). Buat `DuckDbManager` public API. Feature flag di `kuzu-main/Cargo.toml` perlu `dep:kuzu-duckdb` juga untuk extension-extension ini.

4. **Delta/Iceberg fitur C++ vs DuckDB crate**: C++ extension reuses `delta_extension::tableFunc` antar Delta dan Iceberg. Dengan DuckDB crate, pattern-nya:
   ```rust
   let duck = DuckDbManager::in_memory()?;
   duck.install_and_load("delta")?;
   let chunk = duck.query_table("delta_scan('path')")?;
   ```
   Cukup reuse `DuckDbManager` — tidak perlu shared table func seperti C++.
