# Plan: Kuzu Rust Fase P3/P4/P5 — Physical Ops, Ladybug Features, Quality

**TL;DR:** P0 (Crash Recovery), P1 (Table Functions 12/31), P2 (Transaction Enhancement) sudah selesai. Fokus selanjutnya: memperdalam physical operators (AggregateHashTable, JoinHashTable, External Sort), porting Ladybug features (ANALYZE, PERCENTILE, 3 optimizer passes), remaining table functions, dan quality (CI/CD, column specializations).

---

## Fase P3: Physical Operator Completeness (26-36 jam)

**Steps**

### P3.1 — ANALYZE Statement (4-5 jam)
1. Grammar: `analyze_statement` rule di `cypher.pest`
2. AST: `Statement::Analyze(AnalyzeStatement)` di `ast.rs`
3. Parser: `parse_analyze()` di `parser.rs`
4. Binder: `BoundAnalyze` di `bound_statement.rs`, `bind_analyze()` di `binder.rs`
5. Processor: scan table columns, compute stats (row count, distinct, null ratio), store ke `StatsStore`
6. Tests: 3 parser + 3 integration

### P3.2 — AggregateHashTable (5-7 jam)
*Depends on: none. Parallel with P3.1*
1. New struct: `AggregateHashTable` dengan partition-level parallelism di `physical_operator.rs`
2. Thread-local aggregation via `rayon`, merge partitions
3. Ganti `HashMap<Vec<Value>, AggValueState>` dengan hash table dedicated
4. Reference: `src/processor/operator/aggregate/aggregate_hash_table.cpp`
5. Tests: 5 unit tests (single group, multi group, parallel, empty input, large dataset)

### P3.3 — Partitioned JoinHashTable (5-7 jam)
*Depends on: none. Parallel with P3.1, P3.2*
1. New struct: `JoinHashTable` dengan partition-based parallelism di `physical_operator.rs`
2. Thread-local hash table build, parallel probe
3. Ganti single-level hash join sekarang
4. Reference: `src/processor/operator/hash_join/join_hash_table.cpp`
5. Tests: 5 unit tests

### P3.4 — External Sort (RadixSort) (5-7 jam)
*Depends on: none*
1. `RadixSort` untuk integer keys, `KeyBlockMerger` untuk merge sorted blocks
2. Fallback ke comparison sort untuk string/variable-length types
3. `PhysicalOrderBy` gunakan external sort jika data > memory threshold
4. Reference: `src/processor/operator/order_by/`
5. Tests: 4 unit tests

### P3.5 — NodeBatchInsert + RelBatchInsert (5-7 jam)
*Depends on: P0 (StorageManager)*
1. `PhysicalNodeBatchInsert` — bulk insert rows ke NodeTable
2. `PhysicalRelBatchInsert` — bulk insert edges ke RelTable + CSR adjacency
3. Wiring ke `PhysicalCopyFrom` untuk pipeline lengkap
4. Tests: 3 integration tests

### P3.6 — PERCENTILE_DISC/CONT (2-3 jam)
*Depends on: none. Parallel with all P3*
1. `AggregateFunction::PercentileDisc` + `PercentileCont` di `registry.rs`
2. Implementation di `scalar.rs`: collect semua values, sort, pick percentile
3. Tests: 3 unit tests

**Relevant files**
- `kuzu-parser/src/cypher.pest` — grammar additions (P3.1)
- `kuzu-parser/src/ast.rs` — AnalyzeStatement (P3.1)
- `kuzu-parser/src/parser.rs` — parse_analyze (P3.1)
- `kuzu-binder/src/bound_statement.rs` — BoundAnalyze (P3.1)
- `kuzu-binder/src/binder.rs` — bind_analyze (P3.1)
- `kuzu-processor/src/physical_operator.rs` — AggregateHashTable, JoinHashTable, RadixSort, BatchInsert (P3.2-P3.5)
- `kuzu-processor/src/processor.rs` — dispatches (P3.1-P3.5)
- `kuzu-function/src/registry.rs` — PERCENTILE registration (P3.6)
- `kuzu-function/src/scalar.rs` — PERCENTILE implementation (P3.6)

**Verification**
1. `cargo test -p kuzu-parser` — ANALYZE parsing tests
2. `cargo test -p kuzu-processor` — aggregate hash table + join hash table tests
3. `cargo test -p kuzu-function` — PERCENTILE tests
4. `cargo test --workspace` — no regressions
5. Integration: `ANALYZE Person; CALL stats_info('Person');` returns updated stats

---

## Fase P4: Ladybug-Specific Features (12-17 jam)

**Steps**

### P4.1 — OrderByPushDown optimizer (2-3 jam)
1. New pass di `kuzu-optimizer/src/passes.rs`: `OrderByPushDown`
2. Push ORDER BY ke bawah melewati UNION, JOIN (jika preserving order)
3. Register di `optimizer.rs` sebagai flat pass #12
4. Reference: `ladybug/src/optimizer/order_by_push_down_optimizer.cpp`

### P4.2 — UnwindDedup optimizer + physical (2-3 jam)
1. New logical: `LogicalUnwindDedup` di `logical_operator.rs`
2. New physical: `PhysicalUnwindDedup` di `physical_operator.rs`
3. New pass: `UnwindDedup` di `passes.rs`
4. Reference: `ladybug/src/optimizer/unwind_dedup_optimizer.cpp`

### P4.3 — CountRelTable optimizer + physical (2-3 jam)
1. New logical: `LogicalCountRelTable` di `logical_operator.rs`
2. New physical: `PhysicalCountRelTable` di `physical_operator.rs`
3. New pass: `CountRelTable` — deteksi `COUNT(*)` pada rel table, ganti scan dengan CSR metadata lookup
4. Reference: `ladybug/src/optimizer/count_rel_table_optimizer.cpp`

### P4.4 — GRAPH statement (3-4 jam)
*Parallel with P4.1-P4.3*
1. Grammar: `graph_statement` rule
2. AST: `Statement::Graph` + `GraphStatement`
3. Parser: `parse_graph_statement()`
4. Binder: `BoundGraph` + `bind_graph()`
5. Planner → Processor (project graph management)
6. Reference: `ladybug/src/binder/bind/bind_graph.cpp`

### P4.5 — Project graph CALL functions (3-4 jam)
1. `CALL project_cypher_graph(name, query)` — project graph via Cypher
2. `CALL project_native_graph(name, node_tables, rel_tables)` — project from tables
3. `CALL drop_project_graph(name)`
4. `CALL show_projected_graphs()` / `projected_graph_info()`

**Relevant files**
- `kuzu-optimizer/src/passes.rs` — 3 new passes (P4.1-P4.3)
- `kuzu-optimizer/src/optimizer.rs` — register passes (P4.1-P4.3)
- `kuzu-planner/src/logical_operator.rs` — new logical operators (P4.2-P4.4)
- `kuzu-processor/src/physical_operator.rs` — new physical operators (P4.2-P4.3)
- `kuzu-processor/src/processor.rs` — dispatches (P4.2-P4.4)
- `kuzu-parser/src/cypher.pest` — GRAPH grammar (P4.4)
- `kuzu-parser/src/ast.rs` — GRAPH AST (P4.4)
- `kuzu-main/src/connection.rs` — CALL functions (P4.5)

**Verification**
1. `cargo test -p kuzu-optimizer` — 3 new passes
2. `cargo test -p kuzu-parser` — GRAPH parsing
3. `cargo test --workspace` — no regressions

---

## Fase P5: Remaining Table Functions (7-8 jam)

**Steps**
1. `bm_info()` — query BufferManager memory stats (1 jam)
2. `file_info()` — query FileHandle page stats (1 jam)
3. `free_space_info()` — query FSM stats (1 jam)
4. `show_loaded_extensions()` + `show_official_extensions()` — extension registry (1.5 jam)
5. `show_projected_graphs()` + `projected_graph_info()` — graph catalog (1 jam)
6. `clear_warnings()` + `show_warnings()` — WarningContext (1 jam)
7. `disk_size_info()` + `storage_version()` — system info (0.5 jam)

**Relevant files**
- `kuzu-main/src/connection.rs` — 8 new handle_call functions
- `kuzu-storage/src/buffer_manager.rs` — expose stats API (bm_info)
- `kuzu-storage/src/page.rs` — expose page stats (file_info)
- `kuzu-storage/src/free_space_manager.rs` — expose stats (free_space_info)
- `kuzu-extension/src/lib.rs` — expose loaded extensions

---

## Fase P6: Quality & Infrastructure (20-28 jam)

**Steps**
1. CI/CD: GitHub Actions workflow (build, test, clippy, fmt) (3-4 jam)
2. Storage column specializations: StringColumn (dictionary), ListColumn, StructColumn, NullColumn (6-8 jam)
3. PlanPrinter: pretty EXPLAIN output (2-3 jam)
4. kuzu-httpfs completion: full HTTP/S3 support (3-4 jam)
5. kuzu-fts completion: full-text indexing engine (4-6 jam)
6. Wasm stabilization: fix wasm32 issues, CI check (2-3 jam)

---

## Decisions
- AggregateHashTable dan JoinHashTable menggunakan `rayon` untuk parallelism (konsisten dengan GDS framework)
- ANALYZE mengumpulkan basic stats (row count, null count, min/max) — HyperLogLog distinct count deferred ke P6
- External sort sebagai fallback opsional; in-memory sort tetap default
- GRAPH statement: fokus pada project graph management (project_cypher_graph, project_native_graph, drop)
- Ladybug optimizer passes: 3 pass terpenting (OrderByPushDown, UnwindDedup, CountRelTable)

## Further Considerations
1. **AggregateHashTable vs JoinHashTable**: Keduanya bisa dikerjakan paralel karena berbagi pola desain (partition-based hash table) tapi tidak sharing code langsung.
2. **External Sort priority**: Jika benchmark menunjukkan in-memory sort cukup untuk dataset tipikal, bisa di-demote ke P6.
3. **Batch Insert**: Bergantung pada StorageManager yang sudah ada dari P0 — pastikan `NodeTable::insert_row()` dan `RelTable::insert_edge()` sudah berfungsi benar sebelum implement batch insert.
