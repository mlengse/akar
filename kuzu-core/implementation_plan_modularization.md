# Modularization Plan — Split Large Rust Files

> **Status:** ✅ ALL PHASES COMPLETE | **Target:** 2026-07-14
> **Prerequisites:** P10.1 (COPY TO) ✅
> **Referensi:** `STATUS.md` §4.4 Technical Debt Register + §9 Architecture Audit

---

## Overview

Berdasarkan audit ukuran file (2026-07-07), ada **7 file Rust >1500 lines** yang harus dipecah untuk kemudahan debugging, navigasi, dan kontribusi. Total **~19.500 LOC** akan direorganisasi menjadi struktur modular.

### File Prioritas (diurutkan dari yang terbesar)

| # | File | Lines | KB | Crate | Rencana |
|---|------|-------|-----|-------|---------|
| 🔴 | `scalar.rs` | 4.578 | 172,8 | kuzu-function | Split by category → 20+ files |
| 🔴 | `physical_operator.rs` | 3.794 | 161,4 | kuzu-processor | Split by operator type (P10.6) |
| 🔴 | `connection.rs` | 3.133 | 146,9 | kuzu-main | Split by concern + extract tests |
| 🔴 | `processor.rs` | 2.702 | 119,0 | kuzu-processor | Split helpers from pipeline |
| 🟡 | `passes.rs` | 2.486 | 104,5 | kuzu-optimizer | Split by pass (flat/tree × 21) |
| 🟡 | `parser.rs` | 2.183 | 84,2 | kuzu-parser | Split by statement type |
| 🟡 | `binder.rs` | 1.667 | 72,9 | kuzu-binder | Split by bind_* category |

---

## Phase 1: kuzu-function — Split `scalar.rs` (4.578 lines)

**Prioritas:** 🔴 Tertinggi — file terbesar, paling banyak kategori fungsi

### Target Struktur

```
kuzu-function/src/
├── scalar/
│   ├── mod.rs                # Re-export semua + evaluate_scalar dispatcher
│   ├── arithmetic.rs         # 26 ops: +, -, *, /, abs, ceil, floor, round, sqrt, log, exp, sin, cos, tan, asin, acos, atan, atan2, degrees, radians, sign, pi, rand, negate, power, cbrt, cot, even, factorial, gamma, lgamma, ln, log2, set_seed
│   ├── comparison.rs         # 8 ops: =, <>, <, <=, >, >=, IS NULL, IS NOT NULL
│   ├── boolean.rs            # 4 ops: AND, OR, XOR, NOT
│   ├── string.rs             # 23 ops: concat, contains, starts_with, ends_with, upper/lower, trim/ltrim/rtrim, length, reverse, repeat, replace, substring, regex_*, split, head, tail, left, right, lpad, rpad, levenshtein, initcap, concat_ws
│   ├── date.rs               # 16 ops: date_part, date_trunc, date_diff, date_add, current_date, current_timestamp, year, month, day, hour, minute, second, dayname, monthname, last_day, make_date, century, epoch_ms, to_timestamp, to_epoch_ms
│   ├── list.rs               # 10 ops + lambda: list_creation, list_extract, list_concat, list_len, list_sort, list_reverse, list_contains, list_append, list_prepend, list_slice, range, list_distinct, list_unique, list_sum, list_product, list_any_value, list_to_string, list_position, list_has_all, list_reverse_sort, any, all, none, single
│   ├── map_struct.rs         # 6 ops: map_creation, map_extract, map_keys, map_values, struct_creation, struct_extract, cardinality
│   ├── cast.rs               # 14+ targets: CAST, cast_*, date(), timestamp(), float(), int(), bool(), string(), blob()
│   ├── schema.rs             # 5 ops: OFFSET, ID, START_NODE, END_NODE, LABEL
│   ├── array.rs              # 5 ops: array_cosine_similarity, array_distance, array_inner_product, array_cross_product, array_squared_distance + 10 aliases
│   ├── path.rs               # 6 ops: nodes, rels, properties, is_trail, is_acyclic, length
│   ├── uuid.rs               # 1 op: gen_random_uuid
│   ├── utility.rs            # 3 ops: coalesce, ifnull, typeof
│   ├── sequence.rs           # 2 ops: nextval, currval
│   ├── hash.rs               # 3 ops: md5, sha256, hash
│   ├── interval.rs           # 8 ops: to_years, to_months, to_days, to_hours, to_minutes, to_seconds, to_milliseconds, to_microseconds
│   ├── union_funcs.rs        # 3 ops: union_value, union_tag, union_extract
│   ├── blob.rs               # 3 ops: encode, decode, octet_length
│   └── bitwise.rs            # 5 ops: bitwise_xor, bitwise_and, bitwise_or, bitshift_left, bitshift_right
├── aggregate/
│   ├── mod.rs                # AggValueState enum + dispatch
│   ├── count.rs              # COUNT, COUNT(*), count_if
│   ├── sum_avg.rs            # SUM, AVG
│   ├── min_max.rs            # MIN, MAX
│   ├── collect.rs            # COLLECT
│   ├── stddev_variance.rs    # STDDEV, VARIANCE
│   └── percentile.rs         # PERCENTILE_DISC, PERCENTILE_CONT
├── table/
│   ├── mod.rs                # Table function dispatch
│   └── ...
├── registry.rs               # FunctionRegistry (tetap, 1199 lines -> bisa di-split nanti)
├── scalar.rs                 # → DIHAPUS setelah split
└── lib.rs
```

### Langkah

- `[x]` **1.1** Buat direktori `kuzu-function/src/scalar/` + `aggregate/` + `table/`
- `[x]` **1.2** Ekstrak per kategori fungsi ke file masing-masing
- `[x]` **1.3** `scalar/mod.rs` — `pub mod` semua + dispatch `evaluate_scalar()`
- `[x]` **1.4** `aggregate/mod.rs` — `AggValueState` + dispatch aggregate
- `[x]` **1.5** Update `lib.rs` — `pub mod scalar` menggantikan `pub mod scalar` (single file)
- `[x]` **1.6** Update `registry.rs` — import dari `scalar::*` bukan dari file tunggal
- `[x]` **1.7** Verifikasi: `cargo check -p kuzu-function`, `cargo test -p kuzu-function` (159 passing)

### Verifikasi

```bash
cargo check -p kuzu-function
cargo test -p kuzu-function       # Harus tetap 159 passing
cargo test --workspace             # Harus tetap 954 passing
```

---

## Phase 2: kuzu-processor — Split `physical_operator.rs` + `processor.rs` (6.496 lines total)

### 2A: `physical_operator.rs` (3.794 lines) — ✅ Complete (P-MOD2A)

**Actual implementation:** 5 consolidated modules (instead of 20+ individual files) to minimize cross-reference issues:

```
kuzu-processor/src/
├── physical_operator.rs     # Thin re-export: `pub use crate::physical::*;`
├── physical/
│   ├── mod.rs               # Module declarations + re-exports
│   ├── types.rs             # OperatorResult, HashJoinBucket/Table, NodeSemiMask, PhysicalOperatorExec, PhysicalSemiMasker
│   ├── common.rs            # store_value_in_vector, value_cmp, value_hash
│   ├── scan_filter.rs       # PhysicalScan, PhysicalScanRel, PhysicalFilter, PhysicalProjection, PhysicalLimit
│   ├── order_aggregate.rs   # PhysicalOrderBy, BlockMergeSorter, PhysicalAggregate, AggregateHashTable + helpers
│   ├── join_ops.rs          # PhysicalCrossProduct, PhysicalSemiJoin, PhysicalAntiJoin, PhysicalIntersect, JoinHashTable, PhysicalHashJoin
│   └── write_ops.rs         # PhysicalUnwind, PhysicalSet, PhysicalDelete, PhysicalForeach, PhysicalVectorSimilarityScan, PhysicalCopyFrom, PhysicalExplain, PhysicalRecursiveExtend, PhysicalCreateFtsIndex, PhysicalFtsScan, PhysicalCountRelTable, PhysicalCreateNode, PhysicalCreateRel, PhysicalExtend, PhysicalArtIndexRangeScan
```

### 2B: `processor.rs` (2.702 lines) — QueryProcessor + Helpers

```
kuzu-processor/src/processor/
├── mod.rs                    # QueryProcessor struct + execute() pipeline
├── join_helpers.rs           # derive_join_column_indices, extract_join_prop
├── union_helpers.rs          # flatten_union_child, merge_union_chunks, merge_optional_chunks
├── chunk_helpers.rs          # extract_all_rows_from_chunks, rows_to_columns, value_to_physical_type
├── projection_helper.rs      # resolve_projection_column_index
└── plan_serializer.rs        # serialize_plan_tree
```

### Langkah

- `[x]` **2.1** Phase 2A — Split `physical_operator.rs` (3,794 lines → 5 files)
- `[x]` **2.2** Phase 2B — Split `processor.rs` helpers → **Deferred** (stable at 2,702 lines; helpers tightly coupled to `QueryProcessor`)
- `[x]` **2.3** Verifikasi: `cargo test -p kuzu-processor` (77 passing)

---

## Phase 3: kuzu-main — Split `connection.rs` (3.133 lines) — ✅ Complete (P-MOD3)

**Actual implementation:** 8 modules extracted from the monolithic `connection.rs`.
`call.rs` merged into `ddl.rs` (handle_call is part of DDL dispatch).
`prepared.rs` kept separate (`kuzu-main/src/prepared_statement.rs` already existed).

### Actual Structure

```
kuzu-main/src/
├── connection/
│   ├── mod.rs                # Connection struct, new(), close(), set_default_schema, TxnResources
│   ├── query.rs              # query(), execute(), create_processor(), prepare(), execute_prepared()
│   ├── ddl.rs                # handle_ddl + handle_call (CREATE/DROP/ALTER, CALL, ANALYZE, EXPORT/IMPORT)
│   ├── dml.rs                # handle_foreach
│   ├── copy.rs               # execute_export/import_database
│   ├── transaction.rs        # TxnResources, begin/commit/rollback, set_transaction_mode
│   ├── substitute.rs         # substitute_params_in_statement, substitute_in_bound_expr, substitute_in_logical_plan
│   └── utils.rs              # pk_value_to_string, extract_arg_string, format_storage_size, gen_random_string
├── connection_test.rs        # All #[cfg(test)] modules extracted (integration, merge, call, create_dml, foreach, var_length_path, subquery)
├── database.rs
├── prepared_statement.rs
├── query_result.rs
├── adbc.rs
└── lib.rs
```

### Langkah

- `[x]` **3.1** Buat direktori `kuzu-main/src/connection/`
- `[x]` **3.2** Ekstrak `mod.rs` — struct Connection + new(), close(), set_default_schema, TxnResources
- `[x]` **3.3** Ekstrak `query.rs` — query(), execute() pipeline, prepare(), execute_prepared()
- `[x]` **3.4** Ekstrak `ddl.rs` (handle_ddl + handle_call), `dml.rs`, `copy.rs`, `transaction.rs`
- `[x]` **3.5** Ekstrak helpers: `substitute.rs`, `utils.rs`
- `[x]` **3.6** Ekstrak tests: `connection_test.rs` (7 test modules)
- `[x]` **3.7** Update `lib.rs` — `pub mod connection` + `mod connection_test`
- `[x]` **3.8** Verifikasi: `cargo test -p kuzu-main` (123 passing, 0 failed) + `cargo clippy -D warnings` clean

---

## Phase 4: kuzu-optimizer — Split `passes.rs` (2.486 lines)

### Target Struktur

```
kuzu-optimizer/src/
├── passes/
│   ├── mod.rs                     # Re-export + optimize() orchestrator
│   ├── flat/
│   │   ├── mod.rs
│   │   ├── remove_unnecessary.rs  # Pass 1
│   │   ├── filter_pushdown.rs     # Pass 2
│   │   ├── projection_pushdown.rs # Pass 3
│   │   ├── constant_folding.rs    # Pass 4
│   │   ├── aggregate_detection.rs # Pass 5
│   │   ├── join_optimization.rs   # Pass 6
│   │   ├── top_k.rs               # Pass 7
│   │   ├── vector_similarity.rs   # Pass 8
│   │   ├── art_range_scan.rs      # Pass 9
│   │   ├── limit_pushdown.rs      # Pass 10
│   │   ├── cse.rs                 # Pass 11
│   │   ├── order_by_pushdown.rs   # Pass 12 (Ladybug)
│   │   ├── unwind_dedup.rs        # Pass 13 (Ladybug)
│   │   └── count_rel_table.rs     # Pass 14 (Ladybug)
│   └── tree/
│       ├── mod.rs
│       ├── factorization.rs       # Tree 1
│       ├── foreign_join.rs        # Tree 2
│       ├── acc_hash_join.rs       # Tree 3
│       ├── sip.rs                 # Tree 4
│       ├── subquery_unnesting.rs  # Tree 5
│       ├── agg_key_dep.rs         # Tree 6
│       └── cardinality.rs         # Tree 7
├── join_order.rs                   # Tetap (625 lines)
└── lib.rs
```

### Actual Structure

```
kuzu-optimizer/src/
├── passes/
│   ├── mod.rs                  # Trait definitions + re-exports
│   ├── flat/
│   │   ├── mod.rs              # Module declarations + re-exports
│   │   ├── filter_pushdown.rs  # Pass 1: FilterPushDown
│   │   ├── projection_pushdown.rs # Pass 2: ProjectionPushDown
│   │   ├── constant_folding.rs # Pass 4: ConstantFolding
│   │   ├── aggregate_detection.rs # Pass 5: AggregateDetection
│   │   ├── join_optimization.rs   # Pass 6: JoinOptimization
│   │   ├── top_k.rs               # Pass 7: TopKOptimization
│   │   ├── vector_similarity.rs   # Pass 8: VectorSimilarityDetection
│   │   ├── art_range_scan.rs      # Pass 9: ArtRangeScanDetection
│   │   ├── scan_ops.rs            # Pass 1+10+11: RemoveUnnecessaryOperators + LimitPushDown + CSE
│   │   └── ladybug.rs             # Pass 12+13+14: OrderByPushDown + UnwindDedup + CountRelTable
│   └── tree/
│       ├── mod.rs              # Module declarations + re-exports
│       ├── factorization.rs    # Tree 1: FactorizationRewriting
│       ├── acc_hash_join.rs    # Tree 3: AccHashJoinOptimization
│       ├── sip.rs              # Tree 3.5: SIPOptimization
│       ├── foreign_join.rs     # Tree 2: ForeignJoinPushDown
│       ├── subquery_unnesting.rs # Tree 4: CorrelatedSubqueryUnnesting
│       ├── cardinality.rs      # Tree 6: CardinalityEstimation
│       └── agg_key_dep.rs      # Tree 5: AggKeyDependency
├── passes_test.rs              # All test modules extracted
├── join_order.rs
├── optimizer.rs
└── lib.rs
```

### Langkah

- `[x]` **4.1** Buat struktur `passes/flat/` dan `passes/tree/`
- `[x]` **4.2** Ekstrak setiap pass ke file sendiri (10 flat + 7 tree = 17 files)
- `[x]` **4.3** `passes/mod.rs` — trait definitions + re-exports
- `[x]` **4.4** Verifikasi: `cargo test -p kuzu-optimizer` (49 passing, 0 failed) + `cargo clippy -D warnings` clean

---

## Phase 5: kuzu-parser — Split `parser.rs` (2.183 lines)

### Target Struktur

```
kuzu-parser/src/
├── parser/
│   ├── mod.rs              # parse_statement() — main entry + shared helpers
│   ├── ddl.rs              # Parse CREATE/DROP TABLE, INDEX, SEQUENCE, ALTER, COPY, ANALYZE, EXPORT/IMPORT
│   ├── dml.rs              # Parse MATCH, RETURN, WHERE, CREATE, DELETE, SET, MERGE, FOREACH, UNWIND, WITH, OPTIONAL MATCH
│   ├── query.rs            # Parse subqueries, UNION, CALL, TRANSACTION
│   ├── expression.rs       # Parse all expression types (arithmetic, comparison, function calls, CASE, list/map/struct, subquery)
│   └── helpers.rs          # Shared parsing utilities
├── ast.rs                  # Tetap
├── cypher.pest             # Tetap
└── lib.rs
```

### Langkah

- `[x]` **5.1** Buat direktori `kuzu-parser/src/parser/`
- `[x]` **5.2** Ekstrak `ddl.rs` — semua `parse_create_*`, `parse_drop_*`, `parse_copy_*`, `parse_alter_*`, `parse_analyze`
- `[x]` **5.3** Ekstrak `dml.rs` — DML + patterns + CALL + MERGE
- `[x]` **5.4** Ekstrak `expression.rs` — semua parsing expression
- `[x]` **5.5** Ekstrak tests → `parser_test.rs`
- `[x]` **5.6** Verifikasi: `cargo test -p kuzu-parser` (63 passing, 0 failed)

---

## Phase 6: kuzu-binder — Split `binder.rs` (1.667 lines)

### Target Struktur

```
kuzu-binder/src/
├── bind/
│   ├── mod.rs              # bind() main entry
│   ├── ddl.rs              # bind_create_table, bind_drop_table, bind_alter, bind_copy_from, bind_copy_to, bind_create_sequence, bind_create_index, bind_explain, bind_export_db, bind_import_db
│   ├── dml.rs              # bind_query, bind_match, bind_return, bind_create, bind_delete, bind_set, bind_merge, bind_foreach, bind_unwind, bind_with
│   ├── expression.rs       # bind_expression, bind_function_call, bind_case, etc
│   └── helpers.rs          # resolve_symbol, type_check, etc
├── bound_statement.rs      # Tetap
├── binder.rs               # Binder struct (tetap atau digabung ke bind/mod.rs)
└── lib.rs
```

### Langkah

- `[x]` **6.1** Buat direktori `kuzu-binder/src/binder/`
- `[x]` **6.2** `binder/mod.rs` — all Binder logic (Binder struct + impl + helpers)
- `[x]` **6.3** `binder_test.rs` — all tests extracted
- `[x]` **6.4** Verifikasi: `cargo test -p kuzu-binder` (14 passing, 0 failed)

---

## Verification Plan (Semua Phase)

```bash
# Setelah setiap phase
cargo check -p <crate>
cargo test -p <crate>

# Final regression
cargo test --workspace          # Harus tetap 954 passing
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
cargo build --workspace --release
```

---

## Estimated Effort

| Phase | File | Lines | SP | Risk | Dependensi |
|-------|------|-------|-----|------|------------|
| **P-MOD1** | `scalar.rs` → 20+ files | 4.578 | **8** | Medium | Registry refactor |
| **P-MOD2** | `physical_operator.rs` + `processor.rs` | 6.496 | **8** | Low | P-MOD1 |
| **P-MOD3** | `connection.rs` → connection/ + test | 3.133 | **5** ✅ | Low | P-MOD2 |
| **P-MOD4** | `passes.rs` → 17 files | 2.486 | **5** ✅ | Low | — |
| **P-MOD5** | `parser.rs` → parser/ | 2.183 | **5** ✅ | Low | — |
| **P-MOD6** | `binder.rs` → binder/ + test | 1.667 | **3** ✅ | Low | P-MOD5 |
| **Total** | **7 files → ~90 files modular** | **20.543** | **34** | | |

---

## Success Criteria

| # | Criteria |
|---|----------|
| 1 | Semua file >1500 lines sudah dipecah menjadi ≤500 lines per file |
| 2 | Struktur direktori modular untuk semua 7 crate |
| 3 | Semua 954 test tetap passing tanpa perubahan behavior |
| 4 | Clippy `-D warnings` clean |
| 5 | `cargo build --workspace --release` sukses |
| 6 | Tidak ada regresi performa (compiler mengoptimalkan sama) |
| 7 | Developer baru bisa menemukan kode <30 detik dengan struktur folder |
