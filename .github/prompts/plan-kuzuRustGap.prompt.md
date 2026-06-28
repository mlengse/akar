# Plan: Implementasi Kekurangan Rust kuzu-core

## TL;DR
Implementasi Rust kuzu-core saat ini berupa **prototipe/skeleton** — struktur crate mengikuti C++ asli dengan baik, tapi banyak komponen hanya berupa struct definition tanpa logika eksekusi sebenarnya. Plan ini mengelompokkan pekerjaan ke dalam 6 fase bertahap, dari yang paling mudah dan memberikan nilai paling cepat, hingga yang paling kompleks. Setiap fase bersifat independen dan dapat dikerjakan paralel dalam tim, dengan dependensi minimal.

---

## Fase 1: Fungsi Built-in & Evaluator (EASY — parallel-friendly)
**Tujuan:** Membuat fungsi built-in yang sudah terdaftar di registry benar-benar berfungsi.

### Steps

1. **String function implementations** — `kuzu-function/src/scalar.rs`
   - Implement `evaluate_string` untuk semua 16 varian `StringOp`:
     - `Length`, `Reverse`, `Repeat`, `Replace`, `Substring` — operasi string Rust std
     - `Trim`, `LTrim`, `RTrim` — trim whitespace
     - `RegexMatches`, `RegexReplace` — pakai crate `regex` (already a dependency)
     - `StartsWith`, `EndsWith`, `Contains` — std `str` methods
   - *Depends on:* nothing
   - *Parallel with:* steps 2, 3, 4

2. **Date function implementations** — `kuzu-function/src/scalar.rs`
   - Implement `evaluate_date` untuk semua varian `DateOp`:
     - `Year`, `Month`, `Day`, `Hour`, `Minute`, `Second` — extract dari timestamp
     - `DatePart`, `DateTrunc`, `DateDiff`, `DateAdd` — operasi interval
     - `CurrentDate`, `CurrentTimestamp` — system time
   - Pakai crate `time` atau `chrono` (already a dependency)
   - *Depends on:* nothing
   - *Parallel with:* steps 1, 3, 4

3. **List/Map/Struct function implementations** — `kuzu-function/src/scalar.rs`
   - `ListOp::Len`, `ListOp::Extract`, `ListOp::Contains`, `ListOp::Append`, `ListOp::Prepend`, `ListOp::Reverse` — pakai `Value::List` internal
   - `MapOp::Keys`, `MapOp::Values` — pakai `Value::Map`
   - `StructOp::Extract` — pakai `Value::Struct`
   - *Depends on:* nothing (Value type sudah mature)
   - *Parallel with:* steps 1, 2, 4

4. **Cast function implementations** — `kuzu-function/src/scalar.rs`
   - `CastOp::String` → `Value::String` untuk semua tipe
   - `CastOp::Int64` → parse string/double/bool ke i64
   - `CastOp::Double` → parse string/int/bool ke f64
   - `CastOp::Bool` → parse string/int/double ke bool
   - *Depends on:* nothing
   - *Parallel with:* steps 1, 2, 3

5. **Aggregate function evaluation** — `kuzu-function/src/scalar.rs` + `kuzu-processor/src/physical_operator.rs`
   - Implement `evaluate_aggregate` di `scalar.rs` untuk COUNT, SUM, AVG, MIN, MAX, COLLECT, COUNT_STAR, STDDEV, VARIANCE
   - Wire ke `PhysicalAggregate::execute()` — saat ini agregasi hardcoded untuk Int64 saja, perlu dispatch ke `evaluate_aggregate`
   - *Depends on:* nothing (parser/binder/planner sudah support aggregate)
   - *Parallel with:* steps 1, 2, 3, 4

6. **Boolean & Utility operations** — `kuzu-function/src/scalar.rs`
   - `evaluate_boolean` — AND, OR, XOR, NOT sudah bisa? cek dan fix jika perlu
   - `evaluate_utility` — Coalesce, IfNull (COALESCE/IFNULL dari args), TypeOf
   - *Depends on:* nothing
   - *Parallel with:* steps 1-5

**Files modified:**
- `kuzu-core/kuzu-function/src/scalar.rs` — implementasi semua evaluator
- `kuzu-core/kuzu-processor/src/physical_operator.rs` — aggregate wiring

**Verification:**
1. Run existing tests: `cargo test -p kuzu-function`
2. Run integration tests: `cargo test` (all crates)
3. Test via CLI: jalankan query SELECT dengan string functions, date functions, aggregate

---

## Fase 2: Expression Evaluator & Physical Operators (MEDIUM)
**Tujuan:** Membuat pipeline eksekusi query benar-benar memproses data dari storage (bukan dummy data).

### Steps

1. **Create proper ExpressionEvaluator** — `kuzu-processor/src/expression_evaluator.rs` (file baru)
   - Saat ini `PhysicalFilter::evaluate_expression` hanya handle `Variable`, `Constant`, `BinaryOp`, `UnaryOp` secara ad-hoc
   - Buat `ExpressionEvaluator` struct yang recursively evaluate expression tree dengan memanggil `evaluate_scalar` dari `kuzu-function` untuk setiap function call node
   - Support: `Variable` (read from DataChunk), `Constant` (return literal), `BinaryOp` (dispatch ke arithmetic/comparison scalar), `UnaryOp` (dispatch ke NOT/negate), `FunctionCall` (dispatch ke registered function)
   - *Depends on:* Fase 1 (fungsi harus work dulu)
   - *Parallel with:* steps 2, 3

2. **Generalize PhysicalHashJoin** — `kuzu-processor/src/physical_operator.rs`
   - Saat ini hanya Int64, generalize ke semua tipe Value
   - Gunakan `Value` equality/comparison dari `kuzu-function` scalar evaluator
   - *Depends on:* nothing technically, tapi idealnya after Fase 1
   - *Parallel with:* step 1

3. **Generalize PhysicalOrderBy** — `kuzu-processor/src/physical_operator.rs`
   - Saat ini hanya Int64 sort column, generalize ke semua tipe
   - Dukungan multiple sort keys (saat ini hanya `sort_column: u32`)
   - Gunakan `ScalarFunction::Comparison` untuk ordering
   - *Depends on:* nothing
   - *Parallel with:* step 1, 2

4. **Generalize PhysicalAggregate** — `kuzu-processor/src/physical_operator.rs`
   - Saat ini hanya Int64, generalize ke semua tipe Value
   - Hash-based GROUP BY dengan Value key (bukan hardcoded Int64)
   - NULL handling untuk aggregate functions
   - *Depends on:* Fase 1 step 5 (aggregate evaluation)
   - *Parallel with:* steps 1, 2, 3 (if dep chain allows)

5. **Make PhysicalScan read from NodeTable/RelTable** — `kuzu-processor/src/physical_operator.rs`
   - Saat ini `PhysicalScan` generate dummy data: `col_id * 1000 + i`
   - Ubah untuk menerima `&NodeTable` / `&RelTable` reference dan membaca data sungguhan
   - `NodeTable` saat ini punya `nodes: Vec<HashMap<String, Value>>` — baca dari situ
   - *Depends on:* nothing — data sudah ada di NodeTable in-memory
   - *Parallel with:* steps 1-4

**Files modified:**
- `kuzu-core/kuzu-processor/src/physical_operator.rs` — semua operator
- `kuzu-core/kuzu-processor/src/expression_evaluator.rs` — file baru
- `kuzu-core/kuzu-processor/src/lib.rs` — register module baru
- `kuzu-core/kuzu-processor/src/processor.rs` — pass storage references ke PhysicalScan
- `kuzu-core/kuzu-main/src/connection.rs` — pass storage context ke processor

**Verification:**
1. `cargo test` — all tests must pass
2. Test via CLI: `MATCH (n:Person) RETURN n.name WHERE n.age > 25` — harus return data beneran

---

## Fase 3: Parser & Binder Expansion (MEDIUM)
**Tujuan:** Menambah coverage Cypher language yang didukung parser dan binder.

### Steps

1. **Add `OPTIONAL MATCH` grammar** — `kuzu-parser/src/cypher.pest`
   - `optional_match → { "OPTIONAL" ~ "MATCH" ~ pattern ~ where_clause? }`
   - `matching_clause → { match_clause | optional_match }` (update)
   - Parse jadi `AstNode::OptionalMatch { pattern, where_expr }`
   - *Depends on:* nothing
   - *Parallel with:* steps 2, 3, 4

2. **Add `WITH` clause grammar** — `kuzu-parser/src/cypher.pest`
   - `with_clause → { "WITH" ~ return_item ~ ("," ~ return_item)* ~ order_by? ~ limit_clause? }`
   - `reading_clause → { match_clause | optional_match | with_clause }` (update)
   - Parse jadi `AstNode::WithClause { items, order_by, limit }`
   - *Depends on:* nothing
   - *Parallel with:* steps 1, 3, 4

3. **Add `DELETE` and `SET` grammar** — `kuzu-parser/src/cypher.pest`
   - `delete_clause → { "DELETE" ~ expression ("," ~ expression)* }`
   - `set_clause → { "SET" ~ property_expression ~ "=" ~ expression ("," ~ property_expression ~ "=" ~ expression)* }`
   - `updating_clause → { create_clause | delete_clause | set_clause }`
   - *Depends on:* nothing
   - *Parallel with:* steps 1, 2, 4

4. **Add `UNION` grammar** — `kuzu-parser/src/cypher.pest`
   - `union → { "UNION" ~ "ALL"? }` (support UNION ALL)
   - Modify `query_statement` to allow `query ~ union ~ query`
   - *Depends on:* nothing
   - *Parallel with:* steps 1, 2, 3

5. **Binder for new clauses** — `kuzu-binder/src/binder.rs`
   - `bind_optional_match(pattern, where_expr)` — sama seperti match tapi produces `OptionalMatch`
   - `bind_with_clause(items, order_by, limit)` — creates projection scope
   - `bind_delete(exprs)` — validasi node/rel yang akan di-delete
   - `bind_set(assignments)` — validasi property assignments
   - `bind_union(left, right, all)` — validasi column compatibility
   - *Depends on:* steps 1-4 (parser)
   - *Parallel with:* N/A — sequential

6. **Planner for new clauses** — `kuzu-planner/src/planner.rs`
   - Add logical operator types untuk DELETE, SET, UNION
   - Map ke physical operators (atau direct implementation)
   - *Depends on:* step 5
   - *Parallel with:* N/A

7. **Processor for new operations** — `kuzu-processor/src/physical_operator.rs`
   - Add `PhysicalDelete` — remove nodes/rels dari table in-memory
   - Add `PhysicalSet` — update properties di table in-memory
   - Add `PhysicalUnion` — concatenate result chunks
   - *Depends on:* step 6
   - *Parallel with:* N/A

**Files modified:**
- `kuzu-core/kuzu-parser/src/cypher.pest` — grammar rules
- `kuzu-core/kuzu-parser/src/ast.rs` — AstNode variants
- `kuzu-core/kuzu-parser/src/parser.rs` — parse functions
- `kuzu-core/kuzu-binder/src/binder.rs` — bind functions
- `kuzu-core/kuzu-binder/src/bound_statement.rs` — BoundStatement variants
- `kuzu-core/kuzu-planner/src/planner.rs` — logical plan mappings
- `kuzu-core/kuzu-planner/src/logical_operator.rs` — LogicalOperator variants
- `kuzu-core/kuzu-processor/src/physical_operator.rs` — PhysicalDelete, PhysicalSet, PhysicalUnion
- `kuzu-core/kuzu-processor/src/processor.rs` — operator mapping

**Verification:**
1. `cargo test` — parser tests, binder tests
2. Test via CLI: `OPTIONAL MATCH (n) RETURN n`, `MATCH (n) WITH n.name AS name RETURN name`

---

## Fase 4: Storage Engine — In-Memory to On-Disk (HARD)
**Tujuan:** Membuat storage engine benar-benar persistent dengan columnar storage, buffer manager integration, WAL, dan checkpoint.

### Steps

1. **Implement Columnar Storage** — `kuzu-storage/src/column.rs` (file baru)
   - Buat `Column` struct — menyimpan data untuk satu kolom dalam page-based format
   - Buat `ColumnChunk` — subset dari column data (semua nilai untuk sekelompok node)
   - Implement `append_value`, `scan_values`, `get_value` via BufferManager pages
   - Mulai dengan fixed-size types (Int64, Double, Bool), lalu String dengan overflow pages
   - *Depends on:* BufferManager (existing)
   - *Parallel with:* steps 2, 3

2. **Implement NodeGroup & ChunkedNodeGroup** — `kuzu-storage/src/node_group.rs` (file baru)
   - `NodeGroup` — collection of NodeTable data untuk sekelompok node (misal 2048 rows)
   - `ChunkedNodeGroup` — subset kolom dari NodeGroup
   - Integrasi dengan Column untuk read/write
   - *Depends on:* step 1
   - *Parallel with:* N/A

3. **Implement CSR for RelTable** — `kuzu-storage/src/csr.rs` (file baru)
   - `CSRNodeGroup` — CSR adjacency list dengan header + chunks
   - `CSRChunkedNodeGroup` — data chunk dalam format CSR
   - Support: forward/backward adjacency scan, property access via offset
   - *Depends on:* step 1 (Column untuk property storage)
   - *Parallel with:* N/A — sequential with step 2

4. **On-disk HashIndex** — `kuzu-storage/src/index.rs`
   - Ubah `HashIndex<K, u64>` dari in-memory `HashMap` ke persistent paged hash index
   - Header page, slot pages, overflow pages via BufferManager
   - Implement `lookup`, `insert`, `delete`, `bulk_reserve`
   - *Depends on:* BufferManager

5. **Integrate WAL with write operations** — `kuzu-storage/src/wal.rs` + table writes
   - Saat insert/update/delete di NodeTable/RelTable, tulis WALRecord
   - Implement `WALReplayer` — replay WAL untuk recovery
   - Hubungkan dengan `TransactionManager` untuk commit/rollback records
   - *Depends on:* steps 2, 3, 4 (table writes perlu columnar storage)

6. **Implement real Checkpoint** — `kuzu-storage/src/checkpoint.rs`
   - Saat ini hanya `wal.clear()` — ubah untuk flush data pages ke disk
   - Iterasi semua NodeTable/RelTable, flush Column data ke file
   - Update metadata pages (catalog, stats)
   - *Depends on:* steps 2, 3, 4, 5

7. **LocalStorage integration** — `kuzu-storage/src/local_storage.rs`
   - Implement actual write transaction buffering
   - Hubungkan dengan UndoRecord / versioning untuk MVCC
   - Integrasi dengan TransactionManager
   - *Depends on:* steps 2, 3

**Files modified/new:**
- `kuzu-core/kuzu-storage/src/column.rs` — file baru
- `kuzu-core/kuzu-storage/src/node_group.rs` — file baru
- `kuzu-core/kuzu-storage/src/csr.rs` — file baru
- `kuzu-core/kuzu-storage/src/table.rs` — integrasi columnar, CSR, WAL
- `kuzu-core/kuzu-storage/src/index.rs` — on-disk HashIndex
- `kuzu-core/kuzu-storage/src/wal.rs` — WAL integration
- `kuzu-core/kuzu-storage/src/checkpoint.rs` — real checkpoint
- `kuzu-core/kuzu-storage/src/local_storage.rs` — MVCC buffering
- `kuzu-core/kuzu-storage/src/lib.rs` — register modules
- `kuzu-core/kuzu-main/src/database.rs` — Database init dengan StorageManager
- `kuzu-core/kuzu-transaction/src/` — integrasi dengan WAL/undo

**Verification:**
1. `cargo test` — storage tests
2. Test persistent: CREATE NODE TABLE → INSERT → COMMIT → restart → SELECT harus return data
3. Test WAL recovery: crash test → restart → data intact

---

## Fase 5: Optimizer & Planner Enhancement (MEDIUM)
**Tujuan:** Menambah optimization passes dan DP join ordering.

### Steps

1. **Add more optimization passes** — `kuzu-optimizer/src/passes.rs`
   - `CorrelatedSubqueryUnnest` — detect correlated subquery patterns dan unnest jadi joins
   - `AccHashJoin` — detect accumulate-then-hash-join patterns
   - `RemoveUnnecessaryJoin` — eliminate joins yang punya 1:1 cardinality
   - `AggKeyDependency` — optimize GROUP BY keys dengan functional dependencies
   - *Depends on:* existing optimizer framework
   - *Parallel with:* step 2

2. **Implement DPccp join ordering** — `kuzu-planner/src/join_order.rs`
   - Saat ini greedy heuristic — implementasikan dynamic programming dengan connected subgraph enumeration
   - `SubplansTable` — cache subplans by (nodes_set, relationship) → (best_plan, cardinality)
   - DPccp algorithm: enumerate connected subgraphs, find optimal join tree
   - Start dengan simple DP (non-ccp) dulu, upgrade ke full DPccp
   - *Depends on:* existing cardinality estimation
   - *Parallel with:* step 1

3. **SchemaPopulator pass** — `kuzu-optimizer/src/passes.rs`
   - Populate schema (column types, nullability) for each logical operator
   - Membantu optimization passes berikutnya yang butuh type info
   - *Depends on:* nothing (just reading from catalog)
   - *Parallel with:* steps 1, 2

**Files modified:**
- `kuzu-core/kuzu-optimizer/src/passes.rs` — optimization pass implementations
- `kuzu-core/kuzu-optimizer/src/optimizer.rs` — register passes
- `kuzu-core/kuzu-planner/src/join_order.rs` — DPccp algorithm
- `kuzu-core/kuzu-planner/src/planner.rs` — enhanced planning

**Verification:**
1. `cargo test` — optimizer tests
2. Performance test: join 4+ tables — query plan harus optimal (tables with smallest cardinality joined first)
3. Compare with C++ optimizer output via explain

---

## Fase 6: Extensions & Cypher Coverage (MIXED)
**Tujuan:** Membuat extension benar-benar berfungsi dan menambah Cypher statements yang tersisa.

### Steps

1. **Wire JSON extension** — `kuzu-json/src/lib.rs`
   - Helper functions (`json_extract_value`, `is_valid_json`, dll) sudah ada sebagai free functions
   - Ubah registration di `create_extension` untuk pakai `CustomScalar` dengan closure yang manggil helpers
   - Fix function mapping: ganti `UtilityOp::Coalesce` dan `StringOp::RegexMatches` yang salah
   - *Depends on:* Fase 1 (CustomScalar evaluation via evaluate_scalar)
   - *Parallel with:* steps 2-5

2. **Add COPY FROM (CSV) grammar** — `kuzu-parser/src/cypher.pest`
   - `copy_from → { "COPY" ~ table_name ~ "FROM" ~ string }`
   - *Depends on:* nothing
   - *Parallel with:* steps 3, 4, 5

3. **Add COPY FROM binder** — `kuzu-binder/src/binder.rs`
   - Resolve target table, validate column types untuk CSV data
   - *Depends on:* step 2
   - *Parallel with:* N/A

4. **Add COPY FROM execution** — `kuzu-processor/src/physical_operator.rs`
   - Parse CSV (or parquet) → insert rows ke NodeTable/RelTable
   - Gunakan crate `csv` sebagai dependency
   - *Depends on:* steps 2, 3, dan Fase 4 (storage buat insert)
   - *Parallel with:* N/A

5. **Add remaining Cypher statements** (CALL, EXPLAIN, ALTER, CREATE SEQUENCE, dll.)
   - Parsing: grammar rules + AST nodes
   - Binding: validation + catalog lookups
   - Execution: Direct implementation (no complex storage needed)
   - Prioritaskan per statement:
     - `EXPLAIN` — banget mudah, tinggal print query plan
     - `CALL` — table function invocation
     - `ALTER TABLE` — rename/add/drop column (catalog operation)
     - `CREATE SEQUENCE` — sequence counter
     - `MERGE` — match-or-create pattern (complex)
   - *Depends on:* parser/binder framework (existing)
   - *Parallel with:* steps 1-4 (independent)

6. **Extension implementations (JSON already done in step 1)**
   - `kuzu-fts` — implement inverted index + BM25 scoring (refer C++ FTS engine)
   - `kuzu-httpfs` — HTTP file reading via `reqwest` crate
   - `kuzu-vector` — vector similarity search (cosine, dot, euclidean)
   - `kuzu-algo` — wire graph algorithms (BFS, PageRank, WCC, SCC) sebagai table functions
   - Other extensions — basic functional implementations
   - *Depends on:* Fase 1 (function evaluation), Fase 2 (expression evaluator)
   - *Parallel with:* each other (extensions are independent)

**Files modified:**
- `kuzu-core/kuzu-json/src/lib.rs` — wire helper functions
- `kuzu-core/kuzu-parser/src/cypher.pest` — COPY, CALL, EXPLAIN, ALTER grammar
- `kuzu-core/kuzu-parser/src/ast.rs` — AST node variants
- `kuzu-core/kuzu-parser/src/parser.rs` — parse functions
- `kuzu-core/kuzu-binder/src/binder.rs` — bind functions
- `kuzu-core/kuzu-processor/src/physical_operator.rs` — COPY executor
- `kuzu-core/kuzu-fts/src/` — full-text search engine
- `kuzu-core/kuzu-httpfs/src/` — HTTP file system
- `kuzu-core/kuzu-vector/src/` — vector similarity
- `kuzu-core/kuzu-algo/src/lib.rs` — wire GDS algorithms
- Each extension crate's `src/lib.rs`

**Verification:**
1. `cargo test` — extension tests
2. Test JSON: `RETURN json_extract('{"a":1}', '$.a')` → harus return 1
3. Test COPY FROM: `COPY person FROM 'data.csv'`
4. Test EXPLAIN: `EXPLAIN MATCH (n) RETURN n`
5. Test FTS: create index, search query
6. Test GDS: `CALL page_rank(...)`

---

## Ringkasan Prioritas & Estimasi

| Fase | Area | Estimasi | Dependensi |
|------|------|----------|------------|
| **Fase 1** | Functions & Evaluator | 2-3 hari | None |
| **Fase 2** | Expression Evaluator & Operators | 3-5 hari | Fase 1 (ringan) |
| **Fase 3** | Parser & Binder Expansion | 3-5 hari | None (independen) |
| **Fase 4** | Storage Engine | 2-3 minggu | None (dapat dimulai paralel dgn Fase 1-3) |
| **Fase 5** | Optimizer & Planner | 3-5 hari | None (independen) |
| **Fase 6** | Extensions & Cypher Coverage | 1-2 minggu | Fase 1 (ringan), Fase 4 (COPY FROM) |

**Rekomendasi jalur paralel:**
```
Tim A: Fase 1 → Fase 2 → (lanjut Fase 6)
Tim B: Fase 3 → (lanjut Fase 5)
Tim C: Fase 4 → Fase 6 (storage-dependent parts)
```

## Yang TIDAK termasuk (scope luar)
- C API / Arrow integration — butuh stable API surface
- Full multi-threaded execution (morsel-based parallelism) — butuh major refactor
- FactorizedTable result collection — butuh redesign result model
- Full MVCC with version chains — butuh versioning infrastructure
- All 40+ C++ physical operators — prioritas pada yang paling critical

## Referensi C++
- `src/include/processor/operator/` — semua physical operator C++
- `src/include/storage/store/` — storage engine C++
- `src/include/planner/operator/logical_operator.h` — logical operators
- `src/include/function/` — built-in function implementations
- `src/antlr4/Cypher.g4` — full Cypher grammar
