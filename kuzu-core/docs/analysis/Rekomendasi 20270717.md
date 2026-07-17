## 🔴 Prioritas 1: Tutup 68 Ignored Tests

Dari 1099 total test, **68 masih di-ignore** — mayoritas di edge case suite (65 dari 137):

| Test File | Total | Pass | Ignore |
|-----------|-------|------|--------|
| `edge_null_handling` | 40 | 17 | 23 |
| `edge_nested_types` | 13 | 0 | **13** |
| `edge_ddl_errors` | 21 | 11 | 10 |
| `edge_boundary` | 20 | 16 | 4 |
| `edge_concurrency` | 11 | 10 | 1 |
| `edge_empty_tables` | 21 | 10 | 7 |
| `edge_unicode` | 11 | 7 | 4 |

Ini adalah **indikator langsung** bahwa masih ada fitur yang belum stabil. Investigasi root cause dan fix harus jadi prioritas sebelum klaim "production-ready."

---

## 🟡 Prioritas 2: Optimasi Query Kompleks

C++ parity sudah tercapai untuk query sederhana (filter + count). Tapi query kompleks masih **jauh dari optimal**:

| Operator | Current | Target | Gap |
|----------|---------|--------|-----|
| Multi-key GROUP BY (10k) | **3,987 µs** | <2,000 µs | ~2× |
| OrderBy single-key (10k) | **1,388 µs** | <700 µs | ~2× |
| HashJoin build (10k) | **1,450 µs** | <800 µs | ~1.8× |

**3 item deferred dari P27 yang harus dilanjutkan:**
1. **P27c** — Hindari `Vec<Value>` alokasi di multi-key GROUP BY (hash composite key langsung)
2. **P27d** — K-way merge `O(k)` → `O(log k)` pakai `BinaryHeap`
3. **P27f** — `#[inline]` annotations di hot-path aggregate

---

## 🟡 Prioritas 3: LadybugDB Benchmark

Semua klaim performa saat ini hanya terhadap **Vela C++** (`kuzu_benchmark.exe`). Belum ada perbandingan sistematis terhadap **LadybugDB C++**. Padahal `ladybug/` adalah git submodule lengkap dengan benchmark sendiri (`benchmark/click`, `tools/benchmark/navix`).

**Rekomendasi:** Jalankan benchmark yang sama terhadap LadybugDB untuk validasi parity.

---

## 🟢 Prioritas 4: Bangun Regression Pipeline

Beberapa item teknis yang perlu diselesaikan untuk maturity:

| Item | Status | Effort |
|------|--------|--------|
| STANDALONE_CALL dispatch (masih string matching) | 🟡 Deferred | 2 SP |
| WASM tests (3 pass, 1 ignore) | 🟢 Rendah | 1 SP |
| Fuzz targets (butuh nightly Rust) | ✅ Defined | 1 SP |
| GitHub Releases / binary distribution | ❌ Belum | 2 SP |
| MIGRATION.md English | ✅ Done | — |

---

## 🏆 Rekomendasi Sprint Berikutnya

```
Sprint 4: "Stabilisasi & Benchmark Komprehensif" (12-15 SP)
├── 🔴 Fix 68 ignored tests (6 SP)
│   ├── nested_types (13 tests — root cause pemahaman)
│   ├── null_handling (23 tests — paling kritis)
│   └── ddl_errors + empty_tables + boundary (21 tests)
├── 🟡 P27c + P27d + P27f optimasi (4 SP)
├── 🟡 LadybugDB benchmark suite (2 SP)
└── 🟢 STANDALONE_CALL refactor + WASM fix (2 SP)
```

**Target output:** 1099 ✅ pass, 0 ignore, benchmark terverifikasi terhadap Vela **dan** LadybugDB.