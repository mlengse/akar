# Audit Codebase: Kuzu C++ → Kuzu Rust (Porting 1:1) 19/07/2026

## Ringkasan Arsitektur

```
C++ Kuzu (v0.12.0)                     Rust kuzu-core (v0.1.0)
├── src/                                ├── kuzu-core/
│   ├── antlr4/ (ANTLR grammar)         │   ├── kuzu-parser/ (pest.rs PEG)
│   ├── main/                           │   ├── kuzu-main/
│   ├── common/                         │   ├── kuzu-common/
│   ├── parser/                         │   ├── kuzu-parser/
│   ├── binder/                         │   ├── kuzu-binder/
│   ├── planner/                        │   ├── kuzu-planner/
│   ├── optimizer/                      │   ├── kuzu-optimizer/
│   ├── processor/                      │   ├── kuzu-processor/
│   ├── storage/                        │   ├── kuzu-storage/
│   ├── catalog/                        │   ├── kuzu-catalog/
│   ├── transaction/                    │   ├── kuzu-transaction/
│   ├── function/                       │   ├── kuzu-function/
│   ├── graph/                          │   ├── kuzu-graph/
│   ├── expression_evaluator/           │   │   (ada di processor)
│   └── c_api/                          │   ├── kuzu-c/
├── extension/ (14 modules)             │   ├── kuzu-{json,fts,vector,...}*
└── tools/                              └── kuzu-cli/, kuzu-wasm/, kuzu-migrate/
```

**Ladybug C++** (`ladybug/`) = fork Kuzu v0.18.0 dengan tambahan fitur (IceDisk, ART Index, morsel scan, concurrent writers, 4 optimizer passes, PackedExtend).

---

## Status Per Modul (Pipeline Eksekusi Query)

| Layer | C++ Kuzu | Rust Port | Kesenjangan Kritis |
|-------|----------|-----------|-------------------|
| **Parser** | ANTLR4 Cypher (917 line grammar) | pest.rs PEG (477 line) | **ORDER BY, LIMIT, SKIP** di-parse grammar tapi **tidak disimpan ke AST** → silent discard. Named paths `p=(a)-[*]->(b)`, regex `=~`, list slicing, bitwise ops, `COUNT {MATCH}`, `CALL...YIELD`, `IF NOT EXISTS`, multi-statement tidak ada. |
| **Binder** | Resolusi tipe via catalog lookup | Resolusi tipe **hardcoded heuristic** (`name`→String, `age`→Int64) | Tidak bisa resolve tipe property sebenarnya dari catalog. Kode duplikasi antara `mod.rs` dan `dml.rs`. |
| **Planner** | Full logical plan (OrderBy, Limit, Skip, Aggregate GROUP BY, SemiJoin, AntiJoin, Intersect, dll.) | **15+ tipe statement produce empty plan** (Transaction, Extension, Attach/Detach/Use DB, LoadFrom, CreateType, CommentOn, Graph, Analyze, CopyTo, CreateMacro). **ORDER BY/LIMIT/Skip tidak pernah di-plan**. |
| **Optimizer** | 16 passes + 4 Ladybug | 14 flat + 7 tree passes | TopK, LimitPushDown adalah dead code (karena planner tidak produce OrderBy/Limit). GROUP BY extraction tidak ada di AggregateDetection. |
| **Processor** | 60 physical operator types | 45 implemented (75%), **12 DDL no-op stubs**, 3 partial | **Semua DDL adalah no-op** — `map_ddl.rs` return empty chunk tanpa side effect. TableFunctionCall hanya support vector_similarity_scan. |
| **Expression Evaluator** | `FunctionEvaluator`, `CaseEvaluator`, `LambdaEvaluator`, `PatternEvaluator`, `PathEvaluator` | Hanya `FunctionEvaluator` + evaluator inline | Case/when, lambda, path evaluator belum di-port. |

**Kesimpulan Pipeline**: Parser → Binder → Planner → Optimizer → Processor, **ORDER BY, LIMIT, SKIP, DDL (CREATE/DROP/ALTER TABLE), dan 15+ statement tidak bisa dijalankan**. Pipeline hanya berfungsi untuk `MATCH ... RETURN` + filter sederhana.

---

## Status Storage Engine

| Komponen | C++ Kuzu/Ladybug | Rust Port | Kesenjangan |
|----------|-----------------|-----------|-------------|
| **BufferManager** | MMAP regions, NUMA, page readahead, segment-based alloc | Clock eviction, Vec-backed, no NUMA | Tidak ada memory-mapped regions, direct I/O, NUMA placement |
| **Column** | Arrow-format batch scan `scan(vector, offset, length)` | `scan_all_values()` → `Vec<Value>` per-value tag-byte | **Tidak ada batch scan**. Null bitmap via tag byte bukan mask. |
| **NodeTable** | Persistent Column objects, NodeGroupCollection, PrimaryKeyIndex | In-memory dengan `flush_node_groups()` ke Column | OK untuk operasi dasar |
| **RelTable** | **CSR adjacency** per direction (offset + length + nbrID columns) | `Vec<RelData>` flat — **CSR adalah stub** | **`csr.rs::get_neighbors()` return `Ok(vec![])`** — graph traversal tidak bekerja. |
| **HashIndex** | On-disk linear hashing, overflow slots, fingerprint byte | L1 HashMap + L2 OnDiskHashIndex | OK |
| **ART Index** | Node4/16/48/256, persistent, range scan, WAL CREATE_INDEX | **Ported** (art_index.rs, art_key.rs, art_node.rs) | ✅ Production-ready |
| **WAL** | LocalWAL + shared WAL, 16 record types, CRC32 checksums | **Ported** dengan semua record types | ✅ Termasuk CREATE_INDEX_RECORD |
| **Checkpoint** | Shadow file COW, WAL truncation, atomic DB header update | `flush_table()` **no-op**, no WAL truncation | Checkpoint tidak benar-benar mem-persist data |
| **WALReplayer** | Integrated dengan StorageManager | **Callback-based** — delegate ke closure | Tidak otomatis apply ke storage |
| **Compression** | ALP, IntegerBitpacking (fastpfor), BooleanBitpacking, Constant, Uncompressed, StringDictionary | Per-type function dispatch, StringDictionary **pass-through** | Tidak ada SIMD, dictionary encoding tidak jalan |
| **LocalStorage** | Arrow columnar format | **Serialized byte blobs** (`Vec<u8>`) | Tidak vectorized, tidak ada conflict detection |
| **ShadowFile** | COW di level BMFileHandle | **HashMap in-memory** — tidak persist ke disk | Shadow page tidak disimpan |
| **Spiller** | Arrow IPC zero-copy | **JSON lines** — jauh lebih lambat | Spill format tidak efisien |
| **RoaringBitmap** | Array + Bitmap + **Run containers**, SIMD, serde | Array + Bitmap saja, **no serde** | Tidak ada Run container, persistensi |
| **FreeSpaceManager** | Buddy system | BTreeSet PageRange | ✅ OK |
| **IceFormat** | IceDiskNodeTable + IceDiskRelTable (Parquet-based read-only) | **Basic struct + next_row() only** | Tidak ada scan logic, CSR/FLAT layout, version validation |
| **VectorIndex** | HNSW dengan persistensi | **Ported** (vector_index.rs) | ✅ |

---

## Gap Ladybug-Specific

| Fitur Ladybug | C++ Status | Rust Status |
|--------------|-----------|-------------|
| **ART Index** | Full (Node4/16/48/256, persistent, range scan) | **Ported** ✅ |
| **CountRelTable** optimizer pass | Full | **Ported** ✅ |
| **ForeignJoinPushDown** | Full | **Ported** ✅ |
| **OrderByPushDown** (under UNION ALL) | Full | **Ported** ✅ |
| **UnwindDedup** | Full | **Ported** ✅ |
| **Concurrent Writers** (enableMultiWrites) | Full | **Ported** (default `true`) ✅ |
| **WAL CREATE_INDEX_RECORD** | Full | **Ported** ✅ |
| **IceDisk** (Parquet read-only storage) | Full | **Partial** — basic struct saja |
| **ColumnarNodeTableBase / ColumnarRelTableBase** (abstract base classes) | Full | **Not ported** |
| **Morsel-driven scan** (parallel row group dispatching) | Full | **Not ported** |
| **PackedExtend** (enable_packed_path_extend) | Full | **Partial** — physical operator skeleton |
| **ForeignRelTable** (foreign DB scan delegation) | Full | **Not ported** |
| **StorageFormat enum** (NONE / ICEBUG_DISK) | Full | **Not ported** |
| **ADBC extension** | Full | **Partial** — basic wrapper |
| **PackedChildSlices** metadata | Full | **Not ported** |

---

## Ringkasan Ekstensi (Semua Placeholder)

| Ekstensi | C++ | Rust | Status |
|----------|-----|------|--------|
| json | Full | `kuzu-json` | Placeholder |
| fts | Full | `kuzu-fts` | ✅ Implemented (BM25) |
| vector (HNSW) | Full | `kuzu-vector` | ✅ Implemented |
| httpfs | Full | `kuzu-httpfs` | Placeholder |
| duckdb | Full | `kuzu-duckdb` | "2 functions (placeholder)" |
| algo (GDS) | Full | `kuzu-algo` | node2vec + random walk |
| neo4j | Full | `kuzu-neo4j` | Placeholder |
| llm | Full | `kuzu-llm` | Placeholder |
| sqlite | Full | `kuzu-sqlite` | "2 functions (placeholder)" |
| delta | Full | `kuzu-delta` | Placeholder |
| iceberg | Full | `kuzu-iceberg` | "3 functions (placeholder)" |
| azure | Full | `kuzu-azure` | Placeholder |
| postgres | Full | `kuzu-postgres` | "1 function (placeholder)" |
| unity-catalog | Full | `kuzu-unity-catalog` | Placeholder |
| adbc | Full (Ladybug only) | `kuzu-main/adbc.rs` | Basic wrapper |

---

## Prioritas Rekomendasi

### 🔴 Blocker (tidak bisa running query meaningful)

1. **CSR adjacency** (`csr.rs`) — `get_neighbors()` return empty. Tanpa ini, **graph traversal (MATCH (a)-[e]->(b)) tidak bekerja**.
2. **12 DDL no-op stubs** (`map_ddl.rs`) — CREATE/DROP/ALTER TABLE adalah no-op. Database tidak bisa di-schema.
3. **ORDER BY / LIMIT / SKIP** — grammar mem-parse tapi AST tidak menyimpan. Silent discard.
4. **DDL physical operators** — harus diimplementasi di `map_ddl.rs` untuk benar-benar modify catalog/storage.

### 🟡 High Priority

5. **Binder property type resolution** — ganti heuristic hardcoded dengan catalog lookup.
6. **Eliminasi kode duplikasi** antara `binder/mod.rs` dan `binder/dml.rs`.
7. **Planner harus produce LogicalOrderBy, LogicalLimit, LogicalSkip** dari AST yang sudah ada.
8. **Aggregate GROUP BY extraction** — saat ini hanya handle `COUNT(*)`, bukan `COUNT(x) GROUP BY y`.
9. **TableFunctionCall processor** — hanya vector_similarity_scan yang punya handler.
10. **Checkpoint `flush_table()` dari no-op jadi real persistence**.

### 🟠 Medium Priority

11. **Column batch scan** — ganti `scan_all_values()` → `Vec<Value>` dengan Arrow-format batch scan.
12. **StringDictionary compression** — pass-through saat ini, perlu encoding.
13. **ShadowFile disk persistence** — COW harus integrate dengan I/O.
14. **WAL truncation/rotation** saat checkpoint.
15. **Stats persistence** — saat ini in-memory only, hilang restart.
16. **RoaringBitmap Run container + serialization**.
17. **Binder `ORDER BY`/`LIMIT`/`SKIP` binding** — saat ini pass-through.
18. **Planner harus handle 15+ statement yang produce empty plan** — minimal delegasi ke kuzu-main untuk execution by side effect.

### 🟢 Low Priority (Ladybug-specific)

19. **IceDisk full implementation** (NodeTable + RelTable scan logic)
20. **Morsel-driven scan parallelism**
21. **PackedExtend + PackedChildSlices**
22. **ColumnarNodeTableBase / ColumnarRelTableBase hierarchy**
23. **ForeignRelTable**
24. **StorageFormat enum**

### 📦 Ekstensi yang perlu diisi (semua placeholder kecuali fts, vector, algo)

25. httpfs, duckdb, neo4j, llm, sqlite, delta, iceberg, azure, postgres, unity-catalog, adbc

---

## Metrik Keseluruhan

| Area | Perkiraan Completion | Baris Kode (Rust) |
|------|---------------------|-------------------|
| Common/Type System | ~95% | ~5K |
| Storage Engine | ~80% (core jalan, CSR broken) | ~15K |
| Transaction | ~95% | ~1K |
| Catalog | ~100% | ~1.5K |
| Parser | ~70% (grammar ok, AST missing fields) | ~2.5K + 477 line grammar |
| Binder | ~70% | ~3K |
| Planner | ~70% | ~2.5K |
| Optimizer | ~85% | ~3.5K |
| Processor | ~75% (45/60 operators) | ~8K |
| Function | ~90% | ~3K |
| Graph/GDS | ~40% | ~2K |
| Extensions | ~15% (3/15 implemented) | ~5K |
| CLI/WASM/C API | ~90% | ~1.5K |
| **Total** | **~70%** | **~55K+** |

**12 DDL no-op stubs** dan **CSR adjacency empty** adalah dua blocker terbesar yang membuat pipeline tidak bisa menghasilkan output query yang berarti untuk operasi graph database dasar.