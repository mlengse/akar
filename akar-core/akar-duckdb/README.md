# Kuzu DuckDB Extension

DuckDB integration for the Kuzu database engine using the `duckdb` crate (v1.10504.0).

**Modes:** In-memory, file-based, local (temporary)

**Functions:** `duckdb_query` (scalar), `duckdb_scan` (table)

**Shared helper:** `DuckDbAttachHelper` used by Delta, Iceberg, Azure, and Unity Catalog extensions for DuckDB delegation.

**Tests:** 9
