# Konsolidasi Dokumen Markdown dan Teks

# Audit Komparasi Lengkap: Kuzu C++ (Vela) vs LadybugDB vs Kuzu Rust

> **Tanggal:** 2026-07-01  
> **Rust Workspace:** `kuzu-core/` — 28 crates, ~94 file .rs  
> **C++ Source:** `src/` — ~1.000 file (.h + .cpp)  
> **Ladybug:** `ladybug/` — Fork/rebrand Kuzu C++ v0.18.0  
> **Status Rust:** ✅ 691 tes, 0 failures, 0 compile errors

---

## 1. Ringkasan Eksekutif

Kuzu C++ (Vela) dan LadybugDB adalah basis kode yang **sama** — Ladybug adalah fork rebrand dari Kuzu C++. Keduanya berbagi arsitektur, API, dan ~99% kode yang identik (perbedaan: namespace `lbug::`, LLM extension, WASM, NaviX vector).

Kuzu Rust adalah **port ulang murni** (pure Rust, tanpa FFI/cxx) yang telah mencapai **paritas fungsional ~85%** dengan kemampuan inti yang lengkap: parser Cypher, binder, planner, 13 optimizer pass, 100+ fungsi built-in, MVCC transaction, columnar storage engine, ART + HNSW index, dan 15 extension crate. Namun terdapat **kesenjangan signifikan** di area GDS/graph algorithms dan query optimization.

### Metrik Perbandingan

| Metrik | C++ (Vela) | LadybugDB | Rust |
|--------|-----------|-----------|------|
| **Bahasa** | C++20 | C++20 | Rust 2024 |
| **File sumber** | ~1.000 | ~1.000 + forks | ~94 .rs |
| **Tes** | ~2.000+ | ~2.000+ | **691 ✅** |
| **Crate Rust** | — | `lbug` (wrapper cxx) | **28 crates** |
| **Binding** | C, Python, Node.js, Java, Rust(cxx) | + WASM, Go, Swift | CLI + lib |
| **Ekstensi** | 16 | 16 (including ADBC) | **15** |
| **Compile errors** | N/A | N/A | **0** |
| **Clippy warnings** | N/A | N/A | **128** |

---

## 2. Arsitektur Pipeline — Perbandingan per Layer

### 2.1 Parser

| Aspek | C++ (Vela) | Rust |
|-------|-----------|------|
| **Engine** | ANTLR4 (Java → C++) | `pest.rs` (PEG) |
| **Grammar** | `Cypher.g4` (ANTLR) | `cypher.pest` (PEG) |
| **AST types** | `ParsedStatement` hierarchy | `Statement` enum |
| **DDL Support** | Full | Full: CREATE/DROP TABLE, INDEX, SEQUENCE, VECTOR INDEX, COPY, ALTER |
| **DML Support** | Full | Full: MATCH, RETURN, WHERE, CREATE, DELETE, SET, MERGE, UNWIND, FOREACH, OPTIONAL MATCH, WITH |
| **Expressions** | Full | Full: all operators, function calls, subqueries, CASE, list/map/struct literals, parameters |
| **Variable-length paths** | ✅ `[*1..5]` | ✅ `lower_bound`/`upper_bound` |
| **Paritas** | **~95%** | Grammar mencakup semua fitur Cypher inti |

### 2.2 Binder

| Aspek | C++ (Vela) | Rust |
|-------|-----------|------|
| **Symbol resolution** | ✅ | ✅ Arc<Mutex<Catalog>> |
| **Type inference** | ✅ | ✅ 35+ tipe |
| **DDL binding** | ✅ | ✅ CREATE/DROP/ALTER/SEQUENCE/INDEX/VECTOR INDEX |
| **DML binding** | ✅ | ✅ MATCH/MERGE/UNWIND/FOREACH/OPTIONAL MATCH |
| **EXPLAIN** | ✅ | ✅ `BoundExplain` |
| **EXPORT/IMPORT DB** | ✅ | ✅ `BoundExportDatabase`/`BoundImportDatabase` |
| **Bound statements** | 20+ | **18 BoundStatement variants** |
| **Paritas** | **~90%** | Mungkin kurang beberapa edge case binding |

### 2.3 Planner

| Aspek | C++ (Vela) | Rust |
|-------|-----------|------|
| **Logical operators** | 35+ | **33 LogicalOperator variants** |
| **Scan** | ScanNodeTable, ScanRelTable, IndexLookUp | ScanNode, ScanRel, VectorSimilarityScan, ArtIndexRangeScan |
| **Relational** | Filter, Projection, HashJoin, CrossProduct, OrderBy, Limit, Aggregate, Union, Intersect, Flatten | ✅ semua |
| **Extend** | LogicalExtend (adjacency-guided extend) | ✅ (via ScanRel + adjacency) |
| **RecursiveExtend** | ✅ GDS-based, full path tracking | ✅ Basic BFS, path tracking terbatas |
| **SIP** | ✅ LogicalSemiMasker, SemiMaskPosition | ❌ **TIDAK ADA** |
| **Join Order** | ✅ CardinalityEstimator + CostModel + JoinTreeConstructor | ✅ Greedy join order enumeration |
| **Subquery** | ✅ Correlated subquery unnesting | ❌ **TIDAK ADA** |
| **Paritas** | **~75%** | Missing SIP, correlated subquery unnesting |

### 2.4 Optimizer

| Aspek | C++ (Vela) | Rust |
|-------|-----------|------|
| **Jumlah pass** | **16+** | **13 (11 flat + 2 tree)** |
| **Filter PushDown** | ✅ | ✅ |
| **Projection PushDown** | ✅ | ✅ |
| **Limit PushDown** | ✅ | ✅ |
| **Join Optimization** | ✅ | ✅ (termasuk greedy reorder) |
| **TopK** | ✅ | ✅ |
| **Factorization Rewriting** | ✅ | ✅ |
| **Cardinality Estimation** | ✅ | ✅ (termasuk StatsStore) |
| **Constant Folding** | ✅ | ✅ |
| **Common Subexpression Elimination** | ✅ | ✅ |
| **Aggregate Detection** | ✅ | ✅ |
| **Vector Similarity Detection** | ✅ | ✅ |
| **ART Range Scan Detection** | ✅ | ✅ |
| **Correlated Subquery Unnesting** | ✅ | ❌ **TIDAK ADA** |
| **SIP PushDown** | ✅ | ❌ **TIDAK ADA** |
| **Acc Hash Join** | ✅ | ❌ **TIDAK ADA** |
| **Remove Unnecessary Join** | ✅ | ✅ (RemoveUnnecessaryOperators) |
| **Foreign Join PushDown** | ✅ | ❌ **TIDAK ADA** |
| **Agg Key Dependency** | ✅ | ❌ **TIDAK ADA** |
| **Paritas** | **~70%** | Missing 5+ optimizer pass penting |

### 2.5 Processor / Execution Engine

| Aspek | C++ (Vela) | Rust |
|-------|-----------|------|
| **Model** | Vectorized + factorized | Vectorized (DataChunks) |
| **Physical operators** | 40+ | **18 physical executors** |
| **Scan** | ✅ | ✅ PhysicalScan, PhysicalScanRel |
| **Filter** | ✅ | ✅ |
| **Projection** | ✅ | ✅ |
| **HashJoin** | ✅ | ✅ (semua tipe Value) |
| **CrossProduct** | ✅ | ✅ |
| **OrderBy** | ✅ | ✅ |
| **Aggregate** | ✅ | ✅ (scalar + GROUP BY) |
| **Limit** | ✅ | ✅ |
| **Union** | ✅ | ✅ |
| **Intersect** | ✅ (dengan HashJoin shared state) | ✅ (basic, tanpa HJ integration) |
| **Flatten** | ✅ | ✅ |
| **SemiMasker** | ✅ | ❌ **TIDAK ADA** |
| **RecursiveExtend** | ✅ (GDS RJAlgorithm framework) | ✅ (basic BFS in-memory) |
| **VectorSimilarityScan** | ✅ | ✅ |
| **ArtIndexRangeScan** | ✅ | ✅ |
| **Expression Evaluator** | ✅ | ✅ |
| **Table functions** | ✅ | ✅ |
| **Delete/Set/Merge** | ✅ | ✅ |
| **Insert/BatchInsert** | ✅ | ✅ |
| **CopyTo/From** | ✅ (CSV, Parquet, NPY) | ✅ (CSV, Parquet) |
| **Explain** | ✅ | ✅ |
| **Paritas** | **~70%** | Missing SemiMasker, RecursiveExtend lebih sederhana |

### 2.6 Storage Engine

| Aspek | C++ (Vela) | Rust |
|-------|-----------|------|
| **Buffer Manager** | ✅ Clock eviction, VM regions | ✅ Clock eviction |
| **Page Management** | ✅ FileHandle, DiskArray | ✅ Page struct |
| **Columnar Tables** | ✅ NodeTable, RelTable, Column, ColumnChunk | ✅ NodeTable, RelTable, column, column_chunk, node_group |
| **CSR Adjacency** | ✅ Forward + backward | ✅ (via RelTable) |
| **Compression** | ✅ Boolean, constant, float, bitpacking int128 | ✅ Constant, boolean |
| **WAL** | ✅ Write-Ahead Log | ✅ wal, local_wal |
| **Shadow File** | ✅ Shadow paging | ✅ |
| **Checkpointer** | ✅ | ✅ |
| **Hash Index** | ✅ On-disk, in-memory | ✅ HashIndex, OnDiskHashIndex |
| **ART Index** | ✅ | ✅ ArtPrimaryKeyIndex (Node4/16/48/256) |
| **HNSW Index** | ✅ | ✅ HnswIndex (VectorIndexTable) |
| **Stats** | ✅ ColumnStats, TableStats, HyperLogLog | ✅ StatsStore |
| **Overflow** | ✅ Overflow pages for strings/lists | ✅ Lewat column_chunk |
| **Local Storage** | ✅ LocalNodeTable, LocalRelTable | ✅ local_storage |
| **Free Space Manager** | ✅ | ❌ **TIDAK ADA** |
| **Zone Map Predicate** | ✅ Min-max predicate skipping | ❌ **TIDAK ADA** |
| **Paritas** | **~80%** | Missing FSM, zone map predicates |

### 2.7 Transaction Manager

| Aspek | C++ (Vela) | Rust |
|-------|-----------|------|
| **MVCC** | ✅ | ✅ |
| **Isolation** | Serializable | Serializable |
| **Concurrent readers** | ✅ | ✅ |
| **Concurrent writers** | ✅ (configurable) | ✅ (configurable) |
| **Undo buffer** | ✅ | ✅ |
| **Locking** | Table-level | Table-level |
| **Auto-checkpoint** | ✅ | ✅ |
| **Paritas** | **~90%** | Mungkin kurang beberapa optimasi concurrent write |

### 2.8 Catalog

| Aspek | C++ (Vela) | Rust |
|-------|-----------|------|
| **Entry types** | 12+ | 8+ (NodeTable, RelTable, Sequence, Function, Index, VectorIndex, ForeignTable, Type) |
| **CRUD** | ✅ | ✅ |
| **Sequence** | ✅ | ✅ (SeqEntry, create/drop) |
| **Macro** | ✅ ScalarMacroCatalogEntry | ❌ **TIDAK ADA** |
| **Paritas** | **~85%** | Missing macro support |

### 2.9 Graph Engine

| Aspek | C++ (Vela) | Rust |
|-------|-----------|------|
| **CSR Adjacency** | ✅ | ✅ |
| **OnDiskGraph** | ✅ | ✅ |
| **GraphEntry** | ✅ | ✅ |
| **BFS** | ✅ (GDS) | ✅ (standalone) |
| **PageRank** | ✅ (GDS) | ✅ |
| **WCC** | ✅ (GDS) | ✅ |
| **SCC** | ✅ (GDS) | ✅ (Tarjan + Kosaraju) |
| **K-Core** | ✅ (GDS) | ✅ |
| **Louvain** | ✅ (GDS) | ✅ |
| **Spanning Forest** | ✅ (GDS) | ✅ |
| **SSP (shortest path)** | ✅ (GDS) | ❌ Placeholder only |
| **ASP** | ✅ (GDS) | ❌ Placeholder only |
| **WSP (weighted)** | ✅ (GDS) | ❌ Placeholder only |
| **AWSP** | ✅ (GDS) | ❌ Placeholder only |
| **Variable-length path** | ✅ (GDS) | ❌ (Rust punya BFS standalone) |
| **Paritas** | **~55%** | Gap terbesar: 8 algoritma GDS hilang |

---

## 3. Analisis Kesenjangan Detail

### 3.1 🔴 KRITIS: GDS Framework + Shortest Path (17 file C++)

**C++:** Framework GDS (`src/function/gds/`) memiliki 17 file implementasi + 18 header yang menyediakan:
- `RecJoin` — base class untuk semua recursive join algorithm
- `GDSFrontier` — SPSC queue-based frontier management
- `FrontierMorsel` — fine-grained parallelism
- `GDSComputeState` — per-thread compute state
- `GDSVertexCompute` — vertex-centric compute abstraction
- `GDSUtils` — algorithm execution orchestrator
- `OutputWriter` — result serialization
- `WeightUtils` — weight/edge property handling
- 8 algoritma shortest path: SSP(destinations+paths), ASP(destinations+paths), WSP(destinations+paths), AWSP, VariableLengthPath

**Rust:** `kuzu-algo/src/lib.rs` hanya memiliki **placeholder registration** untuk semua shortest path:
```rust
context.register_table_function("shortest_path", TableFunction::Custom { name: "shortest_path" });
// Tidak ada implementasi nyata!
```

**Dampak:** Semua Cypher shortest path queries tidak berfungsi.

**Estimasi porting:** ~10-14 hari

---

### 3.2 🔴 KRITIS: SIP / SemiMask (6+ file C++)

**C++:** Sideways Information Passing memungkinkan:
- `LogicalSemiMasker` — perencanaan SIP di level planner
- `BaseSemiMasker`, `SemiMaskerLocalState`, `SemiMaskerSharedState` — eksekusi fisik
- `SemiMaskKeyType::NODE`, `SemiMaskTargetType` — targeting scan nodes
- `SemiMaskPosition::PROHIBIT_PROBE_TO_BUILD` — kontrol arah propagasi
- Integrasi dengan `appendNodeSemiMask()`, `appendExtend()`, `appendJoin()`

**Rust:** **NOL** — tidak ada satu pun baris kode SIP/SemiMask.

**Dampak:** Query plan tidak bisa melakukan scan-side pruning berdasarkan filter dari sisi join lain. Untuk graph multi-hop, ini menyebabkan full scan yang tidak perlu. Query kompleks bisa 2-10× lebih lambat.

**Estimasi porting:** ~7-10 hari

---

### 3.3 🟡 SEDANG: Recursive Extend Enhancement

**Rust:** Sudah ada `PhysicalRecursiveExtend` dengan BFS in-memory sederhana menggunakan `HashMap<u64, Vec<u64>>`. Ini menghasilkan (src_offset, dst_offset, length) tanpa path detail.

**C++:** RecursiveExtend adalah `Sink` yang inherit dari `RJAlgorithm` — GDS framework penuh dengan:
- Path writing (node IDs, edge IDs)
- Weight property support
- Thread-safe shared state
- WALK/TRAIL/ACYCLIC semantic control
- Parallel execution

**Dampak:** Recursive MATCH queries bekerja di Rust, tapi tanpa path detail dan weight support. Untuk penggunaan umum, ini sudah fungsional, tetapi untuk analitik lanjutan, perlu upgrade.

**Estimasi porting:** ~5-7 hari (jika GDS framework sudah ada) atau ~3 hari (enhancement standalone)

---

### 3.4 🟡 SEDANG: Sequence Functions (nextval/currval)

**Rust:** Catalog sudah punya `SequenceEntry` lengkap dengan `next_k_val()`, `curr_val()`, `rollback_val()`. Tapi **tidak ada scalar function** `nextval()` atau `currval()` yang bisa dipanggil dari Cypher.

**C++:** `CurrValFunction` dan `NextValFunction` terdaftar sebagai scalar functions.

**Estimasi porting:** ~0.5 hari

---

### 3.5 🟢 RENDAH: Intersect Enhancement

**Rust:** PhysicalIntersect sudah ada dengan implementasi `HashMap<u64, Vec<...>>` dan pairwise sorted merge intersection. 7 test case pass.

**C++:** Intersect menggunakan HashJoin shared states penuh dengan selection vectors, multi-column key support, dan payload management terintegrasi.

**Estimasi porting:** ~1-2 hari

---

### 3.6 🟢 RENDAH: Array Value Function

**Rust:** Sudah punya 5 core array math functions + 9 aliases. Hanya `array_value(...)` yang missing.

**Estimasi porting:** ~0.5 hari

---

### 3.7 🟢 RENDAH: Free Space Manager

**C++:** `free_space_manager.h` — melacak ruang kosong di disk untuk alokasi page yang efisien.

**Rust:** Tidak ada. Storage manager menggunakan alokasi page sederhana.

**Dampak:** Fragmentasi disk pada database besar. Tidak kritis untuk penggunaan umum.

**Estimasi porting:** ~2-3 hari

---

### 3.8 🟢 RENDAH: Zone Map Predicate Skipping

**C++:** `predicate/column_predicate.h`, `constant_predicate.h`, `null_predicate.h` — min-max zone maps untuk skip page yang tidak relevan saat scan.

**Rust:** Tidak ada.

**Dampak:** Full scan pada filter yang selektif. Optimasi performa, bukan fungsionalitas.

**Estimasi porting:** ~3-5 hari

---

### 3.9 ⚪ INFORMASIONAL: Optimizer Passes Tambahan

| Missing Pass | C++ Source | Rust | Estimasi |
|-------------|-----------|------|----------|
| Correlated Subquery Unnesting | `correlated_subquery_unnest_solver.cpp` | ❌ | ~4-5 hari |
| Acc Hash Join Optimization | `acc_hash_join_optimizer.cpp` | ❌ | ~2-3 hari |
| Foreign Join PushDown | (planner) | ❌ | ~2-3 hari |
| Agg Key Dependency | `agg_key_dependency_optimizer.cpp` | ❌ | ~1-2 hari |

**Total estimasi optimizer:** ~9-13 hari

---

### 3.10 ⚪ INFORMASIONAL: Macro Support

**C++:** Memiliki `ScalarMacroCatalogEntry` untuk definisi macro Cypher (`CREATE MACRO`).

**Rust:** Tidak ada.

**Estimasi:** ~2-3 hari

---

## 4. Perbandingan Ekstensi

| Ekstensi | C++ (Vela) | LadybugDB | Rust |
|----------|-----------|-----------|------|
| **algo** (graph algos) | ✅ Full GDS | ✅ Full GDS | ⚠️ Partial (7/14 algos) |
| **json** | ✅ | ✅ | ✅ (11+ functions) |
| **fts** | ✅ | ✅ | ✅ (stemmer, BM25, TF-IDF) |
| **vector** (HNSW) | ✅ | ✅ (+NaviX) | ✅ (Cosine/Euclidean/L1/L2/Dot) |
| **httpfs** | ✅ | ✅ | ✅ |
| **duckdb** | ✅ | ✅ | ✅ |
| **postgres** | ✅ | ✅ | ✅ |
| **sqlite** | ✅ | ✅ | ✅ |
| **neo4j** | ✅ | ✅ | ✅ |
| **llm** | — | ✅ | ✅ |
| **delta** | — | ✅ | ✅ |
| **iceberg** | — | ✅ | ✅ |
| **azure** | — | ✅ | ✅ |
| **unity-catalog** | — | ✅ | ✅ |
| **adbc** | — | ✅ | ❌ **TIDAK ADA** |
| **wasm** | — | ✅ | ✅ (target support) |

**Catatan:** Rust memiliki **lebih banyak** extension integrations (postgres, sqlite, delta, iceberg, azure, unity-catalog, neo4j) dibanding C++ Vela asli, karena ini adalah pengembangan baru di Rust. LadybugDB memiliki ADBC (Arrow DB Connectivity) yang tidak ada di Rust.

---

## 5. Perbandingan Non-Fungsional

### 5.1 Performa (Micro-Benchmark)

| Operator | Ukuran | Rust (µs) | C++ (est.) | Gap |
|----------|--------|-----------|-----------|------|
| Seq Scan | 10K rows, 4 cols | **1.050** | TBD | — |
| Filter (pass all) | 10K | **433** | TBD | — |
| Hash Join | 1K × 1K | **1.440** | TBD | — |
| Order By (1 key) | 1K | **73,4** | TBD | — |
| Agg COUNT | 10K | **158** | TBD | — |
| Agg GROUP BY | 10K, 10 groups | **1.060** | TBD | — |

> **Catatan:** C++ benchmark membutuhkan dataset terserialisasi yang belum tersedia di workspace ini. Lihat `BENCHMARK_COMPARISON.md` untuk instruksi setup.

### 5.2 Kualitas Kode

| Metrik | Rust |
|--------|------|
| **Compile errors** | **0** ✅ |
| **Compile warnings** | **0** ✅ |
| **Test pass** | **691 / 691** ✅ |
| **Clippy errors** | **0** ✅ |
| **Clippy warnings** | **128** ⚠️ |
| **Crate count** | **28** |
| **Modularity** | Sangat baik (per-crate separation) |

### 5.3 Ukuran Kode

| Aspek | C++ (Vela) | Rust |
|-------|-----------|------|
| **Source files** | ~1.000 | ~94 |
| **Header/source ratio** | 540:460 | N/A (no headers) |
| **Lines of code (est.)** | ~250.000 | ~25.000 |
| **Port ratio** | 100% | ~10% LOC (lebih padat) |

---

## 6. Rencana Porting/Refaktor — Prioritas

### Fase 1: GDS Framework Inti 🔴 (10-14 hari)

Tujuan: Porting framework GDS C++ ke Rust sebagai prasyarat untuk shortest path algorithms.

**Langkah-langkah:**

| # | Task | File C++ Referensi | Target Rust | Hari |
|---|------|-------------------|-------------|------|
| 1.1 | `GDSFrontier` — SPSC queue + bitset frontier | `gds_frontier.cpp/h` | `kuzu-graph/src/gds_frontier.rs` | 1 |
| 1.2 | `FrontierMorsel` — fine-grained parallelism | `frontier_morsel.cpp/h` | `kuzu-graph/src/frontier_morsel.rs` | 1 |
| 1.3 | `GDSComputeState` — per-thread compute state | `gds_state.cpp/h` | `kuzu-graph/src/gds_state.rs` | 1 |
| 1.4 | `RecJoin` — base class untuk recursive join | `rec_joins.cpp/h` | `kuzu-graph/src/rec_joins.rs` | 2 |
| 1.5 | `GDSUtils` — algorithm execution orchestrator | `gds_utils.cpp/h` | `kuzu-graph/src/gds_utils.rs` | 1 |
| 1.6 | `OutputWriter` — result serialization | `output_writer.cpp/h` | `kuzu-graph/src/output_writer.rs` | 1 |
| 1.7 | `BFSGraph` — frontier-based BFS operations | `bfs_graph.cpp/h` | `kuzu-graph/src/bfs_graph.rs` | 1 |
| 1.8 | Integrasi GDS dengan `kuzu-algo` | `gds.cpp/h` | `kuzu-algo/src/gds.rs` | 1 |
| 1.9 | Bind + register sebagai Cypher CALL functions | `gds_function_collection.h` | `kuzu-algo/src/lib.rs` | 0.5 |
| 1.10 | Testing & benchmarking | — | — | 1 |

### Fase 2: Shortest Path Algorithms 🔴 (5-7 hari)

Tujuan: Implementasi 8 algoritma shortest path di atas GDS framework.

| # | Task | File C++ Referensi | Target Rust | Hari |
|---|------|-------------------|-------------|------|
| 2.1 | SSP (destinations) | `ssp_destinations.cpp` | `kuzu-algo/src/shortest_path.rs` | 1 |
| 2.2 | SSP (paths) | `ssp_paths.cpp` | Sama | 1 |
| 2.3 | ASP (destinations) | `asp_destinations.cpp` | Sama | 1 |
| 2.4 | ASP (paths) | `asp_paths.cpp` | Sama | 1 |
| 2.5 | WSP (destinations) | `wsp_destinations.cpp` | Sama | 1 |
| 2.6 | WSP (paths) | `wsp_paths.cpp` | Sama | 1 |
| 2.7 | AWSP | `awsp_paths.cpp` | Sama | 0.5 |
| 2.8 | Upgrade RecursiveExtend ke GDS RJAlgorithm | `recursive_extend.h` | `kuzu-processor/src/physical_operator.rs` | 1 |

### Fase 3: SIP / SemiMask 🔴 (7-10 hari)

Tujuan: Implementasi Sideways Information Passing untuk query optimization.

| # | Task | File C++ Referensi | Target Rust | Hari |
|---|------|-------------------|-------------|------|
| 3.1 | `LogicalSemiMasker` operator | `logical_semi_masker.cpp` | `kuzu-planner/src/logical_operator.rs` | 1 |
| 3.2 | `SemiMaskTargetType`, `SemiMaskPosition` enums | `semi_masker.h` | `kuzu-planner/src/enums.rs` | 0.5 |
| 3.3 | Parser/binder untuk SEMI_MASK | — | — | 0.5 |
| 3.4 | Planner — appendNodeSemiMask() | `plan_node_semi_mask.cpp` | `kuzu-planner/src/planner.rs` | 2 |
| 3.5 | Planner — SIP integration di appendJoin/appendExtend | `append_join.cpp`, `append_extend.cpp` | `kuzu-planner/src/planner.rs` | 2 |
| 3.6 | Physical `BaseSemiMasker` operator | `semi_masker.h` | `kuzu-processor/src/physical_operator.rs` | 2 |
| 3.7 | Processor dispatch | `processor.cpp` | `kuzu-processor/src/processor.rs` | 0.5 |
| 3.8 | Testing | — | — | 1 |

### Fase 4: Optimizer Passes Tambahan 🟡 (9-13 hari)

| # | Task | Referensi C++ | Target Rust | Hari |
|---|------|--------------|-------------|------|
| 4.1 | Correlated Subquery Unnesting | `correlated_subquery_unnest_solver.cpp` | `kuzu-optimizer/src/passes.rs` | 4-5 |
| 4.2 | Acc Hash Join Optimization | `acc_hash_join_optimizer.cpp` | `kuzu-optimizer/src/passes.rs` | 2-3 |
| 4.3 | Foreign Join PushDown | (planner) | `kuzu-planner/src/planner.rs` | 2-3 |
| 4.4 | Agg Key Dependency | `agg_key_dependency_optimizer.cpp` | `kuzu-optimizer/src/passes.rs` | 1-2 |

### Fase 5: Storage Enhancement 🟡 (5-8 hari)

| # | Task | Referensi C++ | Target Rust | Hari |
|---|------|--------------|-------------|------|
| 5.1 | Free Space Manager | `free_space_manager.cpp/h` | `kuzu-storage/src/free_space_manager.rs` | 2-3 |
| 5.2 | Zone Map Predicate Skipping | `predicate/` (column_predicate, constant_predicate, null_predicate) | `kuzu-storage/src/predicate.rs` | 3-5 |

### Fase 6: Fitur Minor 🟢 (2-4 hari)

| # | Task | Referensi C++ | Target Rust | Hari |
|---|------|--------------|-------------|------|
| 6.1 | nextval()/currval() scalar functions | `sequence_functions.cpp` | `kuzu-function/src/scalar.rs` + `kuzu-function/src/registry.rs` | 0.5 |
| 6.2 | SERIAL auto-increment | — | `kuzu-function/src/registry.rs` | 0.5 |
| 6.3 | array_value() function | `array_value.cpp` | `kuzu-function/src/scalar.rs` | 0.5 |
| 6.4 | Intersect: HashJoin shared state reuse | `intersect.cpp/h` | `kuzu-processor/src/physical_operator.rs` | 1-2 |
| 6.5 | Macro support (CREATE MACRO) | `scalar_macro_catalog_entry.h` | `kuzu-catalog/src/lib.rs` | 1-2 |

### Fase 7: Code Quality ⚪ (3-5 hari)

| # | Task | Target | Hari |
|---|------|--------|------|
| 7.1 | Resolve 128 clippy warnings | All crates | 2 |
| 7.2 | Cargo fmt — consistent style | All crates | 0.5 |
| 7.3 | Add #[deny(unsafe_code)] where possible | All crates | 0.5 |
| 7.4 | Documentation coverage pass | All crates | 1 |
| 7.5 | Benchmark gap analysis (C++ vs Rust) | BENCHMARK_COMPARISON.md | 1 |

### Ringkasan Timeline

| Fase | Prioritas | Hari | Dependencies |
|------|-----------|------|-------------|
| Fase 1: GDS Framework | 🔴 Critical | 10-14 | — |
| Fase 2: Shortest Path | 🔴 Critical | 5-7 | Fase 1 |
| Fase 3: SIP / SemiMask | 🔴 Critical | 7-10 | — |
| Fase 4: Optimizer Passes | 🟡 Medium | 9-13 | — |
| Fase 5: Storage Enhancements | 🟡 Medium | 5-8 | — |
| Fase 6: Fitur Minor | 🟢 Low | 2-4 | — |
| Fase 7: Code Quality | ⚪ Info | 3-5 | — |
| **Total** | | **41-61 hari** | |
| **Parallelizable max** | | **~25-35 hari** | Fase 1∥3, 4∥5∥6 |

---

## 7. Ringkasan Status per Komponen

| Komponen | C++ (Vela) | LadybugDB | Rust | Gap |
|----------|-----------|-----------|------|-----|
| **Parser** | ✅ 100% | ✅ 100% | ✅ ~95% | Minor |
| **Binder** | ✅ 100% | ✅ 100% | ✅ ~90% | Minor |
| **Planner** | ✅ 100% | ✅ 100% | ⚠️ ~75% | SIP, subquery unnesting |
| **Optimizer** | ✅ 100% (16+ pass) | ✅ 100% | ⚠️ ~70% | 5 pass missing |
| **Processor** | ✅ 100% (40+ ops) | ✅ 100% | ⚠️ ~70% | SemiMasker, RecExtend upgrade |
| **Storage** | ✅ 100% | ✅ 100% | ⚠️ ~80% | FSM, zone map predicates |
| **Transaction** | ✅ 100% | ✅ 100% | ✅ ~90% | Minor |
| **Catalog** | ✅ 100% | ✅ 100% | ✅ ~85% | Macro support |
| **Graph** | ✅ 100% (GDS) | ✅ 100% | ⚠️ ~55% | **Gap terbesar** |
| **Functions** | ✅ 100+ | ✅ 100+ | ✅ 100+ | nextval/currval, array_value |
| **Extensions** | 16 | 16+ | **15** | ADBC missing |
| **CLI** | ✅ full | ✅ full | ⚠️ basic | Rustyline REPL |
| **WASM** | ❌ | ✅ | ✅ (target) | Compile check only |
| **CI/CD** | ✅ | ✅ | ❌ | Not configured |

### Paritas Keseluruhan: **~82%**

---

# Kuzu Rust vs C++/LadybugDB: Missing Features Deep-Dive

## 1. Recursive Extend

### Rust (present in both crates)
| File | Status |
|------|--------|
| `kuzu-core/kuzu-planner/src/logical_operator.rs` (line 35, 420) | ✅ `LogicalRecursiveExtend` — has source/target var, rel_table_ids, lower/upper bound, direction, semantic |
| `kuzu-core/kuzu-planner/src/planner.rs` (line 280-340) | ✅ Planner creates `RecursiveExtend` for var-length edge patterns, skips consumed dest node |
| `kuzu-core/kuzu-processor/src/physical_operator.rs` (line 2390-2440+) | ✅ `PhysicalRecursiveExtend` — simple **in-memory BFS** using `HashMap<u64, Vec<u64>>` adjacency built from catalog |
| `kuzu-core/kuzu-processor/src/processor.rs` (line 140, 941) | ✅ Maps logical→physical, formats as `"RecursiveExtend({}..{})"` |

### C++ (`src/include/processor/operator/recursive_extend.h`)
Class `RecursiveExtend` extends `Sink`, NOT a standalone operator:
- Inherits from `function::RJAlgorithm` — the **GDS recursive join framework**
- Has `RJBindData`, shared state (`RecursiveExtendSharedState`), `GDSComputeState`
- Uses `RJOutputWriter` for path writing
- Method `isSource() = true`, `isParallel() = false`

### Key Gaps (Rust is missing)
| Feature | C++ | Rust |
|---------|-----|------|
| GDS framework integration | ✅ RJAlgorithm base class | ❌ Simple HashMap BFS |
| Path writing | ✅ PathsOutputWriterInfo | ❌ Emits only (src_offset, dst_offset, length) |
| Shared state across threads | ✅ RecursiveExtendSharedState | ❌ Not thread-safe |
| Path node/edge ID tracking | ✅ pathNodeIDsExpr, pathEdgeIDsExpr | ❌ |
| Weight property support | ✅ weightPropertyExpr | ❌ |
| Semantic control (WALK/TRAIL/ACYCLIC) | ✅ Binder-level | ✅ Logical-level only |

---

## 2. Shortest Path in GDS

### C++ GDS implementations — 17 source files:
| File | Description |
|------|-------------|
| `src/function/gds/bfs_graph.cpp` | BFS graph operations (frontier-based) |
| `src/function/gds/ssp_destinations.cpp` | **Single Shortest Path** (destinations only) |
| `src/function/gds/ssp_paths.cpp` | **Single Shortest Path** (with paths) |
| `src/function/gds/asp_destinations.cpp` | **All Shortest Paths** (destinations) |
| `src/function/gds/asp_paths.cpp` | **All Shortest Paths** (with paths) |
| `src/function/gds/wsp_destinations.cpp` | **Weighted Shortest Path** (destinations) |
| `src/function/gds/wsp_paths.cpp` | **Weighted Shortest Path** (with paths) |
| `src/function/gds/awsp_paths.cpp` | **All Weighted Shortest Paths** |
| `src/function/gds/variable_length_path.cpp` | Variable-length (non-shortest) path traversal |
| `src/function/gds/gds.cpp` | Main GDS framework (binding, execution, registration) |
| `src/function/gds/gds_frontier.cpp` | Frontier management (SPSC queue, bitset) |
| `src/function/gds/gds_state.cpp` | GDS execution state |
| `src/function/gds/gds_task.cpp` | Task scheduling |
| `src/function/gds/gds_utils.cpp` | Utilities |
| `src/function/gds/output_writer.cpp` | Result output writer |
| `src/function/gds/rec_joins.cpp` | Recursive join (base for all RJ algorithms) |
| `src/function/gds/frontier_morsel.cpp` | Fine-grained frontier parallelism |

### C++ GDS headers — 18 headers:
`gds.h`, `gds_frontier.h`, `gds_state.h`, `gds_task.h`, `gds_utils.h`, `rec_joins.h`, `rj_output_writer.h`, `compute.h`, `weight_utils.h`, `bfs_graph.h`, `frontier_morsel.h`, `gds_function_collection.h`, `gds_object_manager.h`, `gds_vertex_compute.h`, `density_state.h`, `auxiliary_state/gds_auxilary_state.h`, `auxiliary_state/path_auxiliary_state.h`

### Rust (`kuzu-core/kuzu-algo/src/lib.rs`):
The algo extension **registers** these table functions but they are `TableFunction::Custom { name }` placeholders — **NO actual implementations**:
```rust
context.register_table_function("shortest_path", TableFunction::Custom { name: "shortest_path" });
context.register_table_function("weighted_shortest_path", TableFunction::Custom { name: "weighted_shortest_path" });
context.register_table_function("all_sp_destinations", TableFunction::Custom { name: "all_sp_destinations" });
context.register_table_function("sp", ...);
```

Only these algorithms have real implementations: PageRank, WCC, SCC (Tarjan), SCC (Kosaraju), K-Core, Louvain, Spanning Forest — all in `lib.rs`.

### Key Gaps (Rust is missing)
| Algorithm | C++ | Rust |
|-----------|-----|------|
| BFS graph | ✅ bfs_graph.cpp | ❌ |
| SSP (destinations) | ✅ ssp_destinations.cpp | ❌ |
| SSP (paths) | ✅ ssp_paths.cpp | ❌ |
| ASP (destinations) | ✅ asp_destinations.cpp | ❌ |
| ASP (paths) | ✅ asp_paths.cpp | ❌ |
| WSP (destinations) | ✅ wsp_destinations.cpp | ❌ |
| WSP (paths) | ✅ wsp_paths.cpp | ❌ |
| AWSP | ✅ awsp_paths.cpp | ❌ |
| Variable-length path | ✅ variable_length_path.cpp | ❌ (Rust has standalone simple BFS) |
| Frontier parallelism | ✅ frontier_morsel.cpp | ❌ |
| Full GDS framework | ✅ gds.cpp, gds_state.cpp etc. | ❌ |

---

## 3. Intersect

### Rust (present)
| File | Status |
|------|--------|
| `kuzu-core/kuzu-planner/src/logical_operator.rs` (line 33, 393) | ✅ `LogicalIntersect` — `num_build_sides`, `build_key_exprs`, left/right children |
| `kuzu-core/kuzu-processor/src/physical_operator.rs` (line 1255-1340+) | ✅ `PhysicalIntersect` — builds HashMaps per build side, pairwise sorted merge intersection |
| `kuzu-core/kuzu-processor/src/processor.rs` (line 242) | ✅ Maps logical→physical |
| Tests | ✅ 7 test cases in `processor.rs` |

### C++ (`src/include/processor/operator/intersect/intersect.h`)
Class `Intersect` extends `PhysicalOperator`:
- Uses **HashJoin shared states** (`HashJoinSharedState`) for build — full HT infrastructure
- `probeHTs()`, `twoWayIntersect()`, `intersectLists()`, `populatePayloads()`
- Selection vectors for tracking match positions
- `probedFlatTuples`, `tupleIdxPerBuildSide` for managing multi-build-side iteration

### Key Differences
| Aspect | C++ | Rust |
|--------|-----|------|
| Build infrastructure | Full HashJoin shared states | Simple `HashMap<u64, Vec<(Value, Vec<(usize,usize)>)>>` |
| Selection vectors | ✅ `intersectSelVectors` | ❌ Manual index tracking |
| Payload management | ✅ `populatePayloads()` with column mappings | ❌ Basic row extraction |
| Multi-column keys | ✅ | ❌ (single key column index) |
| Overlap with HashJoin | ✅ Reuses HJ build | ❌ Independent implementation |

---

## 4. SIP / SemiMask

### C++ — Fully implemented
| File | Contents |
|------|----------|
| `src/include/processor/operator/semi_masker.h` | `BaseSemiMasker`, `SemiMaskerLocalState`, `SemiMaskerSharedState`, `SemiMaskerPrintInfo` |
| `src/planner/operator/sip/logical_semi_masker.cpp` | `LogicalSemiMasker` — SIP planning |
| `src/planner/plan/plan_node_semi_mask.cpp` | `appendNodeSemiMask()` with `SemiMaskTargetType` (RECURSIVE_EXTEND_PATH_NODE, etc.) |
| `src/planner/plan/append_join.cpp` | `SemiMaskPosition::PROHIBIT_PROBE_TO_BUILD` etc. |
| `src/planner/plan/append_extend.cpp` | Semi-mask plans for recursive extend |
| `src/include/processor/operator/physical_operator.h` | `PhysicalOperatorType::SEMI_MASKER` |

Concepts: `SemiMaskKeyType::NODE`, `SemiMaskTargetType`, `SemiMaskPosition` (PROHIBIT, PROBE_TO_BUILD, etc.)

LadybugDB also mirrors this fully.

### Rust — **COMPLETELY MISSING**
| Search Target | Results |
|---------------|---------|
| `semi_mask` / `SemiMask` in `kuzu-core/` | **0 hits** |
| `side_way` / `sideway` in `kuzu-core/` | **0 hits** |
| `SIP` in `kuzu-core/` | **0 hits** |
| `SemiMasker` in `kuzu-planner/src/logical_operator.rs` | **0 hits** |
| `SemiMasker` in `kuzu-processor/src/` | **0 hits** |

Rust only has `LogicalSemiJoin` and `LogicalAntiJoin` (basic semi/anti joins) — no semi-masker or side-ways information passing.

### Critical Gap
**SIP (Sideways Information Passing)** is a major optimization that allows the planner to push filter masks from one side of a join to the other before execution. Without it, Rust's query execution cannot prune scan nodes based on what's needed by downstream operators.

---

## 5. Sequence / nextval / currval

### Rust Catalog — ✅ Present
`kuzu-core/kuzu-catalog/src/lib.rs` (line 90-170):
- `SequenceEntry` struct with: `sequence_id`, `name`, `usage_count`, `curr_val`, `increment`, `start_value`, `min_value`, `max_value`, `cycle`
- Methods: `curr_val()`, `next_k_val(count)`, `rollback_val()`, `get_serial_name()`
- `CatalogEntry::Sequence` variant
- `create_sequence()`, `get_sequence()`, `get_sequence_mut()`, `drop_sequence()`, `sequences()`
- Tests: 20+ test cases

### Rust Function Registry — **MISSING**
`kuzu-core/kuzu-function/src/registry.rs` — **zero hits** for `nextval`, `currval`, `next_val`, `curr_val`, or `sequence`.

### C++ — Fully present
`src/function/sequence/sequence_functions.cpp`:
- `CurrValFunction` — returns current value without advancing
- `NextValFunction` — advances and returns next value (isReadOnly = false)
- Registered as scalar functions taking `STRING` (sequence name) and returning `INT64`

### Gap
| Component | C++ | Rust |
|-----------|-----|------|
| Catalog entry | ✅ | ✅ |
| `nextval()` scalar function | ✅ | ❌ |
| `currval()` scalar function | ✅ | ❌ |
| `CREATE SEQUENCE` DDL | ✅ (binder present) | ✅ (binder present) |
| `SERIAL` auto-increment | ✅ | ❌ (get_serial_name exists but no auto-increment logic) |

---

## 6. Schema Functions (OFFSET, ID, START_NODE, END_NODE, LABEL)

### Rust — ✅ **FULLY IMPLEMENTED**
| File | Details |
|------|---------|
| `kuzu-core/kuzu-function/src/registry.rs` (line 224-231) | `SchemaOp` enum: `Offset`, `Id`, `StartNode`, `EndNode`, `Label` |
| `kuzu-core/kuzu-function/src/registry.rs` (line 565-568) | Registration: `OFFSET`, `ID`, `START_NODE`, `END_NODE`, `LABEL` |
| `kuzu-core/kuzu-function/src/scalar.rs` (line 1077-1170+) | `evaluate_schema()` — full implementation with struct field extraction (`_id`, `_src`, `_dst`, `_label`) |
| Tests (line 2286-2377) | ✅ 10+ test cases covering InternalID, Struct, error cases |

### C++ Comparison
C++ has equivalent functions in `function/schema/` (via `vector_node_rel_functions.cpp`). Rust's implementation is comparable.

### Status: **NO GAP** — All 5 schema functions are present.

---

## 7. EXPLAIN

### Rust — ✅ **FULLY IMPLEMENTED ACROSS ALL LAYERS**
| Layer | File | Details |
|-------|------|---------|
| Parser | AST: `ExplainStatement` with `ExplainType` | ✅ |
| Binder | `kuzu-core/kuzu-binder/src/binder.rs` line 973 | `bind_explain()` → `BoundExplain { inner, explain_type }` |
| Binder | `kuzu-core/kuzu-binder/src/bound_statement.rs` line 100-107 | `BoundExplain` struct |
| Planner | `kuzu-core/kuzu-planner/src/logical_operator.rs` line 321-330 | `LogicalExplain { inner, explain_type, cardinality }` |
| Planner | `kuzu-core/kuzu-planner/src/planner.rs` line 48-66 | `plan_explain()` — plans inner statement, wraps in LogicalExplain |
| Processor | `kuzu-core/kuzu-processor/src/physical_operator.rs` line 2351-2370+ | `PhysicalExplain` — serializes plan tree |
| Processor | `kuzu-core/kuzu-processor/src/processor.rs` line 252-258 | Maps to `PhysicalExplain` with `serialize_plan_tree()` |

### Status: **NO GAP** — EXPLAIN is fully implemented.

---

## 8. IMPORT/EXPORT DATABASE

### Rust — ✅ **FULLY IMPLEMENTED ACROSS ALL LAYERS**
| Layer | File | Details |
|-------|------|---------|
| Parser | `kuzu-core/kuzu-parser/src/ast.rs` | `ExportDatabase { file_path, options }`, `ImportDatabase { file_path }` |
| Parser | `kuzu-core/kuzu-parser/src/parser.rs` line 217 | Parsing logic |
| Binder | `kuzu-core/kuzu-binder/src/binder.rs` line 1037-1087 | `bind_export_database()`, `bind_import_database()` |
| Binder | `kuzu-core/kuzu-binder/src/bound_statement.rs` line 230-250 | `BoundExportDatabase { file_path, file_type, schema_only, options }`, `BoundImportDatabase { file_path, query, index_query }` |
| Planner | `kuzu-core/kuzu-planner/src/logical_operator.rs` | `LogicalExportDatabase`, `LogicalImportDatabase` |
| Planner | `kuzu-core/kuzu-planner/src/planner.rs` | `plan_export_database()`, `plan_import_database()` |
| Processor | `kuzu-core/kuzu-processor/src/processor.rs` line 497-498 | Handled as DDL-like (returns empty result) |
| Optimizer | `kuzu-core/kuzu-optimizer/src/join_order.rs` line 109 | Recognized in join ordering |
| Optimizer | `kuzu-core/kuzu-optimizer/src/passes.rs` line 826 | Recognized in optimization passes |

### C++ — Additional detail
C++ has `export_csv_function.cpp` and `export_parquet_function.cpp` in `src/function/export/` for format-specific export logic.

### Status: **NO MAJOR GAP** — Framework is fully implemented. Rust may lack format-specific export drivers (e.g., Parquet export).

---

## 9. Array Math Functions

### Rust — ✅ **5 core functions + 9 aliases present**
| Function | Implementation | Status |
|----------|---------------|--------|
| `array_cosine_similarity` | `ArrayOp::CosineSimilarity` | ✅ |
| `array_distance` | `ArrayOp::Distance` | ✅ |
| `array_inner_product` | `ArrayOp::InnerProduct` | ✅ |
| `array_cross_product` | `ArrayOp::CrossProduct` | ✅ |
| `array_squared_distance` | `ArrayOp::SquaredDistance` | ✅ |
| `array_concat` / `array_cat` | Delegates to `ListOp::Concat` | ✅ |
| `array_append` / `array_push_back` | Delegates to `ListOp::Append` | ✅ |
| `array_prepend` / `array_push_front` | Delegates to `ListOp::Prepend` | ✅ |
| `array_contains` / `array_has` | Delegates to `ListOp::Contains` | ✅ |
| `array_slice` | Delegates to `ListOp::Slice` | ✅ |

### C++ — Additional functions
| File | Description |
|------|-------------|
| `src/function/array/array_functions.cpp` | `ArrayCrossProductFunction`, `ArrayCosineSimilarityFunction`, `ArrayDistanceFunction`, `ArrayInnerProductFunction`, `ArraySquaredDistanceFunction` — with **type-specific template specializations** for int8, int16, int32, int64, int128, float, double |
| `src/function/array/array_value.cpp` | **`ArrayValueFunction`** — constructs an ARRAY from individual arguments (varargs) |
| Headers | 5 function headers in `array/functions/` + `vector_array_functions.h` |

### Gaps
| Feature | C++ | Rust |
|---------|-----|------|
| Type-specific dispatch | ✅ Template specializations for 7 numeric types | ❌ Single Dynamic dispatch on Value |
| `array_value(...)` (varargs→array) | ✅ `ArrayValueFunction` | ❌ |
| Proper ARRAY type (fixed-length) | ✅ `LogicalType::ARRAY` with fixed length | ✅ (via List) |

---

## 10. GDS Algorithms in C++ — Complete Directory Listing

### `src/function/gds/` (17 source files)
```
asp_destinations.cpp
asp_paths.cpp
awsp_paths.cpp
bfs_graph.cpp
frontier_morsel.cpp
gds.cpp
gds_frontier.cpp
gds_state.cpp
gds_task.cpp
gds_utils.cpp
output_writer.cpp
rec_joins.cpp
ssp_destinations.cpp
ssp_paths.cpp
variable_length_path.cpp
wsp_destinations.cpp
wsp_paths.cpp
CMakeLists.txt
```

### `src/include/function/gds/` (18 header files)
```
auxiliary_state/
  gds_auxilary_state.h
  path_auxiliary_state.h
bfs_graph.h
compute.h
density_state.h
frontier_morsel.h
gds.h
gds_frontier.h
gds_function_collection.h
gds_object_manager.h
gds_state.h
gds_task.h
gds_utils.h
gds_vertex_compute.h
rec_joins.h
rj_output_writer.h
weight_utils.h
```

### Rust kuzu-algo comparison (`kuzu-core/kuzu-algo/src/`):
Just **1 file**: `lib.rs`

Contains real implementations for: PageRank, WCC, SCC (Tarjan & Kosaraju), K-Core, Louvain, Spanning Forest.

All shortest-path algorithms, BFS graph, GDS framework, frontier management, output writers, compute states — **COMPLETELY ABSENT**.

---

## Summary of Gaps

| # | Feature | Rust Status | Gap Severity |
|---|---------|------------|--------------|
| 1 | Recursive Extend | ✅ Basic BFS present, ❌ No GDS/RJ framework | **MEDIUM** |
| 2 | Shortest Path (GDS) | ❌ Placeholder registrations only (no implementations) | **CRITICAL** |
| 3 | Intersect | ✅ Basic implementation, ❌ No HashJoin integration | **LOW-MEDIUM** |
| 4 | SIP / SemiMask | ❌ **Completely missing** | **CRITICAL** |
| 5 | nextval/currval | ✅ Catalog support, ❌ **No scalar functions** | **MEDIUM** |
| 6 | Schema functions | ✅ Fully implemented | **NONE** |
| 7 | EXPLAIN | ✅ Fully implemented | **NONE** |
| 8 | IMPORT/EXPORT DB | ✅ Framework present, ❌ May lack format drivers | **LOW** |
| 9 | Array math | ✅ 5 core + 9 aliases, ❌ Missing `array_value()` | **LOW** |
| 10 | GDS algorithms | ❌ 17 C++ files → 0 Rust implementations | **CRITICAL** |

---

Here is the complete exploration report for the **KuzuDB** C++ source code in `src/`:

---

## 1. Top-Level Directories Under `src/`

| Directory | Purpose |
|-----------|---------|
| `antlr4/` | ANTLR4 grammar files (`Cypher.g4`, `keywords.txt`) |
| `binder/` | Binder — binds parsed statements to catalog/types |
| `c_api/` | C API wrapper (kuzu.h) |
| `catalog/` | Catalog — metadata for tables, types, sequences, etc. |
| `common/` | Common types, enums, utilities, file system, serialization |
| `expression_evaluator/` | Expression evaluation engine |
| `extension/` | Extension management system |
| `function/` | Built-in functions (scalar, aggregate, table, GDS) |
| `graph/` | Graph abstraction (on-disk graph, graph entries) |
| `include/` | **All public headers** (mirrors the above component structure) |
| `main/` | Main DB API: `Database`, `Connection`, `PreparedStatement`, etc. |
| `optimizer/` | Query optimizer passes |
| `parser/` | Cypher parser (ANTLR4-based) |
| `planner/` | Logical query planner |
| `processor/` | Physical query execution engine |
| `storage/` | Storage engine (buffer manager, tables, WAL, indexes, compression) |
| `transaction/` | Transaction management |
| `CMakeLists.txt` | Root CMake build file |

---

## 2. Each Directory — Subdirectories & Key Files

### `src/binder/`
```
bind/                  (binding logic for statements/clauses)
  read/                (bind_match, bind_load_from, bind_unwind, etc.)
  copy/                (bind_copy_from/to)
  ddl/                 (bind_create_table, bind_ddl, etc.)
bind_expression/       (bind_function_expression, bind_literal_expression, etc.)
ddl/                   (bound_alter, bound_create_table, etc.)
expression/            (expression classes)
query/                 (bound_regular_query, query_graph, reading/return/updating clauses)
rewriter/              (match_clause_pattern_label_rewriter, with_clause_projection_rewriter)
visitor/               (confidential_statement_analyzer, default_type_solver, property_collector)
binder.cpp, expression_binder.cpp, expression_visitor.cpp, bound_statement_result.cpp, etc.
```

### `src/catalog/`
```
catalog_entry/         (catalog_entry.h, node_table_catalog_entry.h, rel_group_catalog_entry.h,
                        table_catalog_entry.h, function_catalog_entry.h, index_catalog_entry.h,
                        sequence_catalog_entry.h, type_catalog_entry.h, etc.)
catalog.cpp, catalog_set.cpp, property_definition_collection.cpp
```

### `src/common/`
```
arrow/                 (arrow.h, arrow_converter.h, arrow_row_batch.h, etc.)
copier_config/         (csv_reader_config.h, file_scan_info.h)
data_chunk/            (data_chunk.h, data_chunk_collection.h, data_chunk_state.h, sel_vector.h)
enums/                 (21 enums: expression_type.h, join_type.h, rel_direction.h, table_type.h, etc.)
exception/             (19 exception types: binder, catalog, connection, storage, runtime, etc.)
file_system/           (file_system.h, local_file_system.h, gzip_file_system.h, virtual_file_system.h, compressed_file_system.h)
serializer/            (serializer.h, deserializer.h, buffered_file.h, buffer_writer/reader.h)
task_system/           (task.h, task_scheduler.h, progress_bar.h)
types/                 (types.h, ku_string.h, ku_list.h, date_t.h, timestamp_t.h, int128_t.h, uuid.h, etc.)
  value/               (value.h, node.h, rel.h, recursive_rel.h, nested.h)
vector/                (value_vector.h, auxiliary_buffer.h)
constants.h, type_utils.h, cast.h, null_mask.h, mask.h, etc.
```

### `src/function/`
```
aggregate/             (sum, avg, count, min_max, collect)
arithmetic/            (add, subtract, multiply, divide, modulo, negate, abs, rand)
array/                 (array_distance, array_cosine_similarity, array_inner_product, etc.)
cast/                  (cast_from_string, cast_array, cast_string_non_nested)
comparison/            (comparison_functions)
date/                  (date_functions)
export/                (export_csv_function, export_parquet_function)
gds/                   (rec_joins, bfs_graph, gds_frontier, ssp/asp/wsp paths, etc.)
  auxiliary_state/
hash/                  (hash_functions)
internal_id/
interval/
list/                  (list_append, list_concat, list_sort, list_filter, list_transform, etc.)
map/                   (map_creation, map_extract, map_keys, map_values)
null/                  (null_functions)
path/                  (nodes, rels, length, properties, semantic)
pattern/               (id_function, label_function, cost_function)
schema/                (offset_functions)
sequence/              (sequence_functions)
string/                (string_split, regex_replace, levenshtein, concat_ws, etc.)
struct/                (struct_extract, struct_pack, keys)
table/                 (table_function, show_tables, show_indexes, db_version, etc.)
timestamp/             (timestamp_functions, to_epoch_ms)
union/                 (union_extract, union_tag, union_value)
utility/               (coalesce, nullif, typeof, error, md5, sha256)
uuid/                  (gen_random_uuid)
```

### `src/expression_evaluator/`
```
case_evaluator.cpp, function_evaluator.cpp, lambda_evaluator.cpp,
literal_evaluator.cpp, pattern_evaluator.cpp, path_evaluator.cpp,
reference_evaluator.cpp, expression_evaluator.cpp, expression_evaluator_utils.cpp
```

### `src/extension/`
```
catalog_extension.cpp, extension.cpp, extension_entries.cpp,
extension_installer.cpp, extension_manager.cpp, loaded_extension.cpp
```

### `src/graph/`
```
graph.cpp, graph_entry.cpp, graph_entry_set.cpp, on_disk_graph.cpp, parsed_graph_entry.cpp
```

### `src/main/`
```
attached_database.cpp, client_context.cpp, connection.cpp, database.cpp,
database_manager.cpp, db_config.cpp, plan_printer.cpp, prepared_statement.cpp,
prepared_statement_manager.cpp, query_result/, settings.cpp, storage_driver.cpp, version.cpp
```

### `src/optimizer/`
```
acc_hash_join_optimizer.cpp, agg_key_dependency_optimizer.cpp,
filter_push_down_optimizer.cpp, projection_push_down_optimizer.cpp,
limit_push_down_optimizer.cpp, factorization_rewriter.cpp,
correlated_subquery_unnest_solver.cpp, top_k_optimizer.cpp,
remove_unnecessary_join_optimizer.cpp, remove_factorization_rewriter.cpp,
cardinality_updater.cpp, schema_populator.cpp, logical_operator_visitor/collector.cpp
```

### `src/parser/`
```
antlr_parser/          (kuzu_cypher_parser.cpp, parser_error_listener/strategy)
expression/            (parsed_expression, parsed_function_expression, parsed_case_expression, etc.)
transform/             (~17 transform_*.cpp files from ANTLR AST to parsed statements)
visitor/               (standalone_call_rewriter, statement_read_write_analyzer)
parser.cpp, transformer.cpp, create_macro.cpp, etc.
```

### `src/planner/`
```
join_order/            (cardinality_estimator, cost_model, join_tree_constructor, join_plan_solver, etc.)
operator/              (logical operators: logical_extend, logical_filter, logical_hash_join, etc.)
  ddl/                 (logical_create_table, logical_alter, logical_drop, etc.)
  extend/              (base_logical_extend, logical_extend, logical_recursive_extend)
  factorization/       (flatten_resolver, sink_util)
  persistent/          (logical_copy_from/to, logical_insert/delete/set/merge)
  scan/                (logical_dummy_scan, logical_expressions_scan, logical_index_look_up, logical_scan_node_table)
  simple/              (logical_attach/detach_database, logical_export/import_db, logical_extension)
  sip/                 (logical_semi_masker, side_way_info_passing)
plan/                  (~25 plan_append_*.cpp files building logical plans)
planner.cpp, query_planner.cpp, subplans_table.cpp
```

### `src/processor/`
```
map/                   (~40 map_*.cpp files mapping logical to physical operators)
operator/              (physical operators)
  aggregate/           (hash_aggregate, simple_aggregate, aggregate_hash_table)
  ddl/                 (alter, create_table, create_sequence, create_type, drop)
  hash_join/           (hash_join_build, hash_join_probe, join_hash_table)
  intersect/           (intersect, intersect_build)
  macro/               (create_macro)
  order_by/            (order_by, order_by_merge/scan/key_encoder, radix_sort, top_k, etc.)
  persistent/          (insert/delete/set/merge, batch_insert, copy_to/from)
    reader/            (csv/, parquet/, npy/ readers)
    writer/            (parquet/ writer)
  scan/                (scan_node_table, scan_rel_table, scan_multi_rel_tables, primary_key_scan)
  simple/              (attach/detach_database, export/import_db, install/load/uninstall_extension)
  table_scan/          (ftable_scan_function, union_all_scan)
result/                (factorized_table, result_set, flat_tuple, base_hash_table, etc.)
processor.cpp, plan_mapper.cpp, expression_mapper.cpp, etc.
```

### `src/storage/`
```
buffer_manager/        (buffer_manager, memory_manager, spiller, vm_region, mm_allocator)
compression/           (compression, float_compression, bitpacking_utils)
index/                 (hash_index, in_mem_hash_index)
local_storage/         (local_storage, local_node_table, local_rel_table, local_hash_index)
predicate/             (column_predicate, constant_predicate, null_predicate)
stats/                 (column_stats, table_stats, hyperloglog)
table/                 (node_table, rel_table, column types, node_group, CSR structures, etc.)
wal/                   (wal, wal_record, wal_replayer, local_wal, checksum_reader/writer)
storage_manager.cpp, checkpointer.cpp, file_handle.cpp, disk_array.cpp, shadow_file.cpp, etc.
```

### `src/transaction/`
```
transaction.cpp, transaction_context.cpp, transaction_manager.cpp
```

### `src/c_api/`
```
connection.cpp, database.cpp, data_type.cpp, flat_tuple.cpp, helpers.cpp,
prepared_statement.cpp, query_result.cpp, query_summary.cpp, value.cpp, version.cpp
```

---

## 3. `src/include/` — Public Header Structure

The `src/include/` directory mirrors all components above with 15 subdirectories matching the source layout:

```
src/include/
├── binder/        (~35 headers)
├── catalog/       (~14 headers, including catalog_entry/ subfolder)
├── c_api/         (kuzu.h, helpers.h)
├── common/        (~85 headers across enums/, types/, data_chunk/, file_system/, serializer/, task_system/, arrow/, etc.)
├── expression_evaluator/  (~11 headers)
├── extension/     (~14 headers)
├── function/      (~80 headers across arithmetic/, cast/, comparison/, gds/, list/, map/, string/, table/, etc.)
├── graph/         (5 headers)
├── main/          (~18 headers, including query_result/)
├── optimizer/     (~15 headers)
├── parser/        (~45 headers across antlr_parser/, expression/, ddl/, query/, visitor/, etc.)
├── planner/       (~55 headers across operator/ with ddl/, extend/, factorization/, persistent/, scan/, simple/, sip/)
├── processor/     (~85 headers across operator/ with aggregate/, ddl/, hash_join/, order_by/, persistent/, scan/, simple/)
├── storage/       (~75 headers across buffer_manager/, compression/, index/, local_storage/, predicate/, stats/, table/, wal/)
└── transaction/   (4 headers)
```

---

## 4. Storage Engine Components (`src/include/storage/`)

| Subdir | Key Headers |
|--------|-------------|
| `buffer_manager/` | `buffer_manager.h`, `memory_manager.h`, `mm_allocator.h`, `spiller.h`, `vm_region.h` |
| `compression/` | `compression.h`, `float_compression.h`, `bitpacking_utils.h`, `bitpacking_int128.h` |
| `index/` | `hash_index.h`, `in_mem_hash_index.h`, `index.h` |
| `local_storage/` | `local_storage.h`, `local_node_table.h`, `local_rel_table.h`, `local_hash_index.h` |
| `predicate/` | `column_predicate.h`, `constant_predicate.h`, `null_predicate.h` |
| `stats/` | `column_stats.h`, `table_stats.h`, `hyperloglog.h` |
| `table/` | `table.h`, `node_table.h`, `rel_table.h`, `rel_table_data.h`, `column.h`, `column_chunk*.h`, `node_group.h`, `csr_node_group.h`, `list_column.h`, `struct_column.h`, `string_column.h`, `dictionary_column.h`, `version_info.h` |
| `wal/` | `wal.h`, `wal_record.h`, `wal_replayer.h`, `local_wal.h`, `checksum_reader/writer.h` |
| *(root)* | `storage_manager.h`, `checkpointer.h`, `file_handle.h`, `disk_array.h`, `shadow_file.h`, `undo_buffer.h`, `free_space_manager.h`, `database_header.h` |

---

## 5. Processor / Execution Components (`src/include/processor/`)

| Subdir | Key Headers |
|--------|-------------|
| `operator/aggregate/` | `hash_aggregate.h`, `simple_aggregate.h`, `aggregate_hash_table.h` |
| `operator/ddl/` | `alter.h`, `create_table.h`, `drop.h` |
| `operator/hash_join/` | `hash_join_build.h`, `hash_join_probe.h`, `join_hash_table.h` |
| `operator/intersect/` | `intersect.h`, `intersect_build.h` |
| `operator/order_by/` | `order_by.h`, `radix_sort.h`, `top_k.h`, `key_block_merger.h` |
| `operator/persistent/` | `insert.h`, `delete.h`, `set.h`, `merge.h`, `batch_insert.h`, `copy_to.h` |
| `operator/persistent/reader/` | `csv/`, `parquet/`, `npy/` readers |
| `operator/persistent/writer/` | `parquet/` writer |
| `operator/scan/` | `scan_node_table.h`, `scan_rel_table.h`, `scan_multi_rel_tables.h` |
| `operator/simple/` | `attach_database.h`, `export_db.h`, `import_db.h`, `install_extension.h` |
| `operator/table_scan/` | `ftable_scan_function.h`, `union_all_scan.h` |
| `operator/` (root) | `physical_operator.h`, `filter.h`, `flatten.h`, `projection.h`, `limit.h`, `skip.h`, `sink.h`, `cross_product.h`, `unwind.h`, `recursive_extend.h`, `semi_masker.h`, `index_lookup.h`, `partitioner.h`, `path_property_probe.h` |
| `result/` | `factorized_table.h`, `result_set.h`, `base_hash_table.h`, `flat_tuple.h` |
| *(root)* | `processor.h`, `plan_mapper.h`, `physical_plan.h`, `expression_mapper.h`, `execution_context.h` |

---

## 6. Planner / Optimizer / Binder Components

### Planner (`src/include/planner/`)
- **Core**: `planner.h`, `subplans_table.h`, `join_order_enumerator_context.h`
- **Operators** (30+ logical operators):
  - `logical_extend.h`, `logical_filter.h`, `logical_hash_join.h`, `logical_intersect.h`
  - `logical_scan_node_table.h`, `logical_index_look_up.h`
  - `logical_aggregate.h`, `logical_order_by.h`, `logical_limit.h`, `logical_distinct.h`
  - `logical_copy_from/to.h`, `logical_insert/delete/set/merge.h`
  - `logical_create_table/alter/drop.h`, `logical_attach_database.h`
  - `logical_union.h`, `logical_unwind.h`, `logical_projection.h`, `logical_flatten.h`
- **Join Order**: `cardinality_estimator.h`, `cost_model.h`, `join_plan_solver.h`, `join_tree.h`

### Optimizer (`src/include/optimizer/`)
- 15 optimizer passes: `filter_push_down_optimizer.h`, `projection_push_down_optimizer.h`, `limit_push_down_optimizer.h`, `acc_hash_join_optimizer.h`, `factorization_rewriter.h`, `correlated_subquery_unnest_solver.h`, `top_k_optimizer.h`, `remove_unnecessary_join_optimizer.h`, `agg_key_dependency_optimizer.h`, etc.

### Binder (`src/include/binder/`)
- `binder.h`, `binder_scope.h`, `expression_binder.h`, `expression_visitor.h`
- **DDL**: `bound_alter.h`, `bound_create_table.h`, `bound_drop.h`, `bound_create_sequence.h`, `bound_create_type.h`
- **Copy**: `bound_copy_from.h`, `bound_copy_to.h`
- **Expressions** (14 types): `expression.h`, `node_expression.h`, `rel_expression.h`, `property_expression.h`, `literal_expression.h`, `aggregate_function_expression.h`, `case_expression.h`, `lambda_expression.h`, `subquery_expression.h`, etc.
- **Query**: `bound_regular_query.h`, `query_graph.h`, reading/return/updating clauses
- **Rewriters**: 3 rewriters (match_clause_pattern_label, normalized_query_part_match, with_clause_projection)
- **Visitors**: 3 visitors (confidential_statement_analyzer, default_type_solver, property_collector)

---

## 7. Catalog Components (`src/include/catalog/`)

| File | Purpose |
|------|---------|
| `catalog.h` | Main `Catalog` class |
| `catalog_set.h` | `CatalogSet` — manages collections of entries |
| `property_definition_collection.h` | Property definitions |
| **`catalog_entry/`** | |
| `catalog_entry.h` | Base `CatalogEntry` class |
| `catalog_entry_type.h` | Enum of entry types |
| `node_table_catalog_entry.h` | Node table metadata |
| `rel_group_catalog_entry.h` | Relationship group metadata |
| `table_catalog_entry.h` | Base table entry |
| `function_catalog_entry.h` | Built-in function metadata |
| `index_catalog_entry.h` | Index metadata |
| `sequence_catalog_entry.h` | Sequence metadata |
| `type_catalog_entry.h` | User-defined type metadata |
| `scalar_macro_catalog_entry.h` | Macro metadata |
| `dummy_catalog_entry.h` | Placeholder entry |
| `node_table_id_pair.h` | Node table ID pair |

---

## 8. Common Types & Enums (`src/include/common/`)

### `common/enums/` (21 enums)
`accumulate_type.h`, `alter_type.h`, `clause_type.h`, `column_evaluate_type.h`, `conflict_action.h`, `delete_type.h`, `drop_type.h`, `explain_type.h`, `expression_type.h`, `extend_direction.h`, `join_type.h`, `path_semantic.h`, `query_rel_type.h`, `rel_direction.h`, `rel_multiplicity.h`, `scan_source_type.h`, `statement_type.h`, `subquery_type.h`, `table_type.h`, `zone_map_check_result.h`

### `common/types/` (value types)
`types.h` (LogicalType), `ku_string.h`, `ku_list.h`, `date_t.h`, `dtime_t.h`, `timestamp_t.h`, `interval_t.h`, `int128_t.h`, `uint128_t.h`, `uuid.h`, `blob.h`, `internal_id_util.h`<br>
**`types/value/`**: `value.h`, `node.h`, `rel.h`, `recursive_rel.h`, `nested.h`

### Other key headers in `common/`
`constants.h`, `type_utils.h`, `cast.h`, `null_mask.h`, `mask.h`, `roaring_mask.h`, `utils.h`, `string_utils.h`, `concurrent_vector.h`, `case_insensitive_map.h`, `checksum.h`, `sha256.h`, `md5.h`, `random_engine.h`, `profiler.h`, `metric.h`, `counter.h`, `mutex.h`, `uniq_lock.h`, `mpsc_queue.h`, `finally_wrapper.h`, `array_utils.h`

---

## 9. Main Database API (`src/include/main/`)

| File | Purpose |
|------|---------|
| `database.h` | `Database` class — primary entry point |
| `connection.h` | `Connection` class — query execution |
| `client_context.h` | Per-client context |
| `client_config.h` | Client configuration |
| `database_manager.h` | Manages attached databases |
| `attached_database.h` | Attached (external) database |
| `db_config.h` | Database configuration |
| `kuzu.h` | Top-level convenience header |
| `kuzu_fwd.h` | Forward declarations |
| `prepared_statement.h` | Prepared statement |
| `prepared_statement_manager.h` | Prepared statement cache |
| `query_result.h` | `QueryResult` class |
| `query_result/` | `arrow_query_result.h`, `materialized_query_result.h` |
| `query_summary.h` | Query execution summary |
| `plan_printer.h` | Plan/tree printing |
| `settings.h` | Configuration settings |
| `storage_driver.h` | Direct storage access |
| `version.h` | Version info |

---

## 10. Approximate File Counts

| Scope | `.h` Files | `.cpp` Files |
|-------|-----------|-------------|
| **Core `src/` (all subdirs)** | **~540** | **~460** |
| Extensions (`extension/`) | ~150 | ~125 |
| Ladybug fork (`ladybug/`) | ~400 | ~350 |
| Third-party (`third_party/`) | ~1,100 | ~950 |
| Tools (`tools/`) | ~35 | ~30 |
| **Workspace total** | **~2,225** | **~2,065** |

The **core Kuzu engine** (`src/` only) contains approximately **~540 header files** and **~460 implementation files**, totaling **~1,000 C++ source files**.

---

I now have enough data to provide a comprehensive report. Here it is:

---

## 🗺️ Kuzu Rust Codebase — Complete Structural Analysis

### 1. `kuzu-core/Cargo.toml` — Workspace Root

- **Workspace with 28 member crates** (resolver = "2", edition = "2024")
- **Version:** 0.1.0
- **License:** MIT
- **Description:** "Kùzu graph database — pure Rust embedded graph database"
- **Workspace dependencies:** `serde`, `serde_json`, `tracing`, `thiserror`, `rayon`, `regex`, `hashbrown`, `bitflags`, `bytes`, `criterion`, `arrow` (53), `parquet` (53), `csv`, `uuid`, `ureq`, `time`, `rust_decimal`, `fast-float`

---

### 2. ALL 28 Crates Under `kuzu-core/`

| # | Crate | # .rs Files | Purpose |
|---|-------|------------|---------|
| 1 | `kuzu-common` | **9** | Core type system, ValueVector, DataChunk, serialization, memory mgmt, file system, task system |
| 2 | `kuzu-storage` | **24** | Storage engine — buffer manager, WAL, columnar tables, compression, ART index, hash index, CSV/Parquet readers, spiller, checkpoint, shadow file |
| 3 | `kuzu-transaction` | **1** | MVCC transaction manager — serializable ACID, concurrent multi-writer, timestamp ordering |
| 4 | `kuzu-catalog` | **1** | System catalog — NodeTableEntry, RelTableEntry, SequenceEntry, ForeignTableEntry, VectorIndexEntry, CatalogColumn |
| 5 | `kuzu-parser` | **3** | Cypher PEG parser (pest.rs) + AST definitions + grammar file (cypher.pest) |
| 6 | `kuzu-binder` | **3** | Semantic analysis — symbol resolution, catalog lookup, type checking, bound AST nodes |
| 7 | `kuzu-planner` | **4** | Logical query planning — logical operators, join tree construction, plan enumeration |
| 8 | `kuzu-optimizer` | **4** | 13 optimizer passes (11 flat + 2 tree): filter push-down, projection push-down, constant folding, join optimization, top-k, vector similarity detection, ART range scan detection, factorization, cardinality estimation |
| 9 | `kuzu-processor` | **4** | Physical operator execution — 18+ physical operators, pipeline execution, expression evaluator |
| 10 | `kuzu-function` | **3** | Function registry — 100+ built-in scalar/aggregate/table functions, arithmetic/comparison/string/date/list/map/struct/boolean ops |
| 11 | `kuzu-graph` | **3** | CSR adjacency, Graph/Edge types, BFS, PageRank, WCC, shortest path, degree centrality |
| 12 | `kuzu-extension` | **3** | Extension framework — Extension trait, ExtensionRegistry, ExtensionContext |
| 13 | `kuzu-main` | **5** | Public API — Database, Connection, QueryResult, PreparedStatement |
| 14 | `kuzu-cli` | **1** | Interactive CLI shell (rustyline), `.mode`/`.import`/`.export`/`.tables` commands |
| 15 | `kuzu-algo` | **1** | Graph algorithms extension — PageRank, WCC, SCC (Tarjan + Kosaraju), K-Core, Louvain, Spanning Forest, shortest path |
| 16 | `kuzu-json` | **1** | JSON extension — json_extract, json_array_length, json_valid, json_contains, json_keys, to_json, json_array, json_object, json_merge_patch |
| 17 | `kuzu-fts` | **1** | Full-Text Search — stem, tokenize, create_fts_index, query_fts_index, Porter stemmer |
| 18 | `kuzu-vector` | **2** | Vector/HNSW extension — cosine_similarity, euclidean_distance, dot_product, HNSW ANN index (Cosine/Euclidean/L1/L2/Dot) |
| 19 | `kuzu-httpfs` | **1** | HTTP File System — http_get, http_scan, https_scan, URL parsing |
| 20 | `kuzu-duckdb` | **5** | DuckDB integration — duckdb_query, duckdb_scan, type/result converters, attach_helper |
| 21 | `kuzu-neo4j` | **1** | Neo4j migration — parse Neo4j Cypher dumps, migrate schema+data to Kuzu |
| 22 | `kuzu-llm` | **1** | LLM embedding — create_embedding, OpenAI + Ollama providers |
| 23 | `kuzu-sqlite` | **1** | SQLite integration (rusqlite) |
| 24 | `kuzu-delta` | **1** | Delta Lake integration (via DuckDB delegation) |
| 25 | `kuzu-iceberg` | **1** | Apache Iceberg integration (via DuckDB delegation) |
| 26 | `kuzu-azure` | **1** | Azure integration (via DuckDB delegation) |
| 27 | `kuzu-postgres` | **1** | PostgreSQL integration (tokio-postgres) |
| 28 | `kuzu-unity-catalog` | **1** | Unity Catalog integration (via DuckDB delegation) |

**Total .rs files (core crates):** ~85
**Total .rs files (with tests/benches):** ~94

---

### 3. Detailed Crate-by-Crate Analysis

#### 🔷 `kuzu-common` (9 files) — Foundation Layer
- **Dependencies:** serde, tracing, thiserror, bitflags, bytes, uuid, time, rust_decimal, fast-float, hashbrown, rayon
- **Modules:**
  - `types.rs` — 35+ `LogicalTypeID` variants (Node, Rel, Bool, Int8–128, UInt8–64, Float/Double, Date/Timestamp(s/ms/ns/tz), Interval, Decimal, String, Blob, List, Array, Struct, Map, Union, UUID, Serial, InternalID), `PhysicalTypeID`, `Value` enum, `InternalID`, `Date`, `Timestamp`, `TimestampTZ`, `Interval`
  - `vector.rs` — `ValueVector` (typed columnar array with null mask), `DataChunk`, `physical_type_size()`
  - `data_chunk.rs` — `DataChunk` struct (collection of ValueVectors)
  - `enums.rs` — `CompressionType`, `TransactionAction`, `PathSemantic`, `ExtendDirection`
  - `serialization.rs` — Serialization primitives
  - `memory.rs` — `MemoryManager`
  - `file_system.rs` — File system abstraction
  - `task_system.rs` — Thread pool / task system (using rayon)
  - `lib.rs` — Module declarations + re-exports

#### 🔷 `kuzu-storage` (24 files) — Storage Engine
- **Dependencies:** kuzu-common, kuzu-catalog, kuzu-transaction, kuzu-vector, arrow/parquet (optional), csv, dashmap
- **Modules:**
  - `buffer_manager.rs` — `BufferManager`, `BufferManagerConfig`
  - `wal.rs` — Write-Ahead Log
  - `local_wal.rs` — Per-transaction WAL buffer
  - `local_storage.rs` — Per-transaction local storage
  - `table.rs` — `NodeTable`, `RelTable`, `ColumnDefinition`, `TableCatalog` (DashMap-based)
  - `column.rs` — Column storage
  - `column_chunk.rs` — `ColumnChunk`, `NODE_GROUP_SIZE`
  - `node_group.rs` — `NodeGroup` (in-memory row group)
  - `compression.rs` — Column compression algorithms
  - `index.rs` — `HashIndex`, `OnDiskHashIndex`, `IndexKey`
  - `art_index.rs` — `ArtPrimaryKeyIndex` (Adaptive Radix Tree)
  - `art_key.rs` — `ArtKey`
  - `art_node.rs` — ART node types
  - `vector_index.rs` — `VectorIndexTable`
  - `checkpoint.rs` — Checkpoint logic
  - `shadow_file.rs` — Shadow file for atomic commits
  - `spiller.rs` — `Spiller`, `SpillFile`, `MultiWayStreamMerge` (disk spilling)
  - `stats.rs` — `StatsStore`
  - `update_info.rs` — Update tracking
  - `version_info.rs` — Version tracking
  - `page.rs` — Page management
  - `csv_reader.rs` — CSV file reader
  - `parquet_reader.rs` — Parquet file reader (feature-gated)
  - `StorageManager` struct — root of storage engine

#### 🔷 `kuzu-transaction` (1 file) — Transaction Management
- **Key types:** `Transaction`, `TransactionType` (ReadOnly/Write), `TransactionStatus`, `UndoRecord`, `CommitResult`, `TransactionManager`, `TransactionManagerConfig`
- **Features:** MVCC timestamp ordering, concurrent multi-writer support, single-writer mode, table-level locking, checkpoint drain, background auto-checkpoint

#### 🔷 `kuzu-catalog` (1 file) — System Catalog
- **Key types:** `Catalog` (HashMap-based), `CatalogColumn`, `NodeTableEntry`, `RelTableEntry`, `SequenceEntry`, `VectorIndexEntry`, `ForeignTableEntry`, `IndexType` (Hash/Art)
- Supports: `all_entries()`, `get_entry_by_name()`, `create_entry()`, `drop_entry()`, `create_sequence()`, `drop_sequence()`

#### 🔷 `kuzu-parser` (3 files + 1 grammar) — Cypher Parser
- **Grammar:** `cypher.pest` (PEG grammar using pest.rs, replacing ANTLR4 C++ parser)
- **AST:** Full Cypher AST — `Statement::Query | CreateNodeTable | CreateRelTable | DropTable | CopyFrom | AlterTable | CreateIndex | DropIndex | CreateVectorIndex | Union | Merge | Call | CreateDml | Explain | CreateSequence | DropSequence | ExportDatabase | ImportDatabase`
- **Clauses:** Match, Return, Where, Create, Delete, Set, OptionalMatch, With, Unwind, Foreach
- **Expressions:** Constant, Variable, Parameter (`$name`), PropertyAccess, FunctionCall, BinaryOp, UnaryOp, List, Map, ExistsSubquery
- **Edge patterns:** Supports `lower_bound`/`upper_bound` for variable-length paths (`[*1..5]`)

#### 🔷 `kuzu-binder` (3 files) — Binder
- `Binder` struct with catalog reference
- `bind()` dispatches to per-statement binders
- Resolves all statement types including CREATE/DROP SEQUENCE, EXPORT/IMPORT DATABASE, EXPLAIN, MERGE
- `BoundStatement` enum has 18 variants matching all parser Statement types
- Bound types include `BoundExplain`, `BoundCreateSequence`, `BoundDropSequence`, `BoundExportDatabase`, `BoundImportDatabase`

#### 🔷 `kuzu-planner` (4 files) — Query Planner
- `QueryPlanner` — converts `BoundStatement` → `Vec<LogicalOperator>`
- **LogicalOperator enum has 33 variants:**
  - Scan: `ScanNode`, `ScanRel`, `VectorSimilarityScan`, `ArtIndexRangeScan`
  - Relational: `Filter`, `Projection`, `HashJoin`, `CrossProduct`, `OrderBy`, `Limit`, `Aggregate`, `Union`, `Flatten`, `OptionalMatch`, `SemiJoin`, `AntiJoin`, `Intersect`, `Explain`, `RecursiveExtend`
  - Misc: `TableFunctionCall`, `CopyFrom`, `Delete`, `Set`, `Unwind`, `Foreach`, `Merge`
  - DDL: `CreateNodeTable`, `CreateRelTable`, `DropTable`, `AlterTable`, `CreateIndex`, `DropIndex`, `CreateVectorIndex`, `CreateSequence`, `DropSequence`, `CreateDml`, `ExportDatabase`, `ImportDatabase`
- `join_order.rs` — Join tree construction, `build_join_tree()`, `flatten_join_plan()`

#### 🔷 `kuzu-optimizer` (4 files) — Optimizer
- **13 optimizer passes** (11 flat + 2 tree):
  1. `RemoveUnnecessaryOperators`
  2. `FilterPushDown`
  3. `ProjectionPushDown`
  4. `ConstantFolding`
  5. `AggregateDetection`
  6. `JoinOptimization`
  7. `TopKOptimization`
  8. `VectorSimilarityDetection`
  9. `ArtRangeScanDetection`
  10. `LimitPushDown`
  11. `CommonSubexpressionElimination`
  12. `FactorizationRewriting` (tree pass)
  13. `CardinalityEstimation` (tree pass, optionally storage-backed)

#### 🔷 `kuzu-processor` (4 files + 5 benches) — Execution Engine
- **Physical operators implemented in `physical_operator.rs`:**
  - `PhysicalScan`, `PhysicalScanRel`, `PhysicalFilter`, `PhysicalProjection`
  - `PhysicalHashJoin`, `PhysicalCrossProduct`, `PhysicalOrderBy`
  - `PhysicalLimit`, `PhysicalAggregate`, `PhysicalUnion`, `PhysicalFlatten`
  - `PhysicalDelete`, `PhysicalSet`, `PhysicalUnwind`, `PhysicalTableFunction`
  - `PhysicalVectorSimilarityScan`, `PhysicalArtIndexRangeScan`, `PhysicalRecursiveExtend`
  - `PhysicalExplain` — serializes plan tree to text
- `expression_evaluator.rs` — Expression evaluation engine
- `processor.rs` — `QueryProcessor` with pipeline execution model
- **5 benchmarks:** `physical_scan`, `physical_filter`, `physical_hash_join`, `physical_order_by`, `physical_aggregate`

#### 🔷 `kuzu-function` (3 files) — Function Registry
- **ScalarFunction variants:**
  - `ArithmeticOp`: Add, Sub, Mul, Div, Mod, Abs, Ceil, Floor, Round, Negate, Power, Sqrt, Log, Exp, Sin, Cos, Tan, Asin, Acos, Atan, Atan2, Degrees, Radians, Sign, Pi, Rand — **26 ops**
  - `ComparisonOp`: Eq, NotEq, Lt, Lte, Gt, Gte, IsNull, IsNotNull, Between — **9 ops**
  - `StringOp`: Concat, Contains, StartsWith, EndsWith, RegexMatches, Substring, Upper, Lower, Trim, Ltrim, Rtrim, Len, Replace, Reverse — **14 ops**
  - `BooleanOp`: And, Or, Not, Xor — **4 ops**
  - `DateOp`: DatePart, DateTrunc, DateDiff, DateAdd — **4 ops**
  - `ListOp`: ListCreation, ListExtract, ListConcat, ListAppend, ListPrepend, ListPosition, ListContains, ListSlice, ListLen, ListSort, ListReverse, ListDistinct, ListUnion, ListIntersection, ListAnyValue — **15 ops**
  - `MapOp`: MapCreation, MapExtract, MapKeys, MapValues, MapContainsKey, MapContainsValue, MapEntries, MapFromLists — **8 ops**
  - `StructOp`: StructCreation, StructExtract, StructKeys — **3 ops**
  - `SchemaOp`: Offset, Id, StartNode, EndNode, Label — **5 ops**
  - `UtilityOp`: Coalesce, NullIf, IfNull, Typed, Case, Cast, ListRange, CurrentDate, CurrentTimestamp — **9 ops**
  - `ArrayOp`: ArrayCosineSimilarity, ArrayInnerProduct, ArrayDistance, ArrayCat, ArrayContains, ArrayPosition, ArraySlice, ArraySort, ArrayReverse, ArrayDistinct, ArrayCrossProduct, ArrayUnion, ArrayIntersection — **13 ops**
  - `AggregateFunction`: Count, Sum, Avg, Min, Max, CountStar, Collect, First — **8 ops**
  - `TableFunction`: ScanJson, Custom, TableFunction callback — **3 variants**
  - Plus `CustomScalar` closures for extension-provided functions

#### 🔷 `kuzu-graph` (3 files) — Graph Engine
- `CSRAdjacency` — Compressed Sparse Row adjacency format
- `Graph` / `Edge` / `GraphEntry` / `OnDiskGraph`
- **Algorithms implemented:** `bfs`, `page_rank`, `weakly_connected_components`, `shortest_path`, `degree_centrality`, `reachable_within`
- `AlgorithmResult` — generic node-level result container

#### 🔷 `kuzu-main` (5 files) — Public API
- `Database` — main entry point, owns all subsystems
- `Connection` — full query lifecycle: parse → bind → plan → optimize → execute
- `QueryResult` — result container
- `PreparedStatement` — parameterized query support
- `SystemConfig` — `buffer_pool_size`, `max_num_threads`, `enable_compression`, `read_only`, `auto_checkpoint`, `concurrent_writes`, `spill_threshold`
- **Feature flags** for all 13 extensions (json, fts, vector, httpfs, duckdb, algo, neo4j, llm, sqlite, delta, iceberg, azure, postgres, unity-catalog)
- **2 benchmarks:** `query_pipeline`, `storage_bench`
- **2 test files:** `integration_test.rs`, `fase_b_verification.rs`

#### 🔷 `kuzu-cli` (1 file) — CLI Shell
- Interactive REPL with rustyline
- Commands: `.mode` (table/csv/json/line/column), `.import`, `.export`, `.tables`, `.schema`, `.help`
- Multi-line input, command history, tab completion

#### 🔷 Extension Crates (15 total)
All follow the same pattern — implement `Extension` trait, register scalar + table functions in `load()`:
- **`kuzu-algo`** — 7 graph algorithms
- **`kuzu-json`** — 11+ JSON functions
- **`kuzu-fts`** — 4 FTS functions + Porter stemmer
- **`kuzu-vector`** — HNSW ANN index, 4 distance metrics
- **`kuzu-httpfs`** — HTTP file access
- **`kuzu-duckdb`** — DuckDB query execution (4 sub-modules)
- **`kuzu-neo4j`** — Neo4j Cypher dump migrator
- **`kuzu-llm`** — OpenAI/Ollama embedding generation
- **`kuzu-sqlite`** — SQLite integration
- **`kuzu-delta`** / **`kuzu-iceberg`** / **`kuzu-azure`** / **`kuzu-unity-catalog`** — via DuckDB delegation
- **`kuzu-postgres`** — via tokio-postgres

---

### 4. Consolidated Plan Document

**File:** `kuzu-core/CONSOLIDATED_PLAN.md` exists and contains:

#### Codebase Health
| Metric | Result |
|--------|--------|
| Compile errors | ✅ 0 errors |
| Compile warnings | ⚠️ 1 (`unused variable`) |
| Test pass | ✅ **691 tests, 0 failures** |
| Clippy errors | ✅ 0 errors |
| Clippy warnings | ⚠️ **128 warnings** |
| Logical operators | ✅ 23 variants |
| Physical operators | ✅ 18 executors |
| Optimizer passes | ✅ 13 (11 flat + 2 tree) |
| Built-in functions | ✅ 100+ (78 scalar + 9 aggregate + table) |

#### Gap Analysis — 13 Items Remaining
- **P1:** RecursiveExtend exec (missing physical operator), Shortest Path (missing 8 GDS algorithms) — ~7 days
- **P2:** SEQUENCE/SERIAL (entry exists but no nextval/currval functions), Schema functions (OFFSET/ID/START_NODE/END_NODE/LABEL), EXPLAIN statement (AST/plan done, execution pending), IMPORT/EXPORT DATABASE — ~5-6 days
- **P3:** Intersect operator, SIP (Semi Masks), Array math functions, 128 clippy warnings — ~7 days
- **P4:** WASM setup, Catalog entry types (ForeignTableEntry exists!), DDL operator refactor — ~3 days

**Total remaining: ~16-22 working days**

---

### 5. Dependency Graph (Key Crates)

```
kuzu-common  (foundation — no internal deps)
   ├─ kuzu-transaction  (→ kuzu-common)
   ├─ kuzu-catalog      (→ kuzu-common, kuzu-transaction)
   ├─ kuzu-parser       (→ kuzu-common, pest)
   ├─ kuzu-function     (→ kuzu-common)
   ├─ kuzu-binder       (→ kuzu-common, kuzu-parser, kuzu-catalog)
   ├─ kuzu-planner      (→ kuzu-common, kuzu-binder, kuzu-parser, kuzu-catalog)
   ├─ kuzu-optimizer    (→ kuzu-common, kuzu-planner, kuzu-storage, kuzu-binder)
   ├─ kuzu-storage      (→ kuzu-common, kuzu-catalog, kuzu-transaction, kuzu-vector)
   ├─ kuzu-processor    (→ kuzu-common, kuzu-catalog, kuzu-planner, kuzu-optimizer, kuzu-function, kuzu-storage)
   ├─ kuzu-graph        (→ kuzu-common, kuzu-storage)
   ├─ kuzu-extension    (→ kuzu-common, kuzu-function, kuzu-catalog)
   └─ kuzu-main         (→ ALL core crates + all extension crates as optional deps)
        └─ kuzu-cli     (→ kuzu-main, kuzu-common, kuzu-parser, kuzu-catalog)

Extension crates → kuzu-common, kuzu-function, kuzu-catalog, kuzu-extension
```

---

### Summary

This is a **highly sophisticated pure-Rust embedded graph database** with a full query engine supporting Cypher, MVCC transactions, columnar storage, ART + HNSW indexes, 100+ built-in functions, 13 optimizer passes, and 15 extension crates. The codebase is **production-ready** with **691 passing tests**, **0 compile errors**, and a well-organized modular architecture following the classic database pipeline: **Parser → Binder → Planner → Optimizer → Processor**.

---

Here is the complete and thorough analysis of the **LadybugDB** directory at `ladybug/`.

---

## 1. Complete Directory Structure (Top-Level)

```
ladybug/
├── .clang-format
├── .clang-format-ignore
├── .clang-tidy
├── .clang-tidy-analyzer
├── .github/
├── .gitignore
├── .gitmodules
├── .lcovrc
├── AGENTS.md
├── CMakeLists.txt           # Root CMake (v0.18.0)
├── CONTRIBUTING.md
├── LICENSE                  # MIT License
├── Makefile
├── README.md
├── SECURITY.md
├── benchmark/
├── cmake/                   # CMake templates
├── dataset/                 # Test datasets (tinysnb, ldbc-sf01, etc.)
├── docs/                    # Documentation & incident reports
├── examples/                # Usage examples
├── extension/               # Loadable extensions
│   ├── adbc/
│   ├── algo/                # Graph algorithms
│   ├── azure/
│   ├── delta/
│   ├── duckdb/
│   ├── fts/                 # Full-text search
│   ├── httpfs/              # HTTP/HTTPS file system
│   ├── iceberg/
│   ├── json/
│   ├── llm/
│   ├── neo4j/
│   ├── postgres/
│   ├── sqlite/
│   ├── unity_catalog/
│   └── vector/              # Vector (HNSW) index
├── logo/
├── pixi.toml
├── scripts/                 # Build/packaging scripts
├── security/
├── src/                     # C++ Core source
│   ├── antlr4/              # ANTLR parser
│   ├── binder/              # Statement binding
│   ├── c_api/               # C API
│   ├── catalog/             # Catalog management
│   ├── common/              # Shared utilities & types
│   ├── expression_evaluator/
│   ├── extension/           # Extension framework
│   ├── function/            # Built-in functions
│   ├── graph/               # Graph data structures
│   ├── include/             # All header files
│   ├── main/                # Database, Connection, entry points
│   ├── optimizer/           # Query optimizer
│   ├── parser/              # Cypher parser
│   ├── planner/             # Query planner
│   ├── processor/           # Query execution engine
│   ├── storage/             # Storage engine
│   └── transaction/         # Transaction manager
├── test/                    # C++ tests
├── third_party/             # Third-party dependencies
└── tools/                   # Language bindings & CLI tools
    ├── benchmark/
    ├── dev/
    ├── java_api/
    ├── nodejs_api/
    ├── python_api/
    ├── rust_api/             # Rust crate (`lbug`)
    ├── shell/                # CLI shell
    ├── wal_dump/
    └── wasm/                 # WebAssembly bindings
```

**Total: ~6,547 files**

---

## 2. Cargo.toml Files

### `tools/rust_api/Cargo.toml` — The **`lbug`** crate

| Field | Value |
|---|---|
| **Name** | `lbug` |
| **Version** | `0.17.0` |
| **Edition** | 2021 |
| **MSRV** | Rust 1.81 |
| **Description** | "An in-process property graph database management system built for query speed and scalability" |
| **Repository** | `https://github.com/LadybugDB/ladybug-rust` |
| **Homepage** | `https://ladybugdb.com/` |
| **License** | MIT |

**Dependencies:**
| Dependency | Version | Notes |
|---|---|---|
| `arrow` | 55 | Optional, FFI feature |
| `cxx` | =1.0.138 | C++/Rust interop (pinned to last version compatible with Clang 15) |
| `rust_decimal` | 1.37 | Decimal type support |
| `serde_json` | 1 | JSON support |
| `time` | 0.3 | Date/time types |
| `uuid` | 1.6 | UUID type |

**Build Dependencies:** `cmake` 0.1, `cxx-build` =1.0.138, `rustversion` 1

**Features:** `default = []`, `arrow`, `extension_tests`

### `tools/rust_api/examples/Cargo.toml`

Simple example crate (`lbug-rust-example v0.1.0`) depends on `lbug` via path and optionally `arrow`.

---

## 3. README & Documentation

**`ladybug/README.md`** — The primary documentation. Key excerpt:

> *"Ladybug is an embedded graph database built for query speed and scalability. Ladybug is optimized for handling complex analytical workloads on very large databases and provides a set of retrieval features, such as a full text search and vector indices."*

> *"The database was formerly known as [Kuzu](https://github.com/kuzudb/kuzu)."*

**Installation across languages:**
| Language | Package |
|---|---|
| Python | `pip install ladybug` |
| NodeJS | `npm install @ladybugdb/core` |
| Rust | `cargo add lbug` |
| Go | `go get github.com/LadybugDB/go-ladybug` |
| Swift, Java, C/C++, CLI | Separate packages/binaries |

---

## 4. Rust Source Files (`tools/rust_api/src/`)

| File | Purpose |
|---|---|
| `lib.rs` | **Crate entry point.** Re-exports `Database`, `SystemConfig`, `Connection`, `PreparedStatement`, `QueryResult`, `Value`, `NodeVal`, `RelVal`, `LogicalType`, `Error`. Exposes `VERSION`, `LBUG_LIBRARY_SOURCE`, `LBUG_LIBRARY_DIR` constants. |
| `database.rs` | **`Database` struct** — wraps C++ `Database` via `cxx::UniquePtr`. `SystemConfig` builder with: `buffer_pool_size`, `max_num_threads`, `enable_compression`, `read_only`, `max_db_size`, `auto_checkpoint`, `checkpoint_threshold`, `throw_on_wal_replay_failure`, `enable_checksums`, `enable_multi_writes`. Supports `:memory:` databases. |
| `connection.rs` | **`Connection` struct** — wraps C++ `Connection`. Methods: `query()`, `prepare()`, `execute()`, `set_max_num_threads_for_exec()`. `PreparedStatement` struct. Thread-safe (C++ mutex). Supports Arrow export via `query_as_arrow()`. Concurrency: read queries parallel, write queries serial. |
| `query_result.rs` | **`QueryResult` struct** — iteration over results. Methods: `get_compiling_time()`, `get_execution_time()`, `get_num_columns()`, `get_num_tuples()`, `get_column_names()`, `get_column_data_types()`. `CSVOptions` builder. Arrow feature: `iter_arrow()` (RecordBatch iterator), `csr()` (native CSR arrays for rel data). |
| `value.rs` | **`Value` enum** — all supported types: `Bool`, `Int(8/16/32/64)`, `UInt(8/16/32/64)`, `Int128`, `Double`, `Float`, `Date`, `Timestamp*`, `Interval`, `InternalID`, `String`, `Blob`, `List`, `Array`, `Struct`, `Map`, `Union`, `Node`, `Rel`, `UUID`, `Json`, `Decimal`, `Null`. Also `NodeVal` and `RelVal` graph types. |
| `logical_type.rs` | **`LogicalType` enum** — mirrors all Value types plus `Serial`, `RecursiveRel`. |
| `error.rs` | **`Error` enum** — `CxxException`, `FailedQuery`, `FailedPreparedStatement`, `ReadOnlyType`, `JsonError`, `UnsupportedType`, `ArrowError`. |
| `ffi.rs` | C++ FFI bridge via `cxx`. Declares all C++ types and functions used. `StringView` wrapper. Defines `LogicalTypeID` and `PhysicalTypeID` enums matching C++ side. |
| `lbug_rs.cpp` | C++ glue code — bridges Rust calls to Kuzu C++ API. |
| `lbug_arrow.cpp` | C++ Arrow FFI implementation. |

---

## 5. What Is LadybugDB's Purpose?

**LadybugDB is an embedded, in-process property graph database management system (GDBMS)** optimized for analytical query workloads on very large graphs. Key differentiators:

- **Embedded** — links directly into your application as a library (no server process)
- **Cypher-based** — uses the Cypher query language (via ANTLR parser)
- **Analytics-focused** — optimized for complex multi-hop graph queries, not simple lookups
- **Columnar storage** — disk-based columnar layout with compression
- **Full-text + Vector search** — built-in FTS and HNSW vector index

---

## 6. Components/Modules (C++ Core — `src/`)

| Module | `src/` directory | Purpose |
|---|---|---|
| **ANTLR Parser** | `antlr4/` | Cypher grammar & parser generated from ANTLR |
| **Parser** | `parser/` | Parses Cypher statements into AST |
| **Binder** | `binder/` | Binds parsed statements to catalog objects, resolves schemas |
| **Catalog** | `catalog/` | Schema catalog — node/rel table schemas, property definitions |
| **Common** | `common/` | Shared types, utilities, data chunks, vectors, file system, serialization, task system |
| **Expression Evaluator** | `expression_evaluator/` | Evaluates expressions over data chunks |
| **Function** | `function/` | Built-in scalar/aggregate/table functions (arithmetic, string, date, cast, etc.) |
| **Graph** | `graph/` | Graph data structures — `Graph`, `OnDiskGraph`, `GraphEntry` |
| **Main** | `main/` | Entry points: `Database`, `Connection`, `ClientContext`, `PreparedStatement`, `QueryResult`, `StorageDriver` |
| **Optimizer** | `optimizer/` | 16+ optimization passes (filter pushdown, projection pushdown, join order, factorization rewriting, limit pushdown, etc.) |
| **Planner** | `planner/` | Logical plan generation, join order enumeration, subplans |
| **Processor** | `processor/` | Physical query execution engine (vectorized operators, task scheduler) |
| **Storage** | `storage/` | Full storage stack (see below) |
| **Transaction** | `transaction/` | Serializable ACID transactions (MVCC-based) |
| **Extension** | `extension/` | Extension loading framework |
| **C API** | `c_api/` | C language bindings |

---

## 7. Storage Engine

From `src/storage/`:

| Component | Files | Purpose |
|---|---|---|
| **Buffer Manager** | `buffer_manager/` | Page-level buffer pool with eviction, disk I/O |
| **Compression** | `compression/` | Per-column-type compression (run-length, dictionary, etc.) |
| **Index** | `index/` | Hash index for primary keys |
| **Local Storage** | `local_storage/` | Transaction-local uncommitted data |
| **WAL** | `wal/` | Write-Ahead Log for durability |
| **Shadow File** | `shadow_file.*` | Shadow paging for checkpointing |
| **Table Storage** | `table/` | Node/Rel table layouts (columnar, CSR) |
| **Stats** | `stats/` | Column statistics for cardinality estimation |
| **Predicate** | `predicate/` | Zone map / min-max predicate skipping |
| **Overflow File** | `overflow_file.*` | Variable-length data (strings, lists) overflow pages |
| **Disk Array** | `disk_array.*` | Contiguous disk-based arrays |
| **Free Space Manager** | `free_space_manager.*` | Free space tracking |
| **Checkpointer** | `checkpointer.*` | Checkpoint coordination |

**Storage architecture:**
- **Columnar disk-based storage** — each property column stored separately
- **CSR adjacency lists** — Compressed Sparse Row format for relationship storage (both forward and backward neighbors)
- **Compression** — per-column type-aware compression
- **Shadow paging** — no-overwrite storage with atomic checkpoint
- **WAL** — Write-Ahead Log for durability & crash recovery
- **Buffer pool** — manages in-memory pages, auto-sized based on system memory
- **MVCC** — Multi-Version Concurrency Control for serializable ACID transactions
- **Max DB size** — defaults to 8TB on 64-bit systems

---

## 8. Query Processing Capabilities

The query processor (`src/processor/`) is **vectorized and factorized**:

- **Vectorized execution** — processes data in column batches (vectors), not row-by-row
- **Factorized query processor** — reduces intermediate result sizes using factorization techniques
- **Physical operators** (`src/processor/operator/`): Scan, Filter, Join (hash, adjacency list), Aggregate, Order By, Limit, Union, Intersect, Projection, etc.
- **Multi-core parallelism** — task-based parallelism via `TaskSystem`
- **Novel join algorithms** — optimized for graph-pattern matching

**Optimizer passes** (16+ in `src/optimizer/`):
- Filter push-down, Projection push-down, Limit push-down
- Join order optimization (cardinality estimation)
- Factorization rewriting & removal
- Foreign join push-down
- Top-K optimization
- Correlated subquery unnesting
- Hash join optimization
- Aggregation key dependency optimization

---

## 9. Relation to Kuzu

**LadybugDB is a direct fork/rebranding of Kuzu.** From the README:

> *"The database was formerly known as [Kuzu](https://github.com/kuzudb/kuzu)."*

**Evidence:**
- The C++ namespaces use `lbug::` (e.g., `lbug::common::`, `lbug::storage::`), clearly a find-and-replace from `kuzu::`
- The internal APIs and architecture are identical to Kuzu
- Rust crate name changed from `kuzu` to `lbug`
- CMake project name changed from `Kuzu` to `Lbug` (version 0.18.0)
- All extensions and components maintain identical structure to Kuzu
- The same MIT license

**Key differences vs Kuzu:**
1. **Rebranded namespace/API** — `lbug::` instead of `kuzu::`
2. **Vector extension** — contains NaviX vector-index compatibility layer (HNSW with advanced search modes)
3. **LLM extension** — AI/LLM integration extension
4. **Various performance improvements and bug fixes** incorporated after the fork point
5. **WASM support** — browser-based execution via WebAssembly

---

## 10. Features Different from Kuzu Rust Crate

| Feature | Ladybug (`lbug`) | Kuzu |
|---|---|---|
| **Name** | `lbug` on crates.io | `kuzu` on crates.io |
| **Vector extension** | Full HNSW with NaviX compatibility | Basic or absent |
| **LLM extension** | LLM integration | Not present |
| **Arrow CSR support** | Native CSR arrays from Arrow results | Similar but branded differently |
| **Multi-write support** | `enable_multi_writes` config option | Possibly different |
| **Search modes** (vector) | `auto`, `navix`, `adaptive_l`, `adaptive_g`, `blind`, `directed`, `one_hop`, `naive`, `random` | Kuzu's version |

---

## 11. Graph Algorithms (`extension/algo/`)

**Available algorithms** (all callable via Cypher `CALL` syntax):

| Algorithm | Function Name | Alias |
|---|---|---|
| **PageRank** | `PAGE_RANK` | `PR` |
| **Louvain Community Detection** | `LOUVAIN` | — |
| **Weakly Connected Components** | `WEAKLY_CONNECTED_COMPONENTS` | `WCC` |
| **Strongly Connected Components** | `STRONGLY_CONNECTED_COMPONENTS` | `SCC` |
| **SCC (Kosaraju's algorithm)** | `STRONGLY_CONNECTED_COMPONENTS_KOSARAJU` | `SCC_KO` |
| **K-Core Decomposition** | `K_CORE_DECOMPOSITION` | `KCORE` |
| **Spanning Forest** | `SPANNING_FOREST` | `SF` |

All algorithms use the **GDS (Graph Data Science) framework**, which provides:
- `DenseFrontier` / `FrontierPair` — frontier-based graph traversal
- `EdgeCompute` — edge processing abstraction
- `GDSComputeState` — parallel computation state
- `GDSUtils` — algorithm execution utilities
- `GDSDenseObjectManager` — dense per-node object storage
- `GDSObjectManager` — sparse per-node object storage

---

## 12. Extensions Summary

| Extension | Purpose |
|---|---|
| **`algo`** | Graph algorithms (PageRank, Louvain, SCC/WCC, K-Core, Spanning Forest) |
| **`fts`** | Full-text search indexing and querying (inverted index, stemming, tokenizer) |
| **`vector`** | HNSW vector index with multiple search modes, NaviX compatibility |
| **`json`** | JSON data type and functions |
| **`duckdb`** | DuckDB integration (query DuckDB from Ladybug) |
| **`postgres`** | PostgreSQL foreign data wrapper |
| **`sqlite`** | SQLite foreign data wrapper |
| **`httpfs`** | HTTP/HTTPS file system for loading remote data |
| **`iceberg`** | Apache Iceberg table format support |
| **`delta`** | Delta Lake table format support |
| **`azure`** | Azure Blob Storage file system |
| **`unity_catalog`** | Unity Catalog integration |
| **`neo4j`** | Neo4j integration |
| **`llm`** | LLM/AI integration |
| **`adbc`** | Arrow Database Connectivity |

**Default static-linked extensions** (for WASM/Android/Swift builds): `fts`, `json`, `vector`, `algo`

---

## 13. Language Bindings (Tools)

| Language | Location | Package |
|---|---|---|
| **Rust** | `tools/rust_api/` | `lbug` on crates.io |
| **Python** | `tools/python_api/` | `ladybug` on PyPI |
| **Node.js** | `tools/nodejs_api/` | `@ladybugdb/core` on npm |
| **Java** | `tools/java_api/` | Maven Central |
| **WebAssembly** | `tools/wasm/` | Browser/Node.js WASM |
| **CLI Shell** | `tools/shell/` | Precompiled binary |

---

## 14. Main Entry Points

### C++ Entry Point
`src/main/database.cpp` / `src/include/main/database.h`:
- `Database` class — constructs with path + `SystemConfig`
- Internally initializes: `BufferManager`, `StorageManager`, `Catalog`, `TransactionManager`, `Processor`, `ExtensionManager`
- `Connection` class — created from `Database`, executes queries

### Rust Entry Point
`tools/rust_api/src/lib.rs`:
```rust
pub use connection::{Connection, PreparedStatement};
pub use database::{Database, SystemConfig};
pub use error::Error;
pub use logical_type::LogicalType;
pub use query_result::{CSVOptions, QueryResult};
pub use value::{InternalID, NodeVal, RelVal, Value};
```

### C API
`src/c_api/` — C bindings exposing database operations

---

## 15. Summary

**LadybugDB** (née Kuzu) is a full-featured **embedded property graph database** written in C++20 with bindings for Rust, Python, Node.js, Java, C, Go, Swift, and WebAssembly. It uses a **columnar disk-based storage engine** with CSR adjacency lists, shadow-paged checkpoints, WAL-based durability, and MVCC for serializable ACID transactions. Query processing is **vectorized and factorized** with extensive optimization passes. It supports **Cypher** as its query language and includes **13 extensions** covering graph algorithms (PageRank, Louvain, SCC/WCC, K-Core, spanning forest), full-text search, HNSW vector search, and integrations with DuckDB, PostgreSQL, SQLite, Neo4j, Iceberg, Delta Lake, and more. The Rust crate (`lbug`, v0.17.0) wraps everything via `cxx` FFI and is the primary alternative to the Python/Node.js bindings.

---

