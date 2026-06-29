# Rencana Implementasi Fitur Unggulan LadybugDB ke Kuzu Rust (kuzu-core)

Dokumen ini membandingkan basis kode **LadybugDB** (C++) dengan porting Rust **Kuzu Core** (`kuzu-core`), mendaftar fitur unggulan LadybugDB yang belum ada di Kuzu Rust, serta merancang rencana implementasi terperinci untuk mengadopsi fitur-fitur tersebut ke dalam Kuzu Rust.

> **Status Terakhir: 2026-06-30** — Concurrent Multi-Writer (Phase A-C) telah selesai diimplementasikan. Fokus berikutnya: ART Index, HNSW Full Integration, Disk Spilling.

---

## 1. Analisis Perbandingan Codebase

Berikut adalah peta perbandingan arsitektur antara tiga varian evolusi Kuzu dengan status terkini:

| Dimensi | **LadybugDB (C++ Fork)** | **Kuzu (Vela Partners C++ Fork)** | **Kuzu Core (Pure Rust Port)** |
|---|---|---|---|
| **Fokus Utama** | Efisiensi graf analitis lokal, AI Agent memory, HNSW native | Multi-Agent concurrency, stabilitas penulisan paralel | Re-implementasi penuh Kuzu ke Rust tanpa dependensi C++ |
| **Model Transaksi** | *Single-Writer Constraint* (tradisional ACID) | **Concurrent Multi-Writer Support** (paralel writes) | ✅ **Concurrent Multi-Writer** (dashmap + LocalWAL + MVCC, default `true`) |
| **Indeks Vektor** | Native HNSW terintegrasi di dalam graf & query engine | Sama dengan upstream Kuzu (ekstensi terpisah) | ⚠️ `HnswIndex` in-memory (`kuzu-vector`), fungsi skalar ✅, **parser/integrasi ❌** |
| **Indeks PK** | Mendukung **HASH** dan **ART** (Adaptive Radix Tree) | Mendukung HASH | ✅ **HASH** (`HashIndex` + `OnDiskHashIndex`), **ART ❌** |
| **Manajemen Memori** | Spilling ke disk & *batch stream-merge* di Arrow-CSR | Pengelolaan antrean transaksi C++ | ❌ Belum ada spilling ke disk |
| **Interoperabilitas** | C++ native dengan binding eksternal luas | C++ native dengan fokus ke Python/Vela | Rust native murni, CLI (`kuzu-cli`) ✅ |
| **CI/CD** | GitHub Actions penuh | — | ❌ **Belum ada** (D1-D4 masih pending) |
| **Benchmark** | C++ benchmark suite | — | ❌ **Belum ada** (perlu `criterion`) |

---

## 2. Ringkasan Status Implementasi Kuzu Rust (kuzu-core)

### ✅ Sudah Diimplementasikan

| Area | Detail | Crate |
|------|--------|-------|
| **Concurrent Multi-Writer** | `concurrent_writes=true` default, dashmap TableCatalog, LocalWAL, MVCC version chains (VersionInfo/UpdateInfo), two-phase checkpoint drain, background auto-checkpoint worker, BEGIN/COMMIT/ROLLBACK Cypher | `kuzu-transaction`, `kuzu-storage`, `kuzu-main` |
| **Storage Engine** | WAL + recovery, checkpoint, BufferManager, ShadowFile, LocalStorage, ColumnChunk, NodeGroup, Column, page management, compression | `kuzu-storage` |
| **HashIndex (PK)** | Two-layer: L1 `HashMap<K,u64>` in-memory + L2 `OnDiskHashIndex` via BufferManager pages | `kuzu-storage/src/index.rs` |
| **Table Storage** | `NodeTable` (NodeGroup-based columnar), `RelTable` (CSR adjacency), `TableCatalog` (DashMap) | `kuzu-storage/src/table.rs` |
| **Cypher Parser** | Pest-based grammar: MATCH, RETURN, WHERE, CREATE, DELETE, SET, MERGE, UNION, CALL, OPTIONAL MATCH, WITH, UNWIND, FOREACH, variable-length path, subquery, ALTER, COPY FROM, DDL | `kuzu-parser` |
| **Binder** | Full binding for all statement types, type resolution, schema validation | `kuzu-binder` |
| **Planner** | Logical plan construction from bound statements | `kuzu-planner` |
| **Optimizer** | 9 passes: FilterPushDown, ProjectionPushDown, ConstantFolding, JoinOptimization (greedy), TopKOptimization, FactorizationRewriting (tree), CardinalityEstimation (tree), RemoveUnnecessary, AggregateDetection | `kuzu-optimizer` |
| **Processor** | 16 physical operators: Scan, Filter, Projection, Limit, OrderBy, Aggregate, HashJoin (generalized), Unwind, CopyFrom, Merge, Foreach, OptionalMatch, Delete, Set, ExpressionEvaluator | `kuzu-processor` |
| **Functions** | Scalar (string, numeric, datetime, boolean, list, cast), aggregate (COUNT, SUM, MIN, MAX, AVG), table functions | `kuzu-function` |
| **Graph Module** | CSR adjacency, in-memory Graph, OnDiskGraph placeholder | `kuzu-graph` |
| **Catalog** | CRUD table entries, column management, name/ID lookup | `kuzu-catalog` |
| **Extension System** | Extension trait + registry, 15 extension crates registered: JSON, FTS, Vector, HTTPFS, DuckDB, ALGO, NEO4J, LLM, SQLite, Delta, Iceberg, Azure, Postgres, UnityCatalog | `kuzu-extension` + `kuzu-*` |
| **CLI** | REPL with history, `kuzu-cli` binary | `kuzu-cli` |
| **PreparedStatement** | Parameterized queries with `$param` syntax | `kuzu-main` |
| **HNSW In-Memory** | `HnswIndex` struct with `insert()`/`search()`, greedy+beam search, 5 distance metrics (Cos, Eucl, L1, L2Sq, Dot) | `kuzu-vector/src/hnsw.rs` |
| **WAL + Recovery** | Crash recovery on startup, replay, auto-checkpoint after checkpoint threshold | `kuzu-storage` |
| **tools/rust_api** | Rust-native API via `Database` + `Connection` | `kuzu-main` |

### ❌ Belum Diimplementasikan (Prioritas)

| Prioritas | Fitur | Detail | Referensi C++ (Ladybug) |
|-----------|-------|--------|------------------------|
| **P0** | **ART Index** | Adaptive Radix Tree untuk range scan PK | `ladybug/src/storage/index/art_index.h` (Node4/16/48/256, `ArtPrimaryKeyIndex`), `art_index.cpp`, `art_index_disk.cpp` |
| **P0** | **HNSW Full Integration** | Parser `CREATE VECTOR INDEX`, catalog registrasi, persistensi via BufferManager, physical operator `VectorSimilarityScan` | (Tidak ada di ladybug — HNSW native asli Kuzu) |
| **P1** | **Disk Spilling** | `Spiller` + stream-merge untuk batch insert besar | `ladybug/src/storage/buffer_manager/spiller.h`, `spiller.cpp`, `spill_result.h` |
| **P1** | **CI/CD** | GitHub Actions build + test + release | — |
| **P2** | **Benchmark** | `criterion` benchmark suite vs C++ baseline | `ladybug/benchmark/` |
| **P2** | **DuckDB Binding** | DuckDB Rust binding via callback bridge | — |

---

## 3. Fitur Unggulan LadybugDB yang Belum Ada di Kuzu Rust

### A. Indeks ART (Adaptive Radix Tree) untuk Primary Key
*   **Deskripsi:** Radix tree adaptif berbasis byte-ordered keys yang menggantikan atau berjalan paralel dengan `HashIndex`.
*   **Keunggulan:** Mendukung pencarian rentang (*range scans*) pada primary key (misal: `p.ID >= 10 AND p.ID < 20`), pemulihan crash (*rollback cleanup*), checkpointing terstruktur, dan efisiensi traversal tinggi.
*   **Referensi C++:** `ladybug/src/include/storage/index/art_index.h` (~300+ LOC) — mendefinisikan `ArtKey`, `ArtPrimaryKeyIndexStorageInfo`, `ArtPrimaryKeyIndex` dengan NODE4/NODE16/NODE48/NODE256. Juga `art_index_disk_utils.h` untuk serialisasi disk dan shadow file.
*   **Parser C++:** `ladybug/src/parser/transform/transform_ddl.cpp` — `CREATE [ART|HASH] INDEX name FOR (n:Label) ON (n.prop)`, default HASH.
*   **Dokumentasi C++:** `ladybug/docs/art_index.md` — contoh penggunaan.
*   **Status Kuzu Rust:** Hanya memiliki `HashIndex` (`kuzu-storage/src/index.rs`) yang tidak mendukung range scan (hanya equality lookup).

### B. Indeks Vektor Native HNSW yang Terintegrasi Penuh
*   **Deskripsi:** Indeks HNSW (Hierarchical Navigable Small World) yang terhubung langsung dengan catalog, storage manager, parser Cypher, optimizer, dan processor.
*   **Keunggulan:** Mengeksekusi pencarian kemiripan vektor (*vector similarity search*) secara hibrida bersanding dengan traversal graf analitis dalam satu kueri Cypher.
*   **Status Kuzu Rust:** ✅ `HnswIndex` in-memory dengan `insert()` dan `search()` sudah ada di `kuzu-vector/src/hnsw.rs` (~400 LOC, lengkap dengan greedy search + layer-0 beam search + 5 distance metrics). ❌ Yang belum:
    - Parser: tidak ada grammar `CREATE VECTOR INDEX`
    - Binder: tidak ada `BoundCreateIndex` untuk vector
    - Catalog: tidak ada registrasi `IndexType::HNSW`
    - Storage: `HnswIndex` tidak terhubung ke `BufferManager` (belum persistensi)
    - Processor: tidak ada `PhysicalVectorSimilarityScan`

### C. Manajemen Memori: Arrow-CSR Spilling & Stream-Merge
*   **Deskripsi:** Mekanisme pengontrolan lonjakan memori transien (*transient peak memory*) menggunakan `Spiller` yang memindahkan *sorted runs* data ke disk saat batch insert melewati batas memori, kemudian digabungkan secara streaming (*stream-merge*).
*   **Keunggulan:** Menjaga performa tetap stabil di mesin berspesifikasi rendah/RAM terbatas saat melakukan `COPY FROM` dataset graf raksasa.
*   **Referensi C++:** `ladybug/src/include/storage/buffer_manager/spiller.h`, `ladybug/src/storage/buffer_manager/spiller.cpp`, `ladybug/src/include/storage/buffer_manager/spill_result.h`.
*   **Status Kuzu Rust:** Belum ada mekanisme spilling ke disk di `ColumnChunk` atau `NodeGroup` selama batch load/DML.

---

## 4. Rencana Implementasi untuk Kuzu Rust (kuzu-core)

### FASE 1: Porting ART Primary Key Index ⬅️ **PRIORITAS TERTINGGI**
Fase ini berfokus pada penambahan struktur indeks radix tree adaptif ke `kuzu-storage` dan integrasinya dengan parser, catalog, dan optimizer.

**Referensi C++ utama:**
- `ladybug/src/include/storage/index/art_index.h` — Definisi kelas `ArtPrimaryKeyIndex`, `ArtKey`, tipe node
- `ladybug/src/storage/index/art_index.cpp` — Implementasi ART
- `ladybug/src/storage/index/art_index_disk.cpp` — Serialisasi/deserialisasi disk
- `ladybug/src/include/storage/index/art_index_disk_utils.h` — Utilitas shadow file
- `ladybug/src/parser/transform/transform_ddl.cpp` — `CREATE [ART|HASH] INDEX` parsing
- `ladybug/docs/art_index.md` — Dokumentasi penggunaan

#### Sub-steps:

| Step | File | Perubahan |
|------|------|-----------|
| **1.1** | `kuzu-storage/src/art_index.rs` **(NEW)** | Implementasi `ArtKey` (order-preserving byte encoding untuk Int64, Float, String, Date, Timestamp dll.), tipe node `Node4`/`Node16`/`Node48`/`Node256`, dan `ArtPrimaryKeyIndex` dengan operasi `insert`, `lookup`, `delete`, `range_scan`, `checkpoint`/`load`. Port dari C++. |
| **1.2** | `kuzu-catalog/src/lib.rs` | Tambah enum `IndexType { Hash, Art }`. Tambah `index_type` field ke `NodeTableEntry`. |
| **1.3** | `kuzu-parser/src/ast.rs` | Tambah varian `Statement::CreateIndex` dan `Statement::DropIndex`. Tambah struct `CreateIndexInfo` dengan `index_type`, `index_name`, `table_name`, `variable`, `property`, `conflict_action`. |
| **1.4** | `kuzu-parser/src/parser.rs` | Grammar untuk `CREATE [ART|HASH] INDEX name FOR (n:Label) ON (n.prop)`. Port dari C++. |
| **1.5** | `kuzu-binder/src/bound_statement.rs` | Tambah `BoundStatement::BoundCreateIndex` dan `BoundStatement::BoundDropIndex`. |
| **1.6** | `kuzu-binder/src/binder.rs` | Binding untuk `CreateIndex` — validasi tabel/kolom, resolusi tipe indeks. |
| **1.7** | `kuzu-planner/src/planner.rs` | Planning untuk `CreateIndex`/`DropIndex` — logical operator. |
| **1.8** | `kuzu-optimizer/src/passes.rs` | Filter push-down untuk ART: jika ada filter ketidaksamaan pada kolom berindeks ART, ubah physical plan jadi `ArtIndexRangeScan`. |
| **1.9** | `kuzu-processor/src/physical_operator.rs` | Operator baru `PhysicalArtIndexRangeScan` — ambil row ID dari `range_scan`, lalu fetch data kolom. |

---

### FASE 2: Integrasi Penuh HNSW Vector Index ⬅️ **PRIORITAS TINGGI**
Fase ini mengintegrasikan indeks HNSW yang sudah ada di `kuzu-vector` ke dalam mesin penyimpanan dan query database.

**Status awal:** ✅ `HnswIndex` in-memory (insert + search + 5 metrics) sudah ada di `kuzu-vector/src/hnsw.rs`.

#### Sub-steps:

| Step | File | Perubahan |
|------|------|-----------|
| **2.1** | `kuzu-vector/src/hnsw.rs` | Tambah serialisasi/deserialisasi: `serialize(&self) -> Vec<u8>` dan `deserialize(data, metric) -> Self`. |
| **2.2** | `kuzu-vector/src/lib.rs` | Integrasi dengan `BufferManager`: `flush_to_bm(bm, file_name, page_id)` dan `load_from_bm(bm, file_name, page_id)`. |
| **2.3** | `kuzu-catalog/src/lib.rs` | Tambah `IndexType::HNSW`. Tambah field `vector_index_info` (dimension, metric) ke `NodeTableEntry`. |
| **2.4** | `kuzu-storage/src/table.rs` | Integrasi `HnswIndex` ke `NodeTable` — auto-insert vektor saat `insert_row()` jika ada HNSW index. |
| **2.5** | `kuzu-parser/src/ast.rs` + `parser.rs` | Grammar `CREATE VECTOR INDEX name FOR (n:Label) ON (n.embedding) WITH (dimension=128, metric=cosine)`. |
| **2.6** | `kuzu-binder/src/binder.rs` | Binding untuk `CreateVectorIndex`. |
| **2.7** | `kuzu-processor/src/physical_operator.rs` | Operator baru `PhysicalVectorSimilarityScan` — panggil `HnswIndex::search(query, k)`, lalu `PhysicalScan` untuk fetch properti node hasil. |
| **2.8** | `kuzu-optimizer/src/passes.rs` | Detection pass: jika WHERE clause memanggil `cosine_similarity`/`euclidean_distance` pada kolom berindeks HNSW, ganti dengan `VectorSimilarityScan`. |

---

### FASE 3: Implementasi Disk Spilling & Stream-Merge (Arrow-CSR)
Fase ini mengoptimalkan penulisan batch besar dengan menghemat konsumsi RAM melalui disk spilling.

**Referensi C++:**
- `ladybug/src/include/storage/buffer_manager/spiller.h`
- `ladybug/src/storage/buffer_manager/spiller.cpp`
- `ladybug/src/include/storage/buffer_manager/spill_result.h`

#### Sub-steps:

| Step | File | Perubahan |
|------|------|-----------|
| **3.1** | `kuzu-storage/src/spiller.rs` **(NEW)** | Port `Spiller`: konstruktor dengan `tmp_dir`, threshold memori. Method `spill(sorted_run)` — tulis sorted run ke file temp. |
| **3.2** | `kuzu-storage/src/column_chunk.rs` | Integrasi `Spiller` — saat `append()` batch besar melebihi threshold, spill ke disk. |
| **3.3** | `kuzu-storage/src/node_group.rs` | Integrasi `Spiller` — spill node group yang melebihi kapasitas memori. |
| **3.4** | `kuzu-storage/src/spiller.rs` | Implementasi `MultiWayStreamMerge` — baca multiple sorted runs, merge streaming, deduplikasi PK, tulis ke storage final. |
| **3.5** | `kuzu-main/src/connection.rs` | Konfigurasi `SET spill_threshold = bytes` — ekspos via Cypher. |

---

## 5. Verification Plan

Untuk memastikan implementasi berjalan dengan benar tanpa merusak fungsi yang sudah ada:

### Automated Tests
*   **Unit Tests Baru di `kuzu-storage`:**
    *   ART: key encoding untuk semua tipe data (`Int64`, `Float`, `String`, dll.) — byte-ordering konsisten
    *   ART: insert, lookup, delete, `range_scan` dengan berbagai ukuran dataset
    *   HNSW: serialisasi/deserialisasi roundtrip
    *   HNSW: persistensi via BufferManager
    *   Spiller: spill + stream-merge pada data yang melampaui threshold
*   **Integration Tests Baru di `kuzu-main`:**
    *   Range scan Cypher: `MATCH (p:Person) WHERE p.ID >= 100 AND p.ID <= 200 RETURN p.name` dengan indeks ART
    *   Vector similarity: `MATCH (p:Person) WHERE cosine_similarity(p.embedding, $query) > 0.8 RETURN p.name`
    *   Stress-test batch `COPY FROM` GB-scale dengan threshold memori rendah (50MB)
*   **Regression Tests:**
    *   Semua test yang sudah ada tetap lulus (`cargo test --workspace`)
    *   Single-writer mode (`SET concurrent_writes = false`) tetap kompatibel

### Manual Verification
*   Validasi kompatibilitas silang platform (Windows, Linux): `cargo build --workspace`
*   Verifikasi performa memori transien via logging statistik RAM selama batch loading
*   Perbandingan hasil query ART range scan vs HASH full-scan untuk dataset identik
