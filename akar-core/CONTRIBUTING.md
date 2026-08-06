# Contributing to Akar

Thanks for your interest in contributing to Akar!

## Getting Started

### Prerequisites

- **Rust 1.80+** (MSRV)
- **Cargo** (comes with Rust)
- Optional: `wasm-pack` for WASM builds

### Setup

```bash
git clone https://github.com/anjangkusumanetra/akar.git
cd akar/akar-core
cargo build --workspace
cargo test --workspace
```

### Project Structure

```
akar-core/
├── akar-common/        # Type system, Value, DataChunk
├── akar-storage/       # BufferManager, WAL, tables, indexes
├── akar-transaction/   # MVCC, TransactionContext
├── akar-catalog/       # Schema management
├── akar-parser/        # Cypher PEG grammar (pest.rs)
├── akar-binder/        # Semantic analysis
├── akar-planner/       # Logical plan (58 operators)
├── akar-optimizer/     # 25 optimization passes
├── akar-processor/     # Physical execution engine
├── akar-function/      # 259 built-in functions
├── akar-graph/         # GDS framework + CSR
├── akar-main/          # Database, Connection, QueryResult
├── akar-cli/           # Interactive Cypher shell
├── akar-server/        # Embedded TCP server mode
└── akar-{extension}/   # 15 extension crates
```

## Development Workflow

### Build

```bash
cargo build --workspace                    # Debug build
cargo build --release --workspace           # Optimized build
cargo check --workspace                     # Fast compile check (no codegen)
```

### Test

```bash
cargo test --workspace              # All 1,349 tests (P48.14 fixed — no skip needed)
cargo test -p akar-processor               # Single crate
cargo test --test test_fts -p akar-main     # Single integration test
```

### Lint

```bash
cargo fmt --all -- --check                  # Format check
cargo clippy --workspace --all-targets -- -D warnings  # Lint (must pass!)
cargo audit                                 # Security audit
```

### Benchmark

```bash
cargo bench -p akar-main                    # Full pipeline
cargo bench -p akar-processor               # Operator micro-benchmarks
cargo bench --workspace --no-run            # Compile check only
```

## Code Conventions

### Style

- Follow `rustfmt` defaults (enforced by CI)
- `cargo clippy -- -D warnings` must pass
- Use `tracing` for logging, not `println!`
- Errors use `thiserror` derive macros

### Naming

- Types: `PascalCase`
- Functions/methods: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`
- Module files: `snake_case.rs`

### Safety

- **No unsafe code** unless porting C++ code that requires it
- If `unsafe` is needed, document the safety invariant with `// SAFETY:`
- Prefer `Option`/`Result` over panics

### Testing

- Unit tests in the same file with `#[cfg(test)] mod tests`
- Integration tests in `akar-main/tests/`
- Test both success and error paths
- Use `tempfile::tempdir()` for database state

## CI Pipeline

GitHub Actions runs on every push/PR to `master`/`main`:

| Job | OS | Command |
|-----|-----|---------|
| `fmt` | Ubuntu | `cargo fmt --all -- --check` |
| `clippy` | Ubuntu | `cargo clippy --workspace --all-targets -- -D warnings` |
| `test-ubuntu` | Ubuntu | `cargo build --workspace` + `cargo test --workspace` |
| `test-macos` | macOS | Same |
| `test-windows` | Windows | Same |
| `feature-gated` | Ubuntu | Build+test all 11 extension features |
| `wasm-check` | Ubuntu | `cargo check --target wasm32-unknown-unknown` |
| `bench-check` | Ubuntu | `cargo bench --workspace --no-run` |
| `coverage` | Ubuntu | `cargo tarpaulin` + Codecov |

## Architecture Decision Records

See [`docs/adr/`](docs/adr/) for key architectural decisions:

- [001: pest.rs vs ANTLR4](docs/adr/001-pest-not-antlr.md)
- [002: Pure Rust vs FFI](docs/adr/002-pure-rust-not-ffi.md)
- [003: Optimizer Pass Ordering](docs/adr/003-optimizer-pass-ordering.md)
- [004: Storage Engine Design](docs/adr/004-storage-column-major.md)
- [005: Transaction MVCC + Multiwriter](docs/adr/005-transaction-mvcc.md)

## Questions?

Open an issue on GitHub or refer to the [main README](README.md).
