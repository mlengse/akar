# Contributing to Kuzu Core

Thanks for your interest in contributing to the Kuzu Rust port!

## Getting Started

### Prerequisites

- **Rust 1.80+** (MSRV)
- **Cargo** (comes with Rust)
- Optional: `wasm-pack` for WASM builds

### Setup

```bash
git clone https://github.com/kuzudb/kuzu.git
cd kuzu/kuzu-core
cargo build --workspace
cargo test --workspace
```

### Project Structure

```
kuzu-core/
├── kuzu-common/        # Type system, Value, DataChunk
├── kuzu-storage/       # BufferManager, WAL, tables, indexes
├── kuzu-transaction/   # MVCC, TransactionContext
├── kuzu-catalog/       # Schema management
├── kuzu-parser/        # Cypher PEG grammar (pest.rs)
├── kuzu-binder/        # Semantic analysis
├── kuzu-planner/       # Logical plan (34 operators)
├── kuzu-optimizer/     # 21 optimization passes
├── kuzu-processor/     # Physical execution engine
├── kuzu-function/      # 150+ built-in functions
├── kuzu-graph/         # GDS framework + CSR
├── kuzu-main/          # Database, Connection, QueryResult
├── kuzu-cli/           # Interactive Cypher shell
└── kuzu-{extension}/   # 14 extension crates
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
cargo test --workspace                      # All 954 tests
cargo test -p kuzu-processor               # Single crate
cargo test --test test_fts -p kuzu-main     # Single integration test
```

### Lint

```bash
cargo fmt --all -- --check                  # Format check
cargo clippy --workspace --all-targets -- -D warnings  # Lint (must pass!)
cargo audit                                 # Security audit
```

### Benchmark

```bash
cargo bench -p kuzu-main                    # Full pipeline
cargo bench -p kuzu-processor               # Operator micro-benchmarks
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
- Integration tests in `kuzu-main/tests/`
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
