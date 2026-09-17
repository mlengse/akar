# Deep Exploration — akar-duckdb

`akar-duckdb` embeds DuckDB as an optional execution/attach helper. Its main value is as the delegation fallback for Lakehouse extensions: when a native reader (delta/iceberg/azure/uc) is insufficient, `DuckDbAttachHelper::attach_file` + `query_sql` hand the heavy lifting to DuckDB's well-tested catalogs.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `DuckDbAttachHelper` | Attach file/URI + run queries in DuckDB | `akar-core/akar-duckdb/src/lib.rs` |
| `DuckDbExecutor` | In-process DuckDB execution via libduckdb-sys | `akar-core/akar-duckdb/src/` |
| Delegation protocol | Extension table functions may forward to DuckDB | `akar-core/akar-duckdb/src/lib.rs` |

## Design Decisions

- **"Native first, DuckDB as fallback."** Delta/iceberg/azure/uc each implement native readers; DuckDB delegation is the safety net for cases the native path doesn't handle (e.g. complex remove/add action sequences in `_delta_log`). This bounds effort while guaranteeing correctness on hard cases.
- **Gated behind Cargo features.** DuckDB is C++ (via `libduckdb-sys`) — excluded from the default `test [akar-core]` build, so the pure-Rust core stays pure and CI stays fast (`--all-features` builds include it, as `check [akar-core]` requires).

## Why It Matters

The Lakehouse extensions would be highly custom without a common delegation path. `akar-duckdb` is the shared piece that lets delta/iceberg/azure/uc deliver on "query your data lake with Cypher" with bounded implementation effort — and it is precisely why `check [akar-core]` needs `--all-features`.