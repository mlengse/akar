# Deep Exploration — akar-sqlite

`akar-sqlite` connects Akar to SQLite databases — reading external SQLite tables into Cypher queries and writing query results back out. In-process via rusqlite.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `sqlite_scan` | Table function: read a SQLite table as a graph table | `akar-core/akar-sqlite/src/lib.rs` |
| `sqlite_attach` / helper | Attach helper functions | `akar-core/akar-sqlite/src/` |
| `sqlite_backup` | Export Akar query results to SQLite | `akar-core/akar-sqlite/src/` |

## Design Decisions

- **Read and write both supported.** Unlike httpfs, sqlite is bidirectional: ingestion from existing SQLite DBs and export to SQLite files. This makes SQLite a convenient interchange/persist format for small memory stores.
- **Separated from core.** SQLite is a C++ dependency (rusqlite → libsqlite3-sys), so it lives behind a feature (`--all-features` to build), consistent with the duckdb policy.

## Why It Matters

Many agent memory stores already exist as SQLite files. `sqlite_scan` lets an agent query them in-memory with Cypher without an ETL step; `sqlite_backup` exports Akar tables for downstream tools. It is also the lightest operational dependency of the extension set.