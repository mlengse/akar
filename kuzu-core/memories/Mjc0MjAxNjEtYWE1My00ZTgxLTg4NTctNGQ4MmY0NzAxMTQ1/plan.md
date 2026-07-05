# Plan: Kuzu Rust — Fase P1: Table Functions (Catalog Inspection)

**TL;DR:** Aktifkan 12 CALL table functions untuk inspeksi catalog/schema. Infrastructure CALL sudah lengkap (Parser→Binder→Planner→Connection handler), tinggal implementasi handler per fungsi di `connection.rs`. Target: 12-16 jam.

---

## Background

Flow CALL yang sudah ada (dari P0 — berfungsi penuh):
```
CALL show_tables() 
  → Pest parser: CallStatement { function_name, args }
  → Binder: BoundCall { function_name, args: Vec<Expression> }
  → Planner: LogicalTableFunctionCall
  → Connection.query(): match fn_lower { "show_tables" => catalog.all_entries() }
  → QueryResult: Vec<DataChunk>
```

`show_tables` sudah berfungsi — jadi template untuk 11 fungsi lain.

Catalog di `kuzu-catalog/src/lib.rs` sudah punya: `all_entries()`, `node_tables()`, `rel_tables()`, `sequences()`, `macros()`, `vector_indexes()`, `foreign_tables()`, `get_entry_by_name()`, `contains()`.

---

## Fase 1: High-Priority (8 items, ~8-10 jam)

### 1.1 `table_info(table_name)` — Column metadata
Handler di connection.rs: `catalog.get_entry_by_name(name)?.columns()` → iterasi `ColumnDefinition` → DataChunk output (table_name, column, type, nullable).

**Modify:** `kuzu-main/src/connection.rs`

### 1.2 `show_functions()` — List all registered functions
Tambah `FunctionRegistry::list_all() -> Vec<(String, String)>` di `registry.rs`. Handler di connection.rs panggil method ini.

**Modify:** `kuzu-function/src/registry.rs`, `kuzu-main/src/connection.rs`

### 1.3 `show_indexes()` — List ART + Vector indexes  
Tambah `Catalog::indexes()` method. Handler iterasi hasilnya → DataChunk (index_name, table_name, type, column).

**Modify:** `kuzu-catalog/src/lib.rs`, `kuzu-main/src/connection.rs`

### 1.4 `show_sequences()` — List sequences
Handler: `catalog.sequences()` → DataChunk (seq_name, current_val).

**Modify:** `kuzu-main/src/connection.rs`

### 1.5 `show_macros()` — List scalar macros
Handler: `catalog.macros()` → DataChunk (macro_name, default_args).

**Modify:** `kuzu-main/src/connection.rs`

### 1.6 `show_connection(table_name)` — Node/rel topology
Tambah `CatalogEntry::connection_info()` method. Node table → (label=NODE), Rel table → (src_table, dst_table).

**Modify:** `kuzu-catalog/src/lib.rs`, `kuzu-main/src/connection.rs`

### 1.7 `db_version()` — Library version
Handler: return `env!("CARGO_PKG_VERSION")`.

**Modify:** `kuzu-main/src/connection.rs`

### 1.8 `catalog_version()` — DDL counter
Tambah `version: AtomicU64` di Catalog, increment di setiap DDL method. Handler return version.

**Modify:** `kuzu-catalog/src/lib.rs`, `kuzu-main/src/connection.rs`

---

## Fase 2: Medium-Priority (4 items, ~4-6 jam)

### 2.1 `current_setting(key)` — Config value
Handler: baca `self.database.config` fields → DataChunk (key, value).

**Modify:** `kuzu-main/src/connection.rs`

### 2.2 `stats_info(table_name)` — Table statistics
Tambah `StatsStore::table_stats(name)` method. Handler query row count + storage size.

**Modify:** `kuzu-storage/src/stats.rs`, `kuzu-main/src/connection.rs`

### 2.3 `storage_info()` — Page/storage stats
Tambah `StorageManager::storage_info()` method. Handler query PageManager::total_pages() + FSM stats.

**Modify:** `kuzu-storage/src/lib.rs`, `kuzu-main/src/connection.rs`

### 2.4 `show_attached_databases()` — Attached DBs
Handler sederhana: return main database info saja (single-DB untuk sekarang).

**Modify:** `kuzu-main/src/connection.rs`

---

## Shared Helpers (Fase 0, ~30 min)

Tambahkan dua helper functions di `connection.rs`:
- `extract_arg_string(args, index) -> Result<String>` — ekstrak string argumen ke-i
- `rows_to_datachunk(rows, column_names) -> DataChunk` — konversi `Vec<Vec<Value>>` → DataChunk

---

## Relevant Files

| File | Change |
|------|--------|
| `kuzu-main/src/connection.rs` | 12 handler baru + 2 helpers |
| `kuzu-catalog/src/lib.rs` | `indexes()`, `connection_info()`, `version` counter |
| `kuzu-function/src/registry.rs` | `list_all()` method |
| `kuzu-storage/src/stats.rs` | `table_stats()` method |
| `kuzu-storage/src/lib.rs` | `storage_info()` method |

---

## Verification

1. `test_call_table_info` — CREATE TABLE → CALL → verify columns
2. `test_call_show_functions` — CALL → verify >100 functions listed
3. `test_call_show_indexes` — CREATE INDEX → CALL → verify listed
4. `test_call_show_sequences` — CREATE SEQUENCE → CALL → verify listed
5. `test_call_show_macros` — CREATE MACRO → CALL → verify listed
6. `test_call_show_connection` — CREATE REL TABLE → CALL → verify src/dst
7. `test_call_db_version` — CALL → non-empty version string
8. `test_call_catalog_version` — DDL → CALL → version incremented
9. `test_call_current_setting` — CALL → spill_threshold value
10. `test_call_stats_info` — INSERT → CALL → row_count > 0
11. `cargo test --workspace` — 905+ tests pass, 0 regressions
12. `cargo clippy --workspace` — 0 new warnings

---

## Decisions

- **Handler location:** Semua di `connection.rs` match arm (seperti `show_tables`), bukan FunctionRegistry callback. Catalog inspection = read-only synchronous.
- **Output format:** `Vec<Vec<Value>>` → `rows_to_datachunk` helper. Column names hardcoded.
- **Error handling:** Table not found / missing args → `Err(String)`.
- **Out of scope:** `project_cypher_graph`, `project_native_graph` (butuh GDS), `bm_info`, `cache_column`, `clear_warnings`, `file_info`, `free_space_info`, `show_warnings`, `show_loaded_extensions`, `show_official_extensions`, `disk_size_info`, `storage_version` (low-priority debug/internal functions).
- **`show_tables` enhancement:** Sudah ada tapi hanya return nama. Enhance return (name, type).

---

## Follow-up Phases

- **Fase P2:** TransactionContext (AUTO/MANUAL), checkpoint worker, conflict detection — 13-18 jam
- **Fase P3:** HashJoin/Aggregate pipelines, DDL operators, COPY pipeline — 28-39 jam
- **Fase P4:** Ladybug passes, ANALYZE, PERCENTILE — 11-16 jam
- **Fase P5:** CI/CD, PlanPrinter, ClientContext, column specializations — 18-25 jam
