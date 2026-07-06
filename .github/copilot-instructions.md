# Copilot Instructions — Kùzu Graph Database

This repository contains the Kùzu embedded graph database. The Rust workspace lives in `kuzu-core/`; the C++ source is in `src/`.

## Cargo commands — use MCP tools, never the terminal

When working in this Rust/Cargo project, ALWAYS use the `cargo-mcp` MCP tools
instead of running `cargo` commands in a terminal. This applies even inside a
larger workflow — do not switch to the terminal for cargo just because a
previous step used the terminal.

| MCP tool | Replaces |
|---|---|
| `cargo_check` | `cargo check` |
| `cargo_build` | `cargo build` |
| `cargo_test` | `cargo test` |
| `cargo_clippy` | `cargo clippy` |
| `cargo_fmt_check` | `cargo fmt --check` |
| `cargo_fmt` | `cargo fmt` |
| `cargo_metadata` | `cargo metadata` |
| `cargo_tree` | `cargo tree` |
| `cargo_doc` | `cargo doc` |
| `cargo_clean` | `cargo clean` |
| `cargo_update` | `cargo update` |
| `cargo_fix` | `cargo fix` |
| `cargo_add` | `cargo add` |
| `cargo_remove` | `cargo remove` |
| `cargo_publish` | `cargo publish` |
| `cargo_nextest_run` | `cargo nextest run` |
| `cargo_nextest_list` | `cargo nextest list` |

Always pass `working_dir` set to the absolute path of the workspace root
(`kuzu-core/`). All boolean flags (`all_targets`, `release`, `workspace`,
`lib`, `bins`, `tests`, `benches`, `examples`, `all_features`,
`no_default_features`, `frozen`, `locked`, `offline`) expect JSON
`true`/`false`.

### cargo_test timeouts

- `timeout_secs` — hard overall wall-clock cap for the entire test-execution phase.
- `per_test_timeout_secs` — per-test budget (filter mode only).

### Redirecting output (`output_path`)

`cargo_check`, `cargo_build`, `cargo_test`, `cargo_clippy`, and `cargo_doc`
accept `output_path` to redirect the full NDJSON transcript to a file, keeping
the tool result compact. Use for large runs; read the file for full details
when errors are reported.

### Environment variables (`env`)

Every `cargo_*` tool accepts an optional `env` object for one-shot environment
overrides. Use for debug knobs (`RUSTFLAGS`, `RUST_LOG`, `RUST_BACKTRACE`) —
do not use for secrets or permanent config.
