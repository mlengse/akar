# Deep Exploration — akar-delta

Delta Lake tables become queryable in Akar through `delta_scan` (table function) and `delta_get_version_information`. The crate implements a native `_delta_log` JSON reader (P56.3a) that handles `commit.json` + `checkpoint-NNNNN.parquet` + sidecar loading, with a DuckDB-delegated fallback for complex action sets.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `delta_scan` | Table function: read a Delta table from a URI | `akar-core/akar-delta/src/lib.rs` |
| native delta log reader | Reads `_delta_log` commit JSONs and Parquet checkpoints | `akar-core/akar-delta/src/` |
| `delta_get_version_information` | Returns table schema from metadata | `akar-core/akar-delta/src/` |
| DuckDB fallback | `DuckDbAttachHelper::attach_file` + `query_sql` for complex tables | `akar-core/akar-duckdb/src/lib.rs` |

## Design Decisions

- **Native + DuckDB fallback.** P56.3a added a native reader that handles the common 1-shot-delta-log format; for more complex action sets (multi-part checkpoints, advanced predicates), `DuckDbAttachHelper` is the safety net. This keeps a pure-Rust path for common cases while retaining correctness coverage.
- **Session-state for output_file.** `session_state.output_file` stores the output path for COPY-like semantics (Delta metadata + file content), decoupling scan from export.

## Why It Matters

Delta Lake is the dominant open Lakehouse format. `delta_scan` is how an agent can query live data lake tables with Cypher (and combine them with in-memory graph patterns) without writing a custom ingestion job.