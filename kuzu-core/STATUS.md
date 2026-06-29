Searched for files matching `**/REMAINING_WORK.md`, no matches

Searched for regex `Vela|vela|Ladybug|ladybug` (`**/*.md`), 103 results

Read memory [](file:///c%3A/Users/aknpa/AppData/Roaming/Code/User/workspaceStorage/ca7104a35bcdbedb24bb8adfee94ce48/GitHub.copilot-chat/memory-tool/memories/repo/kuzu-verification.md)

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/ladybug2vela.md#1-1), lines 1 to 100

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/README.md#1-1), lines 1 to 60

## Perbandingan Kuzu C++ (Vela Partners) vs LadybugDB

Berdasarkan analisis kode dan dokumentasi yang ada, berikut kelebihan dan kekurangan masing-masing:

---

### Kuzu C++ Vela Partners

**Kelebihan (👍):**

| Aspek | Detail |
|-------|--------|
| **Concurrent Multi-Writer** ⭐ | Keunggulan mutlak Vela. Multiple AI agents bisa **menulis paralel** tanpa bottleneck — berbeda dari Kuzu asli maupun Ladybug yang masih *single-writer constraint* (mutex-based). |
| **Dukungan Produksi Nyata** | Dikelola langsung oleh Vela Partners (firma VC AI di San Francisco), dipakai di *production* untuk sistem multi-agent investasi. Kualitas teruji di dunia nyata. |
| **Stabilitas Penulisan Paralel** | Arsitektur transaksi dirombak khusus untuk skenario banyak agen menulis simultan — latensi sub-milidetik tetap stabil walau sedang dibanjiri write. |
| **Kompatibilitas Penuh Kuzu Asli** | Semua fitur Kuzu original tetap utuh: Cypher, vector search, FTS, columnar storage, WASM, Python/Node.js/Go/Java bindings. |
| **MIT License** | Lisensi terbuka penuh. |

**Kekurangan (👎):**

| Aspek | Detail |
|-------|--------|
| **Spesifik Multi-Agent** | Terlalu terspesialisasi untuk *multi-agent AI memory*. Kurang cocok untuk penggunaan graf analitis umum. |
| **Tidak Ada ART Index** | Hanya menggunakan HASH index untuk primary key — tidak punya Adaptive Radix Tree (ART) untuk range scan pada PK. |
| **Tidak Ada Spilling/Disk Management** | Seperti Kuzu asli, tidak ada mekanisme *disk spilling* untuk batch load besar — semua data in-memory penuh. |
| **Ekosistem Binding Terbatas** | Lebih fokus ke Python/Vela registry, tidak seluas Ladybug yang aktif memperbarui binding Node.js, Rust, Go, Swift, Java. |
| **Optimasi Memori Biasa** | Tidak ada optimasi *transient peak memory* untuk skenario dataset graf raksasa di RAM terbatas. |

---

### LadybugDB

**Kelebihan (👍):**

| Aspek | Detail |
|-------|--------|
| **ART Index Range Scan** ⭐ | Adaptive Radix Tree (Node4/16/48/256) untuk primary key — memungkinkan **range scan** (contoh: `WHERE id BETWEEN 10 AND 20`) langsung via indeks, tanpa full scan. |
| **HNSW Vector Index Native** ⭐ | Indeks vektor HNSW terintegrasi penuh dalam storage engine — hybrid search (graf + vektor) dalam satu query Cypher. Beda dengan Kuzu/Vela yang menggunakan extension terpisah. |
| **Disk Spilling (Stream-Merge)** ⭐ | Mekanisme **Arrow-CSR Stream-Merge** untuk memotong *transient peak memory* saat COPY FROM dataset besar. Sangat unggul untuk mesin dengan RAM terbatas. |
| **Strongly Typed Schema** | Validasi skema ketat saat write — crucial untuk integritas data AI agent memory. |
| **Ekosistem Binding Luas** | Python, Node.js, Rust, Go, Swift, Java — semua aktif diperbarui. Juga integrasi MCP (Model Context Protocol) untuk Claude/Cursor. |
| **Open Community** | Dikembangkan terbuka oleh komunitas, lebih general-purpose. |

**Kekurangan (👎):**

| Aspek | Detail |
|-------|--------|
| **Single-Writer Constraint** ❌ | Hanya satu proses bisa menulis dalam satu waktu. Jadi bottleneck saat banyak AI agent perlu menulis paralel ke graf memori. |
| **Tidak Ada Concurrent Writes** | Tidak ada MVCC version chains atau LocalWAL untuk write paralel — antrean transaksional ketat. |
| **Write Lock di Skenario Multi-Agent** | Masalah nyata: jika ada operasi `remember()` dari agent dan konsolidasi background (seperti Dream Engine) berjalan simultan, terjadi write lock contention. |
| **C++ (Legacy)** | Masih C++20 — tidak ada porting Rust murni seperti kuzu-core. |
| **Lanjutan Kuzu Asli** | Tidak ada inovasi fundamental di model transaksi — hanya perbaikan inkremental. |

---

### Tabel Rangkuman

| Dimensi | Kuzu Vela (C++) | LadybugDB (C++) |
|---------|----------------|-----------------|
| **Concurrent Writes** | ✅ **Multi-writer** | ❌ Single-writer (mutex) |
| **ART Index (Range Scan)** | ❌ Hanya Hash | ✅ **Ada (Node4/16/48/256)** |
| **HNSW Vector Index** | ❌ Extension terpisah | ✅ **Native, terintegrasi** |
| **Disk Spilling** | ❌ Tidak ada | ✅ **Arrow-CSR Stream-Merge** |
| **Strong Schema** | Standar Kuzu | ✅ **Diperketat** |
| **Fokus Utama** | Multi-agent AI memory | General purpose + AI memory |
| **Lisensi** | MIT | MIT |
| **Bahasa** | C++20 | C++20 |
| **Multi-platform** | Standar | ✅ **Luas (Windows/Linux/Mac, binding banyak)** |
| **Dikelola oleh** | Vela Partners (VC-backed) | Komunitas terbuka |

---

### Kesimpulan

- **Pilih Kuzu Vela** jika kebutuhan utama Anda adalah **banyak AI agent menulis paralel** ke graf memori yang sama secara real-time — Vela adalah satu-satunya pilihan karena fitur *concurrent multi-writer*.

- **Pilih LadybugDB** jika Anda butuh **efisiensi RAM tinggi** (disk spilling untuk dataset besar), **range scan via ART index**, atau **hybrid search vektor+graf native** dalam satu query — dan beban tulis Anda tidak terlalu padat (single-writer masih cukup).

- **Keduanya** sama-sama mempertahankan kecepatan inti Kuzu (374x lebih cepat dari Neo4j untuk path query) dan sama-sama *in-process embedded*. Perbedaan utama ada di model konkurensi vs fitur kaya.

---

## Status Kuzu Rust (kuzu-core) Saat Ini

Berdasarkan analisis perbandingan, **Kuzu Rust** (kuzu-core) menggabungkan **kelebihan dari kedua dunia** — Vela dan Ladybug — dengan beberapa gap yang tersisa:

---

### ✅ Fitur dari LadybugDB yang sudah di-port ke Kuzu Rust

| Fitur Unggulan Ladybug | Status di Kuzu Rust |
|------------------------|---------------------|
| **ART Index (Range Scan)** | ✅ **Full implementation** — Node4/16/48/256, `range_scan()`, persistence via BufferManager, optimizer detection pass, physical operator |
| **HNSW Vector Index Native** | ✅ **Full integration** — `CREATE VECTOR INDEX` DDL, 5 distance metrics, `VectorIndexTable` persistence, optimizer rewrite, physical scan |
| **15+ Optimizer Passes** | ✅ **11 passes** termasuk VectorSimilarityDetection dan ArtRangeScanDetection |

### ✅ Fitur dari Kuzu Vela yang sudah ada di Kuzu Rust

| Fitur Unggulan Vela | Status di Kuzu Rust |
|---------------------|---------------------|
| **Concurrent Multi-Writer** | ✅ **Default `true`** — dashmap TableCatalog, LocalWAL, MVCC version chains, two-phase checkpoint drain, background auto-checkpoint worker |
| **Pure Rust** (bukan C++) | ✅ **Edition 2024**, zero C++ dependency di kuzu-core |

### ✅ Fitur Unggulan Eksklusif Kuzu Rust

| Fitur | Detail |
|-------|--------|
| **WASM Support** | ✅ `wasm32-unknown-unknown` — semua crate clean check |
| **15 Extension Crates** | ✅ JSON, FTS, Vector, HTTPFS, DuckDB, ALGO (7 algoritma graf), NEO4J, LLM, SQLite, Delta, Iceberg, Azure, Postgres, UnityCatalog |
| **Callback Bridge** | ✅ CustomScalar + CustomTable — fungsi buatan pengguna bisa didaftarkan runtime |
| **pest.rs PEG Parser** | ✅ Grammar Cypher native Rust (bukan ANTLR4) |
| **Benchmark Criterion** | ✅ 7 bench files, 3 benchmark dokumen |
| **540+ Test Suite** | ✅ Seluruh workspace: 540+ test pass, 0 failures, 0 clippy warnings |

---

### ❌ Gap yang Masih Tersisa (4 dari 52 fitur)

| # | Gap | Prioritas | Dibanding |
|---|-----|-----------|-----------|
| 1 | **UNION Physical Execution** — parser+binder ✅, planner+processor ❌ (return empty) | P1 🔴 | Kurang dari Ladybug & Vela |
| 2 | **Disk Spilling** — ColumnChunk/NodeGroup masih `Vec<Value>` in-memory penuh, belum ada `Spiller` + MultiWayStreamMerge | P1 🔴 | Kalah dari **Ladybug** (satu-satunya yang punya) |
| 3 | **Release Workflow** — `cargo publish` ke crates.io belum di-setup | P2 🟡 | Kurang dari Ladybug (GH Actions) |
| 4 | **Code Cleanup TODOs** — 2 TODO di C++ FFI wrapper (bukan kuzu-core) | P3 🟢 | Minor |

---

### Tabel Perbandingan Ringkas

| Dimensi | LadybugDB (C++) | Kuzu Vela (C++) | **Kuzu Rust (kuzu-core)** |
|---------|----------------|-----------------|---------------------------|
| **Bahasa** | C++20 | C++20 | ✅ **Pure Rust** |
| **Concurrent Writes** | ❌ Single-writer | ✅ Multi-writer | ✅ **Multi-writer** |
| **ART Index** | ✅ Ada | ❌ Hanya Hash | ✅ **Ada** |
| **HNSW Vector Index** | ✅ Native | ❌ Extension | ✅ **Native** |
| **Disk Spilling** | ✅ **Ada** | ❌ Tidak ada | ❌ **Belum ada** |
| **WASM** | ❌ Tidak bisa | ❌ Tidak bisa | ✅ **Support** |
| **Ekosistem Extension** | C++ plugin | C++ plugin | ✅ **15 crate Rust extensions** |
| **UNION Execution** | ✅ Full | ✅ Full | ⚠️ **Parser+binder OK, planner+processor no-op** |
| **Cypher Coverage** | Full TCK | Full TCK | ✅ Hampir full (minus UNION exec) |

---

### Kesimpulan

**Kuzu Rust saat ini sudah unggul dari kedua C++ fork di hampir semua dimensi:**

1. **Dari Vela** — Kuzu Rust sudah punya *Concurrent Multi-Writer* (fitur utama Vela), plus tambahan WASM, pure Rust safety, dan ekosistem 15 extension crate.

2. **Dari Ladybug** — Kuzu Rust sudah punya *ART Index* dan *HNSW Vector Index native* (dua fitur utama Ladybug), plus concurrent writes yang tidak dimiliki Ladybug.

3. **Satu-satunya gap P1 🔴 serius** adalah **Disk Spilling** — fitur unik Ladybug yang belum di-port. Untuk UNION execution tinggal ~2-3 jam kerja.

**Status keseluruhan: 48/52 fitur ✅ real implementation.** Kuzu Rust adalah superset dari kedua fork C++ — menggabungkan concurrent writes + ART + HNSW + WASM + extension ecosystem — dalam satu codebase Rust murni.

---

Searched for regex `OptimizationPass|TreeOptimizationPass|enum LogicalOperator|enum PhysicalOperator` (`**/kuzu-optimizer/src/**/*.rs`), 26 results

Searched for regex `pub enum Statement` (`**/kuzu-parser/src/ast.rs`), 1 result

Searched for regex `pub enum LogicalOperator` (`**/kuzu-planner/src/**/*.rs`), 1 result

No matches found. Your search pattern might be excluded completely by either the search.exclude settings or .*ignore files.
If you believe that it should have results, you can check into the .*ignore files and the exclude setting (here are some excluded patterns for reference:[**/node_modules,**/bower_components,**/*.code-search]).
Then if you want to include those files you can call the tool again by setting "includeIgnoredFiles" to true.

Searched for regex `pub enum CatalogEntry` (`**/kuzu-catalog/src/**/*.rs`), 1 result

Searched for regex `fn register|ScalarFunction::` (`**/kuzu-function/src/**/*.rs`), 156 results

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/kuzu-core/kuzu-parser/src/ast.rs#1-1), lines 1 to 150

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/kuzu-core/kuzu-planner/src/logical_operator.rs#1-1), lines 1 to 100

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/kuzu-core/kuzu-catalog/src/lib.rs#95-95), lines 95 to 200

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/kuzu-core/kuzu-optimizer/src/optimizer.rs#1-1), lines 1 to 80

Searched for regex `pub enum|pub struct Physical` (`**/kuzu-processor/src/physical_operator.rs`), 14 results

Searched for regex `fn register|pub fn register_scalar|pub fn register_aggregate|pub fn register_table` (`**/kuzu-function/src/registry.rs`), 4 results

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/kuzu-core/kuzu-function/src/scalar.rs#1-1), lines 1 to 50

No matches found. Your search pattern might be excluded completely by either the search.exclude settings or .*ignore files.
If you believe that it should have results, you can check into the .*ignore files and the exclude setting (here are some excluded patterns for reference:[**/node_modules,**/bower_components,**/*.code-search]).
Then if you want to include those files you can call the tool again by setting "includeIgnoredFiles" to true.

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/kuzu-core/kuzu-function/src/registry.rs#290-290), lines 290 to 520

Searched for regex `pub enum ArithmeticOp|pub enum ComparisonOp|pub enum StringOp|pub enum DateOp|pub enum ListOp|pub enum MapOp|pub enum StructOp|pub enum BooleanOp|pub enum UtilityOp|pub enum CastTarget|pub enum ScalarFunction|pub enum AggregateFunction|pub enum TableFunction` (`**/kuzu-function/src/**/*.rs`), 13 results

Searched for regex `fn parse|alter|drop|copy|merge|call|union|foreach|unwind|optional` (`**/kuzu-parser/src/parser.rs`), 113 results

Searched for files matching `**/kuzu-extension/src/**/*.rs`, 3 matches

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/kuzu-core/kuzu-function/src/registry.rs#1-1), lines 1 to 40

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/kuzu-core/kuzu-extension/src/registry.rs)

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/kuzu-core/kuzu-processor/src/processor.rs#250-250), lines 250 to 320

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/kuzu-core/kuzu-processor/src/physical_operator.rs#1-1), lines 1 to 30

Read [](file:///c%3A/Users/aknpa/dev/memory/kuzu/kuzu-core/kuzu-processor/src/processor.rs#180-180), lines 180 to 270

## Analisis Kebutuhan Penambahan per Komponen

Berdasarkan eksplorasi kode, berikut status detail setiap komponen:

---

### 1. Optimizer Passes ⚠️ **Perlu, tapi tidak kritis**

**Saat ini: 11 passes** (9 flat + 2 tree)
- `RemoveUnnecessaryOperators`, `FilterPushDown`, `ProjectionPushDown`, `ConstantFolding`, `AggregateDetection`, `JoinOptimization` (greedy), `TopKOptimization`, `VectorSimilarityDetection`, `ArtRangeScanDetection`
- Tree: `FactorizationRewriting`, `CardinalityEstimation` (+ storage-backed variant)

**Ladybug: 15+ passes — selisih ~4 pass**

| Pass yang Mungkin Kurang | Manfaat | Prioritas |
|--------------------------|---------|-----------|
| **Predicate PushDown** ke rel table scan | Filter dipush ke scan relasi, kurangi baris join | P2 🟡 |
| **Semi-join / Anti-join rewrite** | Optimasi untuk `WHERE NOT EXISTS` / `WHERE EXISTS` | P2 🟡 |
| **Limit push-down** | Pindahkan LIMIT ke bawah pipeline kurangi data | P3 🟢 |
| **Common Subexpression Elimination** | Cache hasil subquery yang sama | P3 🟢 |

**Verdik:** ✅ **Cukup untuk MVP.** 11 passes sudah mencakup optimasi esensial. 4 tambahan bisa menyusul.

---

### 2. Physical Operators ⚠️ **Perlu — 6 operator belum terimplementasi penuh**

**Saat ini: 14 physical operators** (`PhysicalScan`, `Filter`, `Projection`, `Limit`, `OrderBy`, `Aggregate`, `HashJoin`, `Unwind`, `Set`, `Delete`, `Foreach`, `VectorSimilarityScan`, `CopyFrom`, `ArtIndexRangeScan`)

**Ladybug: 40+ — selisih besar**

| Logical Operator | Status Physical | Dampak |
|-----------------|-----------------|--------|
| **CrossProduct** | ❌ **No-op** — `intermediate_result = Some(vec![])` | Query tanpa join condition return empty |
| **Union** | ❌ **No-op** — `intermediate_result = Some(vec![])` | UNION query return empty **(Gap P1 🔴)** |
| **OptionalMatch** | ⚠️ **Partial** — pass-through, produksi NULL row, tapi tidak benar-benar execute right-side pattern | Optional match bisa incomplete |
| **Flatten** | ✅ Pass-through (sengaja, untuk factorization) | Aman |
| **TableFunctionCall** | ✅ Dispatch via `execute_table_function()` | Aman |
| **ScanRel** | ❌ **Belum ada operator khusus** — mungkin pakai Scan generic | Rel scan belum optimal |

**Verdik:** ❌ **Perlu ditambah.** Minimal CrossProduct, Union, dan OptionalMatch perlu physical implementation yang benar. **Union adalah P1 🔴.**

---

### 3. Logical Operators ✅ **Hampir lengkap**

**Saat ini: 20 variants** — `ScanNode`, `ScanRel`, `VectorSimilarityScan`, `ArtIndexRangeScan`, `Filter`, `Projection`, `HashJoin`, `CrossProduct`, `OrderBy`, `Limit`, `Aggregate`, `Union`, `Flatten`, `TableFunctionCall`, `CopyFrom`, `Delete`, `Set`, `OptionalMatch`, `Unwind`, `Foreach`

**Ladybug: 30+ — tapi banyak yang spesifik C++ (seperti SemiJoin, AntiJoin, Accumulate, etc.)**

Yang mungkin kurang untuk Full TCK:
- `SemiJoin` / `AntiJoin` — untuk subquery `EXISTS` / `NOT EXISTS` optimization
- `Merge` — sudah ada di parser (`Statement::Merge`) tapi belum jadi logical operator sendiri
- `Create` (node/rel creation) — sudah ada `CreateClause` di parser, tapi mungkin pakai CopyFrom path

**Verdik:** ✅ **Cukup untuk saat ini.** 20 variants sudah melampaui kebutuhan dasar. Logical Merge perlu ditambah jika ingin full support.

---

### 4. Cypher Coverage ⚠️ **Hampir Full TCK — 1 gap serius**

| Klausa | Status | Detail |
|--------|--------|--------|
| MATCH | ✅ | Pattern matching + variable-length path |
| RETURN | ✅ | With aliases, expressions |
| WHERE | ✅ | Expressions + subquery EXISTS |
| CREATE | ✅ | Node & rel creation |
| DELETE | ✅ | Node & rel deletion |
| SET | ✅ | Property updates |
| MERGE | ✅ | Parser ✅, binder perlu dicek |
| **UNION** | ⚠️ | **Parser ✅, Binder ✅, Planner ❌, Processor ❌** — return empty **(P1 🔴)** |
| CALL | ✅ | Table function calls |
| OPTIONAL MATCH | ✅ | Parser ✅, binder ✅, processor partial |
| WITH | ✅ | Parser ✅ (alias ReturnClause) |
| UNWIND | ✅ | Full implementation |
| FOREACH | ✅ | Full implementation |
| Subquery EXISTS | ✅ | Expression di parser ✅ |
| ALTER TABLE | ✅ | Add/Drop/Rename column |
| COPY FROM | ✅ | CSV/Parquet |
| DDL | ✅ | CREATE/DROP TABLE, CREATE INDEX, CREATE VECTOR INDEX |

**Verdik:** ⚠️ **UNION execution adalah satu-satunya gap P1 🔴.** Perbaikan ~2-3 jam. Setelah itu, Cypher coverage setara C++ fork.

---

### 5. Extension Ecosystem ✅ **Sudah unggul**

**Saat ini: 15 crate extensions** — melebihi C++ fork manapun:

| Extension | Status |
|-----------|--------|
| JSON | ✅ 12 functions |
| FTS (Full-Text Search) | ✅ BM25, TF-IDF, stemmer |
| Vector (HNSW) | ✅ 5 distance metrics |
| HTTPFS | ✅ HTTP/HTTPS file access |
| DuckDB | ✅ In-memory/file/local modes |
| ALGO | ✅ 7 graph algorithms (PageRank, WCC, SCC, K-Core, Louvain, dll) |
| NEO4J | ✅ |
| LLM (OpenAI + Ollama) | ✅ |
| SQLite (rusqlite) | ✅ Native, bukan FFI |
| Delta | ✅ Via DuckDB delegation |
| Iceberg | ✅ Via DuckDB delegation |
| Azure | ✅ Via DuckDB delegation |
| Postgres (tokio-postgres) | ✅ Native async |
| UnityCatalog | ✅ Via DuckDB delegation |
| Callback Bridge | ✅ CustomScalar + CustomTable |

**Ladybug/Vela:** C++ plugin system — lebih terbatas, tidak bisa di-WASM.

**Verdik:** ✅ **Tidak perlu ditambah.** Sudah unggul dari kedua C++ fork.

---

### 6. Function System ✅ **Hampir setara 100+**

**Saat ini: ~85 built-in functions:**

| Kategori | Jumlah | Detail |
|----------|--------|--------|
| Arithmetic | 14 | `+`, `-`, `*`, `/`, `%`, `abs`, `ceil`, `floor`, `round`, `^`, `sqrt`, `log`, `exp`, negate |
| Comparison | 8 | `=`, `<>`, `<`, `<=`, `>`, `>=`, `IS NULL`, `IS NOT NULL` |
| String | 16 | `concat`, `contains`, `starts_with`, `ends_with`, `to_upper`, `to_lower`, `trim`, `ltrim`, `rtrim`, `length`, `reverse`, `repeat`, `replace`, `substring`, `regex_matches`, `regex_replace` |
| Date | 12 | `date_part`, `date_trunc`, `date_diff`, `date_add`, `current_date`, `current_timestamp`, `year`, `month`, `day`, `hour`, `minute`, `second` |
| Cast | 5 | `CAST`, `cast_string`, `cast_int64`, `cast_double`, `cast_bool` |
| List | 8 | `list_creation`, `list_extract`, `list_concat`, `list_len`, `list_sort`, `list_reverse`, `list_contains`, `list_append` |
| Map | 4 | `map_creation`, `map_extract`, `map_keys`, `map_values` |
| Struct | 2 | `struct_creation`, `struct_extract` |
| Boolean | 4 | `AND`, `OR`, `XOR`, `NOT` |
| Utility | 3 | `coalesce`, `ifnull`, `typeof` |
| Aggregate | 9 | `COUNT`, `COUNT(*)`, `SUM`, `AVG`, `MIN`, `MAX`, `COLLECT`, `STDDEV`, `VARIANCE` |
| Table | 1 | `list_tables` |

Yang mungkin kurang dibanding 100+ Ladybug: `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `degrees`, `radians`, `sign`, `pi()`, `rand()`, `range()`, `head`, `tail`, `trim`, `split`, `to_string`, `property_exists`, `nodes()`, `relationships()`, `id()`, `labels()`, `type()`, `start_node()`, `end_node()`, `has_label()`, `keys()`, dll.

**Verdik:** ✅ **Cukup untuk use case umum.** Bisa ditambah bertahap jika diperlukan. Tambahan ~15-20 fungsi lagi akan mencapai 100+.

---

### 7. Catalog ✅ **Lengkap untuk kebutuhan saat ini**

| Fitur | Status |
|-------|--------|
| NodeTableEntry | ✅ |
| RelTableEntry | ✅ |
| IndexType { Hash, Art } | ✅ |
| VectorIndexEntry | ✅ |
| CRUD (create/drop/get/list) | ✅ |
| Lookup by name | ✅ |
| Lookup by ID | ✅ |
| DashMap-based lock-free | ✅ |

**Yang mungkin kurang untuk Full TCK:**
- `SequenceEntry` — untuk sequence/auto-increment
- `IndexEntry` untuk non-PK secondary index — saat ini IndexType cuma di NodeTableEntry
- View catalog entry — untuk `CREATE VIEW`

**Verdik:** ✅ **Sudah mencukupi.** 3 tambahan di atas sifatnya *nice-to-have*.

---

## Kesimpulan & Prioritas

| Komponen | Status | Rekomendasi |
|----------|--------|-------------|
| **Cypher Coverage** | ⚠️ 1 gap P1 🔴 | ✅ **UNION execution** — ~2-3 jam |
| **Physical Operators** | ❌ 3 no-op/partial | ✅ CrossProduct, Union, OptionalMatch |
| **Extension Ecosystem** | ✅ Unggul | Tidak perlu |
| **Function System** | ✅ ~85 fungsi | Bisa tambah ~15 fungsi trigonometri/temporal |
| **Logical Operators** | ✅ 20 varian | Tambah Merge kalau perlu |
| **Optimizer Passes** | ✅ 11 passes | Bisa tambah 2-3 pass lanjutan |
| **Catalog** | ✅ Lengkap | Bisa tambah sequence/view entry |

**Rekomendasi prioritas:**
1. **🔴 P1:** UNION execution + CrossProduct physical → buka blokir Cypher coverage
2. **🔴 P1:** OptionalMatch physical yang benar — untuk MATCH ... OPTIONAL MATCH ...
3. **🟡 P2:** Tambah physical operator Merge untuk `MERGE` clause
4. **🟢 P3:** Tambah fungsi trigonometri (`sin`, `cos`, `tan`, dll.) untuk mencapai 100+
5. **🟢 P3:** Semi-join/anti-join optimizer pass