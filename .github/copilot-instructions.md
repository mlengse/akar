# Copilot Instructions — Kùzu Graph Database

This repository contains the Kùzu embedded graph database. The Rust workspace lives in `akar-core/`; the C++ source is in `src/`.

## Cargo & diagnostics — strategy

This project uses two sets of tools for Rust development:

### Rust-analyzer MCP (primary — use whenever possible)

| MCP tool | Purpose |
|---|---|
| `rust_analyzer_diagnostics` | Per-file errors, warnings, hints (instant, no compilation) |
| `rust_analyzer_workspace_diagnostics` | All workspace diagnostics |
| `rust_analyzer_format` | Format a Rust file |
| `rust_analyzer_symbols` | Document symbols (functions, structs, etc.) |
| `rust_analyzer_definition` | Go to definition |
| `rust_analyzer_references` | Find all references |
| `rust_analyzer_hover` | Type info and docs |
| `rust_analyzer_code_actions` | Available code actions |
| `rust_analyzer_completion` | Code completion suggestions |
| `rust_analyzer_set_workspace` | Set workspace root to `akar-core/` |

**Workflow after every edit:**
1. `rust_analyzer_set_workspace` → `akar-core/`
2. `rust_analyzer_diagnostics` on each edited file
3. Fix any errors before moving on

### Terminal (secondary — only when MCP lacks the capability)

| Terminal command | When to use |
|---|---|
| `cargo check --workspace` | Cross-crate verification after multi-file changes |
| `cargo build --workspace` | Release build verification |
| `cargo test -p <crate>` | Run tests for a specific crate |
| `cargo test --workspace` | Full test suite (use `timeout_secs`) |
| `cargo clippy --workspace -- -D warnings` | Final gate before marking work complete |
| `cargo fmt --all -- --check` | Format verification |

Always `cd akar-core` before running terminal cargo commands.
