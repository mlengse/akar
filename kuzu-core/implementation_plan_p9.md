# P9: Production Hardening & CI/CD

> **Status:** Planning | **Target Date:** 2026-07-12
> **Prerequisites:** P8 (Native FTS) — ✅ COMPLETE

---

## Overview

Dengan selesainya fitur inti query engine (P0–P8) dan **922 test lulus**, fase P9 fokus pada **production hardening**: CI/CD pipeline, code quality enforcement, dokumentasi developer, publikasi crate, dan benchmark comparison terhadap C++.

---

## Prioritas

### 🔴 P9.1 — CI/CD Pipeline

- `[ ]` **GitHub Actions — CI workflow** (`kuzu-core/.github/workflows/ci.yml`)
  - `[ ]` `cargo check --workspace --all-features` (Windows, Linux, macOS)
  - `[ ]` `cargo test --workspace` (Windows, Linux, macOS)
  - `[ ]` `cargo clippy --workspace -- -D warnings`
  - `[ ]` `cargo fmt --check --workspace`
  - `[ ]` `cargo bench --workspace --no-run` (pastikan benchmark kompilasi)
- `[ ]` **GitHub Actions — WASM workflow**
  - `[ ]` `wasm-pack build kuzu-wasm --target nodejs`
  - `[ ]` `wasm-pack test --node kuzu-wasm`
- `[ ]` **GitHub Actions — Coverage workflow**
  - `[ ]` `cargo tarpaulin --workspace --out Html --out Xml`
  - `[ ]` Upload ke Codecov / Coveralls
- `[ ]` **Dependabot / Renovate** — dependency update automation
- `[ ]` **Release workflow** — `cargo publish` untuk 28 crate (dry-run dulu)

### 🟡 P9.2 — Code Quality & Linting

- `[ ]` **Clippy strict mode** — audit `clippy.toml` dan enable `#![warn(clippy::pedantic)]` bertahap
- `[ ]` **`rustdoc` lint** — `#![warn(missing_docs)]` untuk crate publik (kuzu-main, kuzu-common)
- `[ ]` **Safety audit** — `cargo audit` + `cargo geiger` untuk unsafe code detection
- `[ ]` **Unused dependencies** — `cargo udeps` untuk 28 crate
- `[ ]` **`Cargo.toml` standardization** — uniform `[package]` metadata (license, repository, documentation, keywords)

### 🟡 P9.3 — Benchmark Comparison: Rust vs C++

- `[ ]` **Setup C++ benchmark binary** — build `kuzu_benchmark.exe` dari `benchmark/`
- `[ ]` **Serialized dataset** — gunakan `tinysnb` dataset yang sama untuk kedua runtime
- `[ ]` **Fill gap table** di `BENCHMARK_COMPARISON.md`:
  - `[ ]` Seq Scan (q23): Rust `scan/10k_rows` vs C++
  - `[ ]` Filter (q14): Rust `filter/pass_all_10k` vs C++
  - `[ ]` Hash Join (q29): Rust `join/1k_build_1k_probe` vs C++
  - `[ ]` Order By (q25): Rust `order_by/single_key_1k` vs C++
  - `[ ]` Aggregate (q24): Rust `aggregate/count_10k` vs C++
  - `[ ]` Full Pipeline: Rust `query/match_return_all` vs C++
- `[ ]` **GDS benchmarks** — PageRank, WCC, SCC, K-Core pada dataset `soc-livejournal`
- `[ ]` **Optimization targets** — identifikasi gap > 2× dan buat issue

### 🟢 P9.4 — Documentation

- `[ ]` **`kuzu-main` API docs** — rustdoc examples untuk `Database`, `Connection`, `QueryResult`
- `[ ]` **Architecture decision records (ADR)** — `kuzu-core/docs/adr/`
  - `[ ]` 001: Mengapa pest.rs bukan ANTLR4
  - `[ ]` 002: Mengapa pure Rust, bukan FFI/cxx
  - `[ ]` 003: Optimizer tree pass ordering
  - `[ ]` 004: Storage engine: column-major + buffer manager
  - `[ ]` 005: Transaction: MVCC + Multiwriter
- `[ ]` **Contributor guide** — `kuzu-core/CONTRIBUTING.md`: setup, build, test, conventions
- `[ ]` **Crate-level README** untuk 28 crate (minimal: description + example)

### 🟢 P9.5 — WASM & NodeJS Polish

- `[ ]` **`kuzu-wasm` API completion** — ensure all `Connection` methods exposed
- `[ ]` **Browser target** — `wasm-pack build --target web` (saat ini hanya `nodejs`)
- `[ ]` **NPM publish** — `kuzu-wasm` ke npm registry
- `[ ]` **WASM integration tests** — `wasm-bindgen-test` untuk NodeJS environment

### 🟢 P9.6 — Performance Optimizations (Quick Wins)

- `[ ]` **Zero-copy Value references** — `Cow<str>` untuk `Value::String` agar menghindari clone
- `[ ]` **ExpressionEvaluator caching** — cache compiled regex, pre-compute constant sub-expressions
- `[ ]` **Parallel COPY FROM** — `rayon` parallel row parsing saat bulk load
- `[ ]` **BufferManager prefetch** — hint-based page prefetch untuk sequential scan
- `[ ]` **String interning** — `string_cache` atau `lasso` untuk column names, table names

---

## Verification Plan

### CI/CD
```bash
# Simulate CI locally
cargo check --workspace --all-features
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check --workspace
cargo bench --workspace --no-run
```

### Benchmark Comparison
```bash
# Rust
cargo bench -p kuzu-main -- --output-format bencher > rust_bench.txt
cargo bench -p kuzu-processor -- --output-format bencher >> rust_bench.txt

# C++ (future)
build/release/tools/benchmark/kuzu_benchmark.exe --json > cpp_bench.json
```

### Coverage
```bash
cargo tarpaulin --workspace --out Html --output-dir target/coverage
# Open target/coverage/tarpaulin-report.html
```

---

## Success Criteria

| # | Criteria | Target |
|---|----------|--------|
| 1 | CI green on 3 OS (Windows, Linux, macOS) | ✅ |
| 2 | Clippy pedantic: 0 warnings | ✅ |
| 3 | Coverage ≥ 80% line coverage | ✅ |
| 4 | Rust vs C++ gap ratio documented for 6 benchmark categories | ✅ |
| 5 | `kuzu-main` rustdoc with examples published | ✅ |
| 6 | `kuzu-wasm` published to npm | ✅ |
| 7 | All 28 crates have README + uniform Cargo.toml metadata | ✅ |

---

## Dependencies

| Crate | New Deps | Purpose |
|-------|----------|---------|
| `kuzu-wasm` | `wasm-bindgen-test` | WASM testing |
| All (CI) | `cargo-tarpaulin`, `cargo-audit`, `cargo-udeps` | CI tools (not runtime deps) |

---

## Estimated Effort

| Phase | Story Points | Risk |
|-------|-------------|------|
| P9.1 CI/CD | 5 | Low |
| P9.2 Code Quality | 3 | Low |
| P9.3 Benchmark Comparison | 5 | Medium (needs C++ build) |
| P9.4 Documentation | 3 | Low |
| P9.5 WASM Polish | 3 | Medium |
| P9.6 Performance Quick Wins | 8 | Medium |
| **Total** | **27** | |
