> **SUPERSEDED** - Arsip per 2026-08-02. Dokumen snapshot audit/eksplorasi kondisi 17-19 Juli 2026; codebase sudah berubah signifikan (per 2026-08-02: 32 crate, ~86K LOC, 1,311 test, 25 optimizer passes). Untuk state saat ini lihat `akar-core\STATUS.md`, `akar-core\SPEC.md`, `akar-core\implementation_plan.md`, dan README per crate.

---

# Comprehensive Audit: Ladybug-Specific Features vs Original Kuzu C++ and Rust Port Status 19/07/2026

## 1. IceDisk / IceFormat (Parquet-Based Read-Only Storage)

### C++ Ladybug (`ladybug/src/`)
**Ladybug-specific files (not in original Kuzu):**
- `src/include/storage/table/ice_disk_constants.h` — Version metadata constants (`icebug_disk_version = v1`)
- `src/include/storage/table/ice_disk_utils.h` — Path construction, version validation
- `src/include/storage/table/ice_disk_node_table.h` — `IceDiskNodeTable` class (read-only Parquet-backed node table)
- `src/include/storage/table/ice_disk_rel_table.h` — `IceDiskRelTable` class (CSR/FLAT Parquet-backed rel table)
- `src/storage/table/ice_disk_node_table.cpp` — Full implementation of node table scanning from Parquet
- `src/storage/table/ice_disk_rel_table.cpp` — Full implementation of rel table scanning from Parquet
- `src/common/enums/storage_format.h` — `StorageFormat` enum with `ICEBUG_DISK` value (not in original Kuzu)
- `src/common/enums/storage_format.cpp` — String-to-enum parser

**Concept:** A read-only graph format where node tables are stored as `nodes_{tableName}.parquet` files (one column per property) and relationship tables use `indices_{tableName}.parquet` + `indptr_{tableName}.parquet` for CSR layout, or a single flat `.parquet` file for the FLAT layout. Tables are created with `CREATE ... WITH (storage = '<path>', format = 'icebug-disk')`. Immutable -- no inserts/updates/deletes/ALTER.

**Rust Port Status (`akar-core/akar-storage/src/ice_format.rs`):**
- **PARTIALLY PORTED.** The file contains basic data structures:
  - `IceDiskRelTableLayout` enum (Flat, Csr)
  - `IceDiskRelTable` struct (holds name, layout, paths)
  - `IceDiskRelTableScanState` struct with `ParquetStreamReader`
  - `new()`, `scan_indices()`, `scan_indptr()`, `next_row()` methods
- **Not ported:** `IceDiskNodeTable`, full scan logic, version validation, DDL binder integration, BinderException for mixed tables, semi-mask integration, degree/top-k queries, `getAllDegreeEntries`, `getTopKDegreeEntries`
- Status: Basic plumbing only, not the full production scanning logic.

---

## 2. Columnar Base Classes (Arrow + IceDisk Abstraction)

### C++ Ladybug (not in original Kuzu):
- `src/include/storage/table/columnar_node_table_base.h` — `ColumnarNodeTableBase` abstract class, `ColumnarNodeTableScanState`, `ColumnarNodeTableScanSharedState`
- `src/include/storage/table/columnar_rel_table_base.h` — `ColumnarRelTableBase` abstract class

These provide template-method-pattern abstractions shared by `IceDiskNodeTable`, `ArrowNodeTable`, `IceDiskRelTable`, `ArrowRelTable`. They disable insert/update/delete and define virtual methods like `getColumnarFormatName()`, `getNumBatches()`, `getTotalRowCount()`.

**Rust Port:** **NOT PORTED.** No equivalent abstract base classes exist. The `ice_format.rs` module is standalone with no `ColumnarNodeTableBase` or `ColumnarRelTableBase` hierarchy.

---

## 3. Morsel-Driven Scanning / Multi-Threaded Scan

### C++ Ladybug
Ladybug extends the original Kuzu's scan system with morsel-driven parallelism for columnar tables:
- `ArrowNodeTable` splits batches into smaller scan morsels (2048 rows, configurable via `scanMorselSize`)
- `IceDiskNodeTable` assigns Parquet row groups as morsels
- `ColumnarNodeTableScanSharedState::getNextMorsel()` virtual method
- `IceDiskNodeTableScanSharedState` (derived from `ColumnarNodeTableScanSharedState`) manages row group assignments
- Native tables use one node group (~128K rows) per morsel

**Key documentation:** `docs/morsel_parallelism.md`, `docs/morsel_scan.py`

**Rust Port:** **NOT PORTED.** The morsel-driven scan architecture (`ColumnarNodeTableScanSharedState`, per-thread morsel dispatching, Parquet row group morsels) does not exist in the Rust port. The `ice_format.rs` has a basic streaming reader but no morsel-based parallelism.

---

## 4. ART Index (Adaptive Radix Tree Primary Key Index)

### C++ Ladybug (not in original Kuzu):
- `src/include/storage/index/art_index.h` — `ArtPrimaryKeyIndex` class (full Node4/16/48/256 adaptive radix tree)
- `src/include/storage/index/art_index_disk_utils.h` — Disk serialization helpers
- `src/storage/index/art_index.cpp` — Core tree operations (insert, lookup, erase, collectRange, scanPrimaryKeyRange)
- `src/storage/index/art_index_disk.cpp` — Checkpoint/serialize/deserialize/reload

**DDL:** `CREATE ART INDEX person_pk FOR (p:Person) ON (p.ID);`

**Optimizer integration:** Filter pushdown optimizer can use ART range scans for constant inequality predicates (`p.ID > 10`).

**WAL integration:** `CREATE_INDEX_RECORD` WAL record type (value 15, not present in original Kuzu).

**Large index optimization:** When serialized tree > 256 MiB, uses blocking checkpoint instead of WAL to avoid duplicating gigabytes.

**Rust Port Status (`akar-core/akar-storage/src/art_index.rs`, `art_key.rs`, `art_node.rs`):**
- **PORTED.** Full implementation including:
  - ArtPrimaryKeyIndex with Node4/16/48/256 adaptive layouts
  - ArtKey encoding for all types (strings with escaping, fixed-width big-endian)
  - collectRange for bounded primary-key range scans
  - WAL serialization (CREATE_INDEX_RECORD supported in `wal.rs`)
  - Checkpoint/reload via `ArtPrimaryKeyIndexStorageInfo`
- Status: Production-ready

---

## 5. WAL Format Changes

### C++ Ladybug additions vs original Kuzu:
- **`CREATE_INDEX_RECORD = 15`** — New WAL record type (inserted between CREATE_CATALOG_ENTRY_RECORD=14 and DROP_CATALOG_ENTRY_RECORD=16)
- **`serializeWithLength()`** — New static method on WALRecord (added length-prefixed serialization)
- New sub-record files: `create_index_record.h`, `begin_transaction_record.h`, `checkpoint_record.h`, `commit_record.h`, etc. (refactored from monolithic `wal_record.h` into separate files in `src/include/storage/wal/record/`)
- Ladybug restructures WAL records from a single `wal_record.h` to individual files per record type

### Rust Port:
- **WAL CREATE_INDEX is PORTED** in `akar-storage/src/wal.rs` (WALRecord::CreateIndex { table_id })
- **Restructured record files:** NOT ported (Rust uses the monolithic `WALRecord` enum approach instead of separate structs)

---

## 6. Additional Optimizer Passes

### C++ Ladybug has 4 optimizer passes not present in original Kuzu:
| Pass | File | Purpose |
|------|------|---------|
| `CountRelTableOptimizer` | `src/optimizer/count_rel_table_optimizer.cpp` | Detects `COUNT(*)` on isolated `ScanRel` and replaces with CSR metadata lookup |
| `ForeignJoinPushDownOptimizer` | `src/optimizer/foreign_join_push_down_optimizer.cpp` | Pushes hash joins down to foreign table backends (duckdb, postgres, sqlite, neo4j) |
| `OrderByPushDownOptimizer` | `src/optimizer/order_by_push_down_optimizer.cpp` | Pushes ORDER BY below UNION ALL when safe |
| `UnwindDedupOptimizer` | `src/optimizer/unwind_dedup_optimizer.cpp` | Removes duplicate UNWIND operators on the same list expression |

**Rust Port Status:**
- **ALL FOUR PORTED** in `akar-optimizer/src/passes/flat/ladybug.rs` (OrderByPushDown, UnwindDedup, CountRelTable) and `akar-optimizer/src/passes/tree/foreign_join.rs` (ForeignJoinPushDown)
- They are registered in the optimizer pipeline (tested in `optimizer.rs` line 167-171)
- Status: **Production-ready**

---

## 7. Packed Path Extend (enable_packed_path_extend)

### C++ Ladybug (not in original Kuzu):
- `src/include/planner/operator/extend/logical_packed_extend.h` — `LogicalPackedExtend` operator (type `PACKED_EXTEND`)
- `src/planner/operator/extend/logical_packed_extend.cpp`
- `src/planner/plan/append_extend.cpp` — Integrates packed extend into planning
- `src/include/common/data_chunk/data_chunk_state.h` — `PackedChildSlices` struct (parent positions + prefix-sum offsets)
- `src/include/processor/operator/scan/scan_rel_table.h` — `updatePackedChildSlices()` and `reservePackedChildSlicesForBatch()`
- Setting: `SET enable_packed_path_extend = true`

**Concept:** When enabled, the CSR rel scan produces a `PackedChildSlices` descriptor correlated with the output DataChunkState, allowing downstream operators to correlate each batch of children with the parent that produced them. Currently single-parent-per-batch only.

**Rust Port Status:**
- **PARTIALLY PORTED.** `akar-core/akar-processor/src/physical/write_ops/packedextend.rs` has a `PhysicalPackedExtend` operator that produces flattened output rows, but the `PackedChildSlices` metadata descriptor and the full packing logic are **NOT ported**.
- Status: Early stage, only the physical operator skeleton exists.

---

## 8. Concurrent Writer Support (enableMultiWrites / concurrent_writes)

### C++ Ladybug (not in original Kuzu):
- `DBConfig::enableMultiWrites` — Config option defaulting to `false`
- `SET debug_enable_multi_writes = true` — Runtime setting
- Write gate in `TransactionManager`: When `enableMultiWrites` is true, multiple concurrent write transactions are allowed. When false, write transactions are serialized.
- WAL commit sequencing with `walCommitSequence`, `nextWALCommitSequenceToPublish` — Ensures WAL records are committed in order even with concurrent writers.
- `mtxForSerializingPublicFunctionCalls` — Fine-grained locking separates the "writer gate" from checkpoint.

### Rust Port (`akar-core/akar-transaction/src/lib.rs`):
- **PORTED.** `TransactionManager` has `concurrent_writes` config flag (defaults to `true`).
- `begin_write()` uses a condition variable to serialize when `concurrent_writes=false`.
- `allow_concurrent_writes()` / `set_concurrent_writes()` methods.
- Status: **Production-ready** (though default differs -- Rust defaults to `true`, Ladybug C++ defaults to `false`)

---

## 9. ForeignRelTable (Foreign Database Integration)

### C++ Ladybug (not in original Kuzu):
- `src/include/storage/table/foreign_rel_table.h` — `ForeignRelTable` backed by `TableFunction` + `TableFuncBindData`
- `src/storage/table/foreign_rel_table.cpp` — Delegates scan to external table functions
- Created in `storage_manager.cpp` for attached database rel tables

**Rust Port:** **NOT PORTED.** No `ForeignRelTable` equivalent exists.

---

## 10. Extension Differences

### C++ Ladybug extension additions vs original Kuzu:
- **`adbc/`** — Arrow Database Connectivity extension (ADBC driver manager integration). **This does not exist in original Kuzu.**
- All other extensions (algo, azure, delta, duckdb, fts, httpfs, iceberg, json, llm, neo4j, postgres, sqlite, unity_catalog, vector) exist in **both** codebases.

### Rust Port:
- The `adbc.rs` module at `akar-core/akar-main/src/adbc.rs` provides a basic ADBC wrapper (AdbcDatabase, AdbcConnection, AdbcStatement).
- Status: **PARTIALLY PORTED** (core connection infrastructure exists).

---

## 11. StorageFormat Enum

### C++ Ladybug:
- `StorageFormat` enum: `{ NONE, ICEBUG_DISK }`
- Stored in `NodeTableCatalogEntry` and `RelGroupCatalogEntry`
- Checked in DDL binding (`binder/bind/bind_ddl.cpp`) to prevent mixed tables and mutations
- Legacy format upgrade from string storage path

### Original Kuzu:
- No `StorageFormat` enum at all. No icebug-disk support.

### Rust Port:
- **NOT PORTED.** No `StorageFormat` enum equivalent exists in the Rust catalog entries.

---

## 12. README / Documentation Files Unique to Ladybug

| File | Content |
|------|---------|
| `docs/icebug-disk.md` | Icebug-Disk storage format specification |
| `docs/morsel_parallelism.md` | Morsel-driven parallelism architecture |
| `docs/morsel_scan.py` | Python-like pseudocode for scan flows |
| `docs/semi_mask_in_scan.md` | Semi mask optimization in node table scanning |
| `docs/art_index.md` | ART index design and usage |
| `docs/index_build_recovery.md` | Index build recovery invariants and large-index optimization |
| `docs/multi_parent_lifetime.md` | PackedChildSlices lifetime/representation design notes |
| `docs/extensions.md` | Ladybug extension list (same as Kuzu but with Ladybug proxy vars) |
| `docs/incidents/` | Incident post-mortems (4 documents) |
| `AGENTS.md` | Agent build/test instructions (Ladybug-specific) |
| `SECURITY.md` | Security policy (Ladybug-specific) |
| `docs/build_tips.md` | Build config (PAGE_SIZE_LOG2, VECTOR_CAPACITY_LOG2) |
| `docs/testing.md` | Test structure guide |
| `docs/python.md` | Python build instructions with `uv` |
| `docs/cpp_style.md` | C++ style guide |
| `docs/shell.md` | Shell CLI dev guide |

---

## Summary: Rust Port Status

| Feature | C++ Ladybug | Rust Port Status | Notes |
|---------|-------------|-----------------|-------|
| IceDisk Node Table | Full | **Not ported** | Only `ice_format.rs` with basic data structures |
| IceDisk Rel Table | Full | **Partial** | Basic struct + streaming next_row(), no CSR/FLAT scan logic |
| IceDisk Version Validation | Full | **Not ported** | No version metadata checking |
| ColumnarNodeTableBase | Full | **Not ported** | No abstract base class hierarchy |
| ColumnarRelTableBase | Full | **Not ported** | No abstract base class hierarchy |
| Morsel-driven scan | Full | **Not ported** | No morsel dispatching for columnar tables |
| ART Index | Full | **Ported** | Production-ready |
| WAL CREATE_INDEX_RECORD | Full | **Ported** | WALRecord::CreateIndex exists |
| CountRelTableOptimizer | Full | **Ported** | In ladybug.rs |
| ForeignJoinPushDownOptimizer | Full | **Ported** | In foreign_join.rs |
| OrderByPushDownOptimizer | Full | **Ported** | In ladybug.rs |
| UnwindDedupOptimizer | Full | **Ported** | In ladybug.rs |
| Packed Path Extend | Full | **Partial** | Physical operator exists, no PackedChildSlices |
| Concurrent Writers | Full | **Ported** | concurrent_writes config flag |
| ForeignRelTable | Full | **Not ported** | No foreign table backing |
| ADBC Extension | Full | **Partial** | Basic AdbcDatabase/Connection/Statement wrapper |
| StorageFormat enum | Full | **Not ported** | No catalog-level storage format tracking |
| Zone map pushdown | Both exist | Both have | Not a Ladybug-specific feature |
| Semi-mask (RoaringBitmap) | Both exist | Both have | Not a Ladybug-specific feature |