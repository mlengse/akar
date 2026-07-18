## Audit Porting: Kuzu C++ (Vela) + LadybugDB → Kuzu Rust 18/07/2026

### Kesimpulan Awal: **~95% fitur inti sudah diporting.** Tidak ada critical gap di query engine, storage, atau GDS.

---

### 🔴 Critical Gaps (0) — NONE

Query engine, storage engine, GDS algorithms, extensions — semua sudah parity.

---

### 🟡 Medium Gaps (perlu diporting untuk full parity)

| # | Gap | Ada di | Detail |
|---|-----|--------|--------|
| 1 | **`list_transform` / `list_filter` / `list_reduce`** | C++ Vela + Ladybug | Evaluator sudah support (expression_evaluator.rs:432-647), tapi **tidak terdaftar di FunctionRegistry**. Artinya `list_transform(list, x -> x+1)` akan error "function not found". |
| 2 | **`GREATEST` / `LEAST`** | C++ Vela | Fungsi extremum (ambil max/min dari N argumen). Tidak ada implementasi Rust. |
| 3 | **Function aliases (7 missing)** | C++ Vela | `pow`, `log10`, `prefix`, `suffix`, `list_cat`, `element_at`, `cardinality` — base functions exist, aliases tidak terdaftar. |
| 4 | **`show_projected_graphs` / `projected_graph_info` / `drop_projected_graph`** | Ladybug | CALL function untuk manajemen projected graph. Rust tidak ada handler. |
| 5 | **`kuzu-migrate` 1 ignored test** | Rust-only | Parquet footer corruption. Pre-existing, perlu fix parquet writer. |

---

### 🟢 Minor Gaps (nice-to-have, non-critical)

| # | Gap | Detail |
|---|------|--------|
| 6 | **`StorageDriver` API** | Low-level storage access API (ada di Ladybug) |
| 7 | **`ConfidentialStatementAnalyzer`** | Security feature: scan query untuk PII |
| 8 | **Shell: HTML/LaTeX output** | Alternatif format output |
| 9 | **Shell: extended commands** | `:schema`, `:highlight`, `:max_rows`, dll |
| 10 | **WAL dump tool** | Debug forensic tool |
| 11 | **Gzip file system** | `gzip_file_system.h` — wrapper for compressed files |
| 12 | **Progress bar** | Infrastruktur progress display |
| 13 | **`ConstantOrNullFunction`** | C++ utility function |

---

### ✅ Rust Melebihi C++ (keunggulan)

| Fitur | Rust vs C++ |
|-------|------------|
| **Optimizer passes** | 22 vs 17 (+5) |
| **Join ordering** | DP Bushy Trees vs Greedy |
| **Fuzz testing** | 3 cargo-fuzz target terintegrasi CI |
| **Property-based testing** | proptest: round-trip, associativity, equivalence |
| **GDS algorithms** | 15+ termasuk Node2Vec, Random Walk (C++: 15) |
| **Arrow-native execution** | Zero-copy `ColumnChunk→ArrayRef`, `arrow::compute::take()` |
| **Roaring Bitmap** | Native Rust (vs C CRoaring) |
| **Automatic CI** | 10 job (fmt, clippy, test×3, feature, wasm-test, coverage, fuzz×3) |

---

### Rekomendasi Prioritas

Kalau mau porting sisanya sebelum **P30.6**:

1. **`list_transform/filter/reduce`** — daftarkan di `register_builtins()` — **~30 menit**
2. **`GREATEST`/`LEAST`** — implement + register — **~30 menit**
3. **7 function aliases** — daftarkan di `register_builtins()` — **~15 menit**
4. **`show_projected_graphs` dkk.** — 3 CALL handler — **~1 jam**

Total: **~3 jam** untuk porting semua gap medium → 100% function parity.
