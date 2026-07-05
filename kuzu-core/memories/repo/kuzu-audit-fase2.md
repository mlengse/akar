# Audit Komparasi Kuzu C++ → Rust — Rencana Fase Selanjutnya (v2)

> **Tanggal:** 2026-07-05 | **Update:** P0, P1, P2 selesai. P3.1 (ANALYZE) + P3.6 (PERCENTILE) SELESAI. Fokus: P3.2 (AggregateHashTable) berikutnya.

## PROGRESS FASE P3
- ✅ **P3.1 ANALYZE** — grammar+AST+parser+binder+execution+StatsStore, 6 tests. `ANALYZE *`, `ANALYZE <table>`, `ANALYZE TABLE <table>`
- ✅ **P3.6 PERCENTILE_DISC/CONT** — AggValueState::Percentile, 5 tests. Registry + scalar + physical_operator mapping
- ✅ **P3.2 AggregateHashTable** — rayon parallel aggregation + `AggValueState::merge()`, 4 merge tests. `hashbrown::HashMap` for local tables, `rayon::par_iter()` for chunk parallelism
- ✅ **P3.3 Partitioned JoinHashTable** — parallel build with rayon + `hashbrown::HashMap`, `JoinHashTable` struct
- ✅ **P3.4 External Sort** — LSD radix sort for Int64 keys + `BlockMergeSorter` (block-based parallel + k-way merge). `PhysicalOrderBy` delegates to block merge when >10K rows
- ✅ **P3.5 Batch Insert** — `NodeTable::insert_rows_batch()` + `RelTable::insert_rels_batch()`. Pre-validate, pre-allocate, bulk-append. `PhysicalCopyFrom` wired.

**✅ FASE P3 SELESAI!** 6/6 items done.
**✅ FASE P4 SELESAI!** 3/3 Ladybug passes: OrderByPushDown, UnwindDedup, CountRelTable.
**21 total optimizer passes** (14 flat + 7 tree). **922 test lulus, 0 gagal.**
**✅ FASE P5 SELESAI!** 9 new CALL functions: bm_info, file_info, free_space_info, disk_size_info, storage_version, show_loaded_extensions, show_official_extensions, clear_warnings, show_warnings.

---

## Ringkasan Perubahan Sejak Audit v1

| Prioritas | Status v1 | Status v2 |
|-----------|----------|-----------|
| P0: Crash Recovery | ❌ Gap kritis | ✅ Selesai (UndoBuffer, WALReplayer 6 DDL variants, PageManager) |
| P1: Table Functions | ❌ 26 missing | ✅ 12 CALL functions selesai |
| P2: Transaction | ❌ 50% missing | ✅ Selesai (AUTO/MANUAL, checkpoint worker, QuerySummary) |

**907 test lulus, 0 gagal.**

---

## 1. Kesenjangan Aktual (Post-P0/P1/P2)

### 1.1 Table Functions — Masih Missing (~17 dari 31 C++)

| # | Fungsi | Prioritas |
|---|--------|-----------|
| 1 | `bm_info()` — buffer manager stats | Low |
| 2 | `cache_column()` | Low |
| 3 | `clear_warnings()` | Low |
| 4 | `drop_project_graph()` | Medium |
| 5 | `file_info()` | Low |
| 6 | `free_space_info()` | Low |
| 7 | `projected_graph_info()` | Medium |
| 8 | `project_cypher_graph()` — graph projection | **High** |
| 9 | `project_native_graph()` — graph projection | **High** |
| 10 | `show_loaded_extensions()` | Low |
| 11 | `show_official_extensions()` | Low |
| 12 | `show_projected_graphs()` | Medium |
| 13 | `show_warnings()` | Low |
| 14 | `disk_size_info()` (Ladybug) | Low |
| 15 | `show_graphs()` (Ladybug) | Medium |
| 16 | `storage_version()` (Ladybug) | Low |
| 17 | Variable-length path functions | Medium |

### 1.2 Physical Operators — Simplified vs Full C++

Rust memiliki **28 physical operator dispatches** (termasuk DDL stubs). Namun sebagian besar adalah implementasi **sederhana**:

| Operator | Status Rust | Gap vs C++ |
|----------|------------|------------|
| **HashJoin** | ✅ Simplified (single-level hash) | ❌ Partitioned JoinHashTable, parallel build/probe |
| **Aggregate** | ✅ Value-based HashMap | ❌ AggregateHashTable with parallel hash aggregation |
| **OrderBy** | ✅ In-memory sort | ❌ External sort (RadixSort, KeyBlockMerger, OrderByMerge) |
| **CopyFrom** | ✅ Basic CSV/Parquet | ❌ NodeBatchInsert + RelBatchInsert pipeline |
| **Merge** | ✅ Basic match-insert | ❌ Lock-based concurrent MERGE |
| **DDL operators** | ⚠️ Stub (empty result) | ❌ Actual catalog mutation at execution time |

### 1.3 Ladybug Features — Belum Diporting

| Fitur | C++ Ladybug | Rust | Prioritas |
|-------|------------|------|-----------|
| **OrderByPushDown** optimizer | ✅ | ❌ | Medium |
| **UnwindDedup** optimizer + physical | ✅ | ❌ | Medium |
| **CountRelTable** optimizer + physical | ✅ | ❌ | Medium |
| **ANALYZE statement** | ✅ | ❌ | High |
| **PERCENTILE_DISC/CONT** aggregates | ✅ | ❌ | Medium |
| **GRAPH statement** | ✅ | ❌ | Low |
| **CREATE INDEX** physical | ✅ | ⚠️ Stub | Medium |

### 1.4 Quality & Infrastruktur

| Item | Status |
|------|--------|
| CI/CD (GitHub Actions) | ❌ |
| Wasm build | ⚠️ Partial |
| kuzu-httpfs extension | ⚠️ Stub |
| kuzu-fts extension | ⚠️ Stub |
| PlanPrinter (pretty EXPLAIN) | ❌ |
| Storage column specializations (string/list/struct/null/dictionary) | ❌ |
| External sort (RadixSort) | ❌ |

---

## 2. Rencana Implementasi — Fase P3 / P4 / P5

### 🔴 P3: Physical Operator Completeness (Prioritas Tinggi)

| # | Item | Deskripsi | Estimasi |
|---|------|-----------|----------|
| **P3.1** | ANALYZE statement | Parser → Binder → Planner → Processor. Kumpulkan statistik tabel (row count, distinct values, null ratio) via StatsStore | 4-5 jam |
| **P3.2** | AggregateHashTable | Hash table untuk parallel aggregation dengan group-by keys. Ganti HashMap<Vec<Value>> yang sekarang dengan struktur hash table dedicated | 5-7 jam |
| **P3.3** | Partitioned JoinHashTable | Hash table untuk hash join dengan partition-level parallelism. Ganti single-level hash join sekarang | 5-7 jam |
| **P3.4** | External Sort (RadixSort) | Sort berbasis disk untuk dataset besar. Radix sort + KeyBlockMerger + OrderByMerge pipeline | 5-7 jam |
| **P3.5** | NodeBatchInsert + RelBatchInsert | Bulk insert pipeline untuk COPY FROM | 5-7 jam |
| **P3.6** | PERCENTILE_DISC/CONT aggregates | Aggregate functions untuk percentile | 2-3 jam |

**Estimasi P3: 26-36 jam**

### 🟡 P4: Ladybug-Specific Optimizer Passes

| # | Item | Deskripsi | Estimasi |
|---|------|-----------|----------|
| **P4.1** | OrderByPushDown optimizer | Push ORDER BY ke bawah melewati UNION, JOIN | 2-3 jam |
| **P4.2** | UnwindDedup optimizer + physical | Deduplikasi UNWIND list elements | 2-3 jam |
| **P4.3** | CountRelTable optimizer + physical | Optimize COUNT(*) pada rel tables via CSR metadata | 2-3 jam |
| **P4.4** | GRAPH statement (opsional) | Parser + Binder untuk graph projection management | 3-4 jam |
| **P4.5** | Project graph CALL functions | `project_cypher_graph()`, `project_native_graph()` | 3-4 jam |

**Estimasi P4: 12-17 jam**

### 🟢 P5: Remaining Table Functions

| # | Item | Deskripsi | Estimasi |
|---|------|-----------|----------|
| **P5.1** | `bm_info()` — buffer manager stats | Query BufferManager untuk memory usage | 1 jam |
| **P5.2** | `file_info()` — file page statistics | Query FileHandle/PageManager | 1 jam |
| **P5.3** | `free_space_info()` — FSM stats | Query FreeSpaceManager | 1 jam |
| **P5.4** | `show_loaded_extensions()` | List extensions dari ExtensionRegistry | 1 jam |
| **P5.5** | `show_official_extensions()` | Static list official extensions | 0.5 jam |
| **P5.6** | `show_projected_graphs()` / `projected_graph_info()` | Graph catalog query | 1 jam |
| **P5.7** | `clear_warnings()` / `show_warnings()` | WarningContext management | 1 jam |
| **P5.8** | `disk_size_info()` / `storage_version()` | System info functions | 1 jam |

**Estimasi P5: 7-8 jam**

### ⚪ P6: Quality & Infrastructure

| # | Item | Deskripsi | Estimasi |
|---|------|-----------|----------|
| **P6.1** | CI/CD setup | GitHub Actions: build + test + clippy + fmt check | 3-4 jam |
| **P6.2** | Storage column specializations | StringColumn (dictionary encoding), ListColumn, StructColumn, NullColumn | 6-8 jam |
| **P6.3** | PlanPrinter | Pretty EXPLAIN output dengan indentation + operator details | 2-3 jam |
| **P6.4** | kuzu-httpfs completion | Selesaikan stub HTTPFS dengan full HTTP/S3 support | 3-4 jam |
| **P6.5** | kuzu-fts completion | Selesaikan stub FTS dengan full-text indexing engine | 4-6 jam |
| **P6.6** | Wasm build stabilization | Fix wasm32 target issues, CI untuk wasm | 2-3 jam |

**Estimasi P6: 20-28 jam**

---

## 3. Estimasi Total

| Fase | Item | Jam |
|------|------|-----|
| 🔴 P3: Physical Ops | 6 items | 26-36 |
| 🟡 P4: Ladybug Features | 5 items | 12-17 |
| 🟢 P5: Table Functions | 8 items | 7-8 |
| ⚪ P6: Quality | 6 items | 20-28 |
| **Total** | **25 items** | **65-89 jam** |

---

## 4. Rekomendasi Urutan Eksekusi

1. **Minggu 1:** P3.1 (ANALYZE) + P3.6 (PERCENTILE) — low hanging fruit
2. **Minggu 2:** P3.2 (AggregateHashTable) — impact besar untuk aggregation queries
3. **Minggu 3:** P3.3 (Partitioned JoinHashTable) — impact besar untuk join queries
4. **Minggu 4:** P3.4 (External Sort) + P3.5 (Batch Insert)
5. **Minggu 5:** P4 (Ladybug passes) + P5 (table functions)
6. **Minggu 6:** P6.1 (CI/CD) + P6.3 (PlanPrinter)
7. **Minggu 7:** P6.2 (column specializations) + P6.4/P6.5 (extensions)
8. **Minggu 8:** P6.6 (Wasm) + final polish

---

## 5. Catatan Teknis

### AggregateHashTable Design
- Saat ini: `PhysicalAggregate` menggunakan `HashMap<Vec<Value>, AggValueState>` — single-threaded
- Target: `AggregateHashTable` dengan partition-level parallelism, thread-local aggregation, lalu merge
- Referensi C++: `src/processor/operator/aggregate/aggregate_hash_table.cpp`

### Partitioned JoinHashTable Design
- Saat ini: single-level hash join dengan `HashMap<u64, Vec<(Value, Vec<(usize, usize)>)>>`
- Target: partition-based hash join dengan multiple partitions, thread-local build, parallel probe
- Referensi C++: `src/processor/operator/hash_join/join_hash_table.cpp`

### ANALYZE Statement Design
- Pipeline: Parser (grammar `analyze_statement`) → AST (`Statement::Analyze`) → Binder (`BoundAnalyze`) → Processor (scan table, compute stats, store ke StatsStore)
- Stats: row count, distinct count per column (HyperLogLog), null ratio, min/max, avg size
- Referensi Ladybug: `ladybug/src/binder/bind/bind_analyze.cpp`


---

## Ringkasan Eksekutif

| Metrik | C++ (Kuzu) | C++ (Ladybug) | Rust (kuzu-core) | Coverage |
|--------|-----------|---------------|-------------------|----------|
| **Total file .cpp** | ~250+ | ~280+ | ~94 .rs | — |
| **Parser** | ANTLR4 | ANTLR4 | pest.rs PEG | ~95% |
| **Binder** | 20+ BoundStatement | 22+ (incl. Analyze, Graph) | 18 BoundStatement | ~90% |
| **Planner** | 36+ LogicalOperator | 40+ | 34 variants | ~90% |
| **Optimizer** | 16 passes | 20 passes | 18 passes | ~90% |
| **Processor** | 40+ physical ops | 45+ | 22 physical ops | ~55% |
| **Storage** | 50+ modules | 60+ modules | 26 modules | ~50% |
| **Functions** | 180+ | 185+ | 150+ | ~85% |
| **GDS** | 18 specializations | 18 specializations | 8 algos + framework | ~70% |
| **Tests** | — | — | 898 (0 failed) | ✅ |

**Kesimpulan:** Framework-level coverage bagus (parser/binder/planner/optimizer ~90%). Gap terbesar ada di **physical operators** (~45% missing) dan **storage detailed implementations** (~50% missing).

---

## 1. Perbandingan per Layer

### 1.1 Function / Registry

#### Scalar Functions
| Kategori | C++ | Rust | Gap |
|----------|-----|------|-----|
| Arithmetic | 26 | 26 | ✅ 100% |
| Comparison | 8 | 8 | ✅ 100% |
| Boolean | 4 | 4 | ✅ 100% |
| String | 23 | 23 | ✅ 100% |
| Date/Time | 18 | 16 | ⚠️ 2 missing (CENTURY, EPOCH_MS) |
| Cast | 14+ | 14+ | ✅ 100% |
| List | 24 | 24 | ✅ 100% |
| Map | 5 | 4 | ⚠️ CARDINALITY missing |
| Struct | 2 | 2 | ✅ 100% |
| Schema | 5 | 5 | ✅ 100% |
| Array | 5 | 5 | ✅ 100% |
| Path | 6 | 6 | ✅ 100% |
| UUID | 1 | 1 | ✅ 100% |
| Bitwise | 5 | 5 | ✅ 100% |
| Hash | 3 | 3 | ✅ 100% |
| Blob | 3 | 3 | ✅ 100% |
| Union | 3 | 3 | ✅ 100% |
| Interval | 8 | 8 | ✅ 100% |
| Sequence | 2 | 2 | ✅ 100% |

**Scalar coverage: ~98%** — hanya CENTURY, EPOCH_MS, CARDINALITY yang perlu diverifikasi.

#### Aggregate Functions
| C++ | Rust | Status |
|-----|------|--------|
| COUNT, COUNT(*) | ✅ | |
| SUM | ✅ | |
| AVG | ✅ | |
| MIN, MAX | ✅ | |
| COLLECT | ✅ | |
| STDDEV, VARIANCE | ✅ | |
| **PERCENTILE_DISC** (Ladybug) | ❌ | Missing |
| **PERCENTILE_CONT** | ❌ | Missing |

#### Table Functions — GAP BESAR (31 C++ vs 8 Rust)

| # | Fungsi C++ | Rust | Prioritas |
|---|-----------|------|-----------|
| 1 | `bm_info()` | ❌ | Low |
| 2 | `cache_column()` | ❌ | Low |
| 3 | `catalog_version()` | ❌ | Medium |
| 4 | `clear_warnings()` | ❌ | Low |
| 5 | `current_setting()` | ❌ | Medium |
| 6 | `db_version()` | ❌ | Medium |
| 7 | `drop_project_graph()` | ❌ | Medium |
| 8 | `file_info()` | ❌ | Low |
| 9 | `free_space_info()` | ❌ | Low |
| 10 | `projected_graph_info()` | ❌ | Medium |
| 11 | `project_cypher_graph()` | ❌ | High |
| 12 | `project_native_graph()` | ❌ | High |
| 13 | `show_attached_databases()` | ❌ | Medium |
| 14 | `show_connection()` | ❌ | Medium |
| 15 | `show_functions()` | ❌ | Medium |
| 16 | `show_indexes()` | ❌ | Medium |
| 17 | `show_loaded_extensions()` | ❌ | Low |
| 18 | `show_macros()` | ❌ | Medium |
| 19 | `show_official_extensions()` | ❌ | Low |
| 20 | `show_projected_graphs()` | ❌ | Medium |
| 21 | `show_sequences()` | ❌ | Medium |
| 22 | `show_tables()` | ⚠️ Partial | Medium |
| 23 | `show_warnings()` | ❌ | Low |
| 24 | `stats_info()` | ❌ | Medium |
| 25 | `storage_info()` | ❌ | Low |
| 26 | `table_info()` | ⚠️ Partial (ShowColumns) | Medium |
| 27 | `disk_size_info()` | ❌ | Low |
| 28 | `show_graphs()` | ❌ | Medium |
| 29 | `storage_version()` | ❌ | Low |

**Rust has:** `ListTables`, `ShowColumns`, `ScanCsv`, `ScanParquet`, `ScanJson`, `CurrentSetting`, `Custom`, `CustomTable`

### 1.2 Optimizer

| Pass | C++ | Ladybug | Rust | Notes |
|------|-----|---------|------|-------|
| RemoveUnnecessaryOperators | ✅ | ✅ | ✅ | |
| FilterPushDown | ✅ | ✅ | ✅ | |
| ProjectionPushDown | ✅ | ✅ | ✅ | |
| ConstantFolding | ✅ | ✅ | ✅ | |
| AggregateDetection | ✅ | ✅ | ✅ | |
| JoinOptimization | ✅ | ✅ | ✅ | DP Bushy Trees |
| TopKOptimization | ✅ | ✅ | ✅ | |
| VectorSimilarityDetection | ✅ | ✅ | ✅ | |
| ArtRangeScanDetection | ✅ | ✅ | ✅ | |
| LimitPushDown | ✅ | ✅ | ✅ | |
| CommonSubexpressionElimination | ✅ | ✅ | ✅ | |
| FactorizationRewriting | ✅ | ✅ | ✅ | |
| ForeignJoinPushDown | ✅ | ✅ | ✅ | Ladybug has this too |
| AccHashJoinOptimization | ✅ | ✅ | ✅ | |
| SIPOptimization | ✅ | ✅ | ✅ | |
| CorrelatedSubqueryUnnesting | ✅ | ✅ | ✅ | |
| AggKeyDependency | ✅ | ✅ | ✅ | |
| CardinalityEstimation | ✅ | ✅ | ✅ | with StatsStore |
| **CountRelTable** | ❌ | ✅ | ❌ | Ladybug only |
| **OrderByPushDown** | ❌ | ✅ | ❌ | Ladybug only |
| **UnwindDedup** | ❌ | ✅ | ❌ | Ladybug only |
| **TopK materialized** | ✅ | ✅ | ❌ | Multi-column TopK |

### 1.3 Storage — GAP RINCI

#### Yang SUDAH ada di Rust (✅):
`art_index`, `art_key`, `art_node`, `buffer_manager`, `checkpoint`, `column`, `column_chunk`, `compression`, `csr`, `csv_reader`, `free_space_manager`, `index`, `local_storage`, `local_wal`, `node_group`, `page`, `parquet_reader`, `predicate`, `shadow_file`, `spiller`, `stats`, `table`, `update_info`, `vector_index`, `version_info`, `wal`

#### Yang BELUM ada (❌) — Prioritas Tinggi:
| Modul C++ | Deskripsi | Dampak |
|-----------|-----------|--------|
| `undo_buffer.cpp` | Undo buffer untuk rollback transaksi | Correctness: rollback tidak aman tanpa undo |
| `wal_replayer.cpp` + `wal_record.cpp` | WAL replay untuk crash recovery | Correctness: database corrupt setelah crash |
| `page_manager.cpp` | Alokasi/hapus halaman via FSM | Feature: tanpa ini, file tumbuh tak terbatas |
| `disk_array.cpp` | Array berbasis disk dengan halaman | Foundation: banyak storage structures bergantung ini |
| `file_handle.cpp` | Abstraksi file handle untuk page I/O | Foundation: semua I/O lewat sini |
| `storage_manager.cpp` | Orchestrator utama storage | Architecture: wiring semua komponen |

#### Yang BELUM ada (❌) — Prioritas Medium:
| Modul C++ | Deskripsi |
|-----------|-----------|
| `table/string_column.cpp` | Kolom string dengan dictionary |
| `table/list_column.cpp` | Kolom list (array) |
| `table/struct_column.cpp` | Kolom struct |
| `table/null_column.cpp` | Null column bitmap |
| `table/node_table.cpp` | Node table storage lengkap |
| `table/rel_table.cpp` | Rel table storage lengkap |
| `table/chunked_node_group.cpp` | Chunked node group format |
| `table/dictionary_chunk.cpp` | Dictionary encoding untuk string |
| `compression/bitpacking*.cpp` | Bitpacking compression |
| `compression/float_compression.cpp` | Float compression |
| `stats/hyperloglog.cpp` | HyperLogLog cardinality estimation |

### 1.4 Processor / Physical Operators

#### Yang SUDAH ada (✅ 22 ops):
PhysicalScan, PhysicalScanRel, PhysicalVectorSimilarityScan, PhysicalArtIndexRangeScan, PhysicalFilter, PhysicalProjection, PhysicalHashJoin, PhysicalCrossProduct, PhysicalOrderBy, PhysicalLimit, PhysicalAggregate, PhysicalUnion, PhysicalFlatten, PhysicalIntersect, PhysicalSemiJoin, PhysicalAntiJoin, PhysicalSemiMasker, PhysicalRecursiveExtend, PhysicalExplain, PhysicalForeach, PhysicalCopyFrom, PhysicalCreateFtsIndex

#### Yang BELUM (❌) — Prioritas Tinggi:
| Operator C++ | Deskripsi |
|-------------|-----------|
| **DDL operators** (create_table, alter, drop, create_sequence, create_type) | DDL execution |
| **HashJoinBuild + HashJoinProbe + JoinHashTable** | Full hash join pipeline (saat ini simplified) |
| **AggregateHashTable + HashAggregate + SimpleAggregate** | Full aggregate pipeline |
| **OrderByMerge + RadixSort + TopKScanner** | Full sort pipeline |
| **NodeBatchInsert + RelBatchInsert** | COPY pipeline lengkap |
| **Merge** | MERGE execution |
| **IndexLookup** | PK index probe |

#### Yang BELUM (❌) — Prioritas Medium:
| Operator C++ | Deskripsi |
|-------------|-----------|
| ArrowResultCollector | Arrow output collector |
| MultiplicityReducer | Dedup after joins |
| Partitioner | COPY data partitioning |
| PathPropertyProbe | Property extraction from paths |
| ResultCollector | Accumulate results |
| Skip | OFFSET clause |
| StandaloneCall | SET/CALL execution |
| TableFunctionCall | CALL invocation |
| TableScan + UnionAllScan | Table function scan |

#### Ladybug-specific yang BELUM (❌):
| Operator | Deskripsi |
|----------|-----------|
| CountRelTable | COUNT on rel tables |
| RelDegreeTable | Rel degree scan |
| UnwindDedup | UNWIND deduplication |
| CreateIndex | Physical CREATE INDEX |

### 1.5 Transaction Layer

| Komponen | C++ | Rust | Gap |
|----------|-----|------|-----|
| Transaction (MVCC) | ✅ | ⚠️ Partial | Missing undo records, local cache |
| TransactionContext (AUTO/MANUAL) | ✅ | ❌ | Entire module missing |
| TransactionManager (commit/rollback/checkpoint) | ✅ | ⚠️ Partial | No checkpoint worker thread |
| Concurrent write detection | ✅ Serializable | ⚠️ Basic MVCC | Conflict detection missing |

### 1.6 Main / Connection

| Komponen | C++ | Rust | Gap |
|----------|-----|------|-----|
| Database | ✅ | ✅ | |
| Connection | ✅ | ✅ | |
| PreparedStatement | ✅ | ✅ | |
| QueryResult | ✅ | ✅ | |
| ClientContext | ✅ | ❌ | Warnings, progress, pipeline state |
| DatabaseManager | ✅ | ❌ | Attach/detach external DBs |
| PlanPrinter | ✅ | ❌ | Pretty EXPLAIN output |
| PreparedStatementManager | ✅ | ❌ | LRU cache |
| QuerySummary | ✅ | ❌ | Timing stats |
| Settings | ✅ | ❌ | SET parameter implementations |
| StorageDriver | ✅ | ❌ | Direct table access for testing |

---

## 2. Rencana Implementasi — Fase Selanjutnya

### 🔴 Prioritas 0: Correctness & Crash Recovery (critical path)

| # | Item | Dependensi | Estimasi |
|---|------|-----------|----------|
| 0.1 | **Undo Buffer** — `undo_buffer.rs` | — | 3-4 jam |
| 0.2 | **WAL Replayer** — replay records dopo crash | WAL records | 4-6 jam |
| 0.3 | **Page Manager** — alokasi/hapus halaman | FSM | 3-4 jam |
| 0.4 | **FileHandle** — page I/O abstraction | Page Manager | 3-4 jam |
| 0.5 | **StorageManager** — orchestrator | FileHandle, WAL, BufferManager | 4-6 jam |

**Estimasi total Prioritas 0: 17-24 jam**

### 🟠 Prioritas 1: Table Functions (inspection & management)

| # | Item | Dependensi | Estimasi |
|---|------|-----------|----------|
| 1.1 | `show_tables()` — full implementation | Catalog | 1-2 jam |
| 1.2 | `table_info()` — columns, types, PK, nullability | Catalog | 1-2 jam |
| 1.3 | `show_functions()` — list all registered | FunctionRegistry | 1 jam |
| 1.4 | `show_sequences()` | Catalog (sequences) | 1 jam |
| 1.5 | `show_indexes()` | Catalog (indexes) | 1 jam |
| 1.6 | `show_macros()` | Catalog (macros) | 1 jam |
| 1.7 | `show_connection()` — node/rel pattern | Catalog | 1-2 jam |
| 1.8 | `show_attached_databases()` | DatabaseManager (0.6) | 1 jam |
| 1.9 | `current_setting()` | Settings (0.7) | 1 jam |
| 1.10 | `db_version()` | Version constant | 0.5 jam |
| 1.11 | `catalog_version()` | Catalog version tracking | 1 jam |
| 1.12 | `stats_info()` + `storage_info()` | StatsStore | 2-3 jam |

**Estimasi total Prioritas 1: 12-16 jam**

### 🟡 Prioritas 2: Transaction Layer Enhancement

| # | Item | Dependensi | Estimasi |
|---|------|-----------|----------|
| 2.1 | **TransactionContext** — AUTO/MANUAL modes | Transaction | 3-4 jam |
| 2.2 | **Checkpoint worker thread** | TransactionManager | 3-4 jam |
| 2.3 | Conflict detection for concurrent writes | TransactionManager | 3-4 jam |
| 2.4 | **PreparedStatementManager** — LRU cache | — | 2-3 jam |
| 2.5 | **QuerySummary** — timing stats | — | 2-3 jam |

**Estimasi total Prioritas 2: 13-18 jam**

### 🟢 Prioritas 3: Physical Operator Completeness

| # | Item | Dependensi | Estimasi |
|---|------|-----------|----------|
| 3.1 | **AggregateHashTable** — full hash-based aggregation | — | 6-8 jam |
| 3.2 | **JoinHashTable** — proper hash join infra | — | 6-8 jam |
| 3.3 | **RadixSort + TopKScanner** — full sort pipeline | — | 4-6 jam |
| 3.4 | **Merge** — MERGE physical execution | — | 3-4 jam |
| 3.5 | **DDL operators** (create_table, alter, drop) | — | 4-6 jam |
| 3.6 | **NodeBatchInsert + RelBatchInsert** — COPY pipeline | — | 5-7 jam |

**Estimasi total Prioritas 3: 28-39 jam**

### 🔵 Prioritas 4: Ladybug-Specific Features

| # | Item | Dependensi | Estimasi |
|---|------|-----------|----------|
| 4.1 | **OrderByPushDown** optimizer pass | — | 2-3 jam |
| 4.2 | **UnwindDedup** optimizer + physical | — | 2-3 jam |
| 4.3 | **CountRelTable** optimizer + physical | — | 2-3 jam |
| 4.4 | **PERCENTILE_DISC/CONT** aggregates | — | 2-3 jam |
| 4.5 | **ANALYZE statement** (parser→binder→processor) | — | 3-4 jam |

**Estimasi total Prioritas 4: 11-16 jam**

### ⚪ Prioritas 5: Quality & Polish

| # | Item | Dependensi | Estimasi |
|---|------|-----------|----------|
| 5.1 | CI/CD setup (GitHub Actions) | — | 3-4 jam |
| 5.2 | PlanPrinter — pretty EXPLAIN output | — | 2-3 jam |
| 5.3 | ClientContext — warnings, progress | — | 2-3 jam |
| 5.4 | String/List/Struct column specializations | — | 6-8 jam |
| 5.5 | Dictionary encoding for strings | — | 3-4 jam |
| 5.6 | HyperLogLog cardinality estimation | — | 2-3 jam |

**Estimasi total Prioritas 5: 18-25 jam**

---

## 3. Estimasi Total

| Prioritas | Item | Jam |
|-----------|------|-----|
| 🔴 P0: Crash Recovery | 5 items | 17-24 |
| 🟠 P1: Table Functions | 12 items | 12-16 |
| 🟡 P2: Transaction | 5 items | 13-18 |
| 🟢 P3: Physical Ops | 6 items | 28-39 |
| 🔵 P4: Ladybug Features | 5 items | 11-16 |
| ⚪ P5: Quality | 6 items | 18-25 |
| **Total** | **39 items** | **99-138 jam** |

---

## 4. Rekomendasi Urutan Eksekusi

1. **Minggu 1:** P0 (Crash Recovery) — ini critical path, tanpa ini database tidak durable
2. **Minggu 2:** P1 (Table Functions) — inspeksi catalog, mudah diimplementasi, high visibility
3. **Minggu 3:** P2 (Transaction) — AUTO/MANUAL modes + checkpoint worker
4. **Minggu 4-5:** P3 (Physical Ops) — aggregate hash table, join hash table, sort pipeline
5. **Minggu 6:** P4 (Ladybug) — optimizer passes + ANALYZE + PERCENTILE
6. **Minggu 7:** P5 (Quality) — CI, plan printer, column specializations

---

## 5. Catatan Teknis

### Ladybug vs Kuzu C++ vs Rust
- **Ladybug** adalah fork yang menambahkan: ART index, Arrow-native tables, WAL record system (28 record types), 4 optimizer passes, ANALYZE, GRAPH, PERCENTILE_DISC, ADBC
- **Rust** sudah mem-port ART index (`art_index.rs`), ADBC (`adbc.rs`), dan ForeignJoinPushDown
- **Rust belum** mem-port: OrderByPushDown, UnwindDedup, CountRelTable, ANALYZE, GRAPH, PERCENTILE_DISC
- **Arrow tables** di Ladybug: Rust memiliki `arrow` crate sebagai dependency, dan `parquet_reader`, tapi belum ada Arrow-native table storage

### WAL di Rust vs Ladybug
- Rust WAL: 8 record types (Insert, Delete, Update, UpdateFsm, ColumnWrite, LocalWALData, Commit, Rollback, Checkpoint) — simplified
- Ladybug WAL: 28 record types dengan type-spec code generation dan replay logic per record
- Gap: Rust tidak punya WAL replayer dan record types untuk DDL (alter, create catalog entry, drop, load extension, dll)

### Physical Operators: Kenapa gampang vs susah
- Simple operators (Filter, Projection, Limit, Flatten, Unwind) sudah selesai
- Binary operators (HashJoin, CrossProduct, Intersect, SemiJoin, AntiJoin) sudah selesai dengan `execute_binary` pattern
- Yang susah: HashJoinBuild/Probe dengan JoinHashTable penuh, AggregateHashTable, Sort (RadixSort), dan DDL operators yang butuh akses ke Catalog + StorageManager + WAL
