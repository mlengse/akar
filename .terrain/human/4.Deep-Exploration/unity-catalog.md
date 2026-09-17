# Deep Exploration — akar-unity-catalog

Databricks Unity Catalog tables are exposed to Akar through `uc_scan` — either via a native REST client (catalog + table + column listing) or via DuckDB's `uc_catalog` extension. This is the smallest and newest extension, still stabilizing.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `uc_scan` | Table function: scan a UC table | `akar-core/akar-unity-catalog/src/lib.rs` |
| native REST client | Catalog/table/column listing without Databricks runtime | `akar-core/akar-unity-catalog/src/` |
| DuckDB uc_catalog delegation | Fallback when native REST is incomplete | `akar-core/akar-duckdb/src/lib.rs` |

## Design Decisions

- **Same pattern as azure/delta: native + DuckDB fallback.** The common delegation library makes this crate tiny — the UC-specific code is a thin native REST client for catalog metadata, and all query execution delegates to DuckDB.

## Why It Matters

Unity Catalog is the metadata governance layer for Databricks Lakehouse. For agents analyzing governed data, `uc_scan` provides direct Cypher access to UC-managed tables, with auth/billing handled at the DuckDB/REST level.