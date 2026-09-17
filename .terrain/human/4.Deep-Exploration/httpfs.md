# Deep Exploration — akar-httpfs

`akar-httpfs` adds HTTP access as a read path for data and (optionally) model artifacts. It registers `http_get` (fetch a URL as text/bytes) and `http_scan` (read CSV/Parquet over HTTP as a table source), and it established the pattern other network extensions (azure, unity-catalog) reuse.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `http_get` | Scalar: fetch URL content | `akar-core/akar-httpfs/src/lib.rs` |
| `http_scan` | Table function: CSV/Parquet over HTTP | `akar-core/akar-httpfs/src/` |
| VFS integration | HTTP reads usable where local reads are | `akar-core/akar-httpfs/src/` |
| `check_httpfs` run config | CI gate for HTTPFS tests | `akar-main` run configs |

## Design Decisions

- **Read-only by design.** `http_scan` / `http_get` are read-only; writing to remote stores goes to the purpose-specific Lakehouse extensions (delta/azure/uc). Chosen to avoid a general write layer with all its auth/retry complexity.
- **Reuses the storage VFS abstraction.** Working through `akar-common`'s VFS means the same pyarrow-free readers handle local and HTTP sources, minimizing duplicate code.

## Why It Matters

An agent's memory often references external data (web pages, model cards, tool outputs). Being able to `CALL`/scan a remote Parquet into the same query as local memory tables is how Akar supports live external context without a data pipeline.