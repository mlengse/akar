# Modularization Plan — Split Large Rust Files

> **Status:** Not Started | **Target:** 2026-07-14
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

- `[ ]` **1.1** Buat direktori `kuzu-function/src/scalar/` + `aggregate/` + `table/`
- `[ ]` **1.2** Ekstrak per kategori fungsi ke file masing-masing
- `[ ]` **1.3** `scalar/mod.rs` — `pub mod` semua + dispatch `evaluate_scalar()`
- `[ ]` **1.4** `aggregate/mod.rs` — `AggValueState` + dispatch aggregate
- `[ ]` **1.5** Update `lib.rs` — `pub mod scalar` menggantikan `pub mod scalar` (single file)
- `[ ]` **1.6** Update `registry.rs` — import dari `scalar::*` bukan dari file tunggal
- `[ ]` **1.7** Verifikasi: `cargo check -p kuzu-function`, `cargo test -p kuzu-function` (159 passing)

### Verifikasi

```bash
cargo check -p kuzu-function
cargo test -p kuzu-function       # Harus tetap 159 passing
cargo test --workspace             # Harus tetap 954 passing
```

---

## Phase 2: kuzu-processor — Split `physical_operator.rs` + `processor.rs` (6.496 lines total)

### 2A: `physical_operator.rs` (3.794 lines) — P10.6

Lihat `implementation_plan_p10.md` §P10.6 untuk detail lengkap.

```
kuzu-processor/src/
├── operators/
│   ├── mod.rs
│   ├── scan.rs              # PhysicalScan, PhysicalScanRel
│   ├── filter.rs            # PhysicalFilter
│   ├── projection.rs        # PhysicalProjection
│   ├── hash_join.rs         # PhysicalHashJoin + JoinHashTable
│   ├── cross_product.rs     # PhysicalCrossProduct
│   ├── intersect.rs         # PhysicalIntersect
│   ├── semi_join.rs         # PhysicalSemiJoin
│   ├── anti_join.rs         # PhysicalAntiJoin
│   ├── aggregate.rs         # PhysicalAggregate + AggregateHashTable
│   ├── order_by.rs          # PhysicalOrderBy + BlockMergeSorter
│   ├── limit.rs             # PhysicalLimit
│   ├── union.rs             # PhysicalUnion
│   ├── flatten.rs           # PhysicalFlatten
│   ├── semi_masker.rs       # PhysicalSemiMasker + NodeSemiMask
│   ├── recursive_extend.rs  # PhysicalRecursiveExtend
│   ├── explain.rs           # PhysicalExplain
│   ├── foreach.rs           # PhysicalForeach
│   ├── ddl/
│   │   ├── mod.rs
│   │   ├── copy_from.rs
│   │   ├── copy_to.rs
│   │   └── ...
│   └── scan/
│       ├── mod.rs
│       ├── node_table.rs
│       ├── rel_table.rs
│       └── index_scan.rs
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

- `[ ]` **2.1** Phase 2A — Split `physical_operator.rs` (lihat P10.6)
- `[ ]` **2.2** Phase 2B — Split `processor.rs` helpers
- `[ ]` **2.3** Verifikasi: `cargo test -p kuzu-processor` (77 passing)

---

## Phase 3: kuzu-main — Split `connection.rs` (3.133 lines)

**Analisis:** ~2000 lines logic + ~1130 lines inline tests (`#[cfg(test)]`)

### Target Struktur

```
kuzu-main/src/
├── connection/
│   ├── mod.rs                # Connection struct + new(), close()
│   ├── query.rs              # query(), execute() — main pipeline entry
│   ├── ddl.rs                # handle_ddl: CREATE/DROP/ALTER TABLE, INDEX, SEQUENCE, ANALYZE, EXPORT/IMPORT DB
│   ├── dml.rs                # handle_dml: MATCH/MERGE/DELETE/SET/FOREACH
│   ├── call.rs               # handle_call: show_tables, table_info, show_functions, dll (12 CALL functions)
│   ├── copy.rs               # copy_from, copy_to
│   ├── transaction.rs        # begin_write_txn, commit, rollback, set_transaction_mode
│   ├── prepared.rs           # PreparedStatement, execute_prepared
│   ├── substitute.rs         # substitute_params_in_statement, substitute_in_bound_expr, dll
│   └── utils.rs              # pk_value_to_string, extract_arg_string, rows_to_datachunk, format_storage_size, value_to_csv_string
├── connection_test.rs        # Semua #[cfg(test)] dari connection.rs dipindahkan
├── database.rs
├── query_result.rs
├── adbc.rs
└── lib.rs
```

### Langkah

- `[ ]` **3.1** Buat direktori `kuzu-main/src/connection/`
- `[ ]` **3.2** Ekstrak `mod.rs` — struct Connection + new(), close(), set_default_schema
- `[ ]` **3.3** Ekstrak `query.rs` — query(), execute() pipeline
- `[ ]` **3.4** Ekstrak `ddl.rs`, `dml.rs`, `call.rs`, `copy.rs`, `transaction.rs`, `prepared.rs`
- `[ ]` **3.5** Ekstrak helpers: `substitute.rs`, `utils.rs`
- `[ ]` **3.6** Ekstrak tests: `connection_test.rs`
- `[ ]` **3.7** Update `lib.rs` — `pub mod connection`
- `[ ]` **3.8** Verifikasi: `cargo test -p kuzu-main` (64 unit + 44 integration)

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

### Langkah

- `[ ]` **4.1** Buat struktur `passes/flat/` dan `passes/tree/`
- `[ ]` **4.2** Ekstrak setiap pass ke file sendiri
- `[ ]` **4.3** `passes/mod.rs` — orchestrate semua pass dalam urutan yang benar
- `[ ]` **4.4** Verifikasi: `cargo test -p kuzu-optimizer` (49 passing)

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

- `[ ]` **5.1** Buat direktori `kuzu-parser/src/parser/`
- `[ ]` **5.2** Ekstrak `ddl.rs` — semua `parse_create_*`, `parse_drop_*`, `parse_copy_*`, `parse_alter_*`, `parse_analyze`
- `[ ]` **5.3** Ekstrak `dml.rs` — `parse_match`, `parse_return`, `parse_create`, `parse_delete`, `parse_set`, `parse_merge`, `parse_foreach`, `parse_unwind`, `parse_with`
- `[ ]` **5.4** Ekstrak `expression.rs` — semua parsing expression
- `[ ]` **5.5** Ekstrak `query.rs` — `parse_query`, `parse_union`, `parse_call`, `parse_transaction`
- `[ ]` **5.6** Verifikasi: `cargo test -p kuzu-parser` (63 passing)

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

- `[ ]` **6.1** Buat direktori `kuzu-binder/src/bind/`
- `[ ]` **6.2** Ekstrak DDL binding → `bind/ddl.rs`
- `[ ]` **6.3** Ekstrak DML binding → `bind/dml.rs`
- `[ ]` **6.4** Ekstrak expression binding → `bind/expression.rs`
- `[ ]` **6.5** Verifikasi: `cargo test -p kuzu-binder` (14 passing)

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
| **P-MOD3** | `connection.rs` → connection/ + test | 3.133 | **5** | Low | P-MOD2 |
| **P-MOD4** | `passes.rs` → 21 files | 2.486 | **5** | Low | — |
| **P-MOD5** | `parser.rs` → parser/ | 2.183 | **5** | Low | — |
| **P-MOD6** | `binder.rs` → bind/ | 1.667 | **3** | Low | P-MOD5 |
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
