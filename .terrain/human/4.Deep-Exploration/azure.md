# Deep Exploration — akar-azure

Azure Blob Storage becomes readable in Akar through `azure_scan`, which exposes objects as table-like rows (blob name, content, metadata) — either through a native REST+SAS client or by delegating to DuckDB's httpfs.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `azure_scan` | Table function: read blob data from Azure via REST+SAS | `akar-core/akar-azure/src/lib.rs` |
| REST+SAS URL support | Native HTTP path without Azure SDK | `akar-core/akar-azure/src/` |
| DuckDB httpfs delegation | Fallback for SAS-less or complex auth scenarios | `akar-core/akar-duckdb/src/lib.rs` |

## Design Decisions

- **SAS URL (no SDK) as default.** SAS URLs encode auth in the query string, enabling a plain HTTP client — avoiding the heavy `azure-storage-blobs` dependency for simple reads. The alternative (SDK-first) was rejected for binary size and build time.
- **DuckDB delegation for the rest.** When SAS isn't available, `DuckDbAttachHelper::query_sql` with DuckDB's httpfs extension provides the remaining path — another instance of the "native first, DuckDB as safety net" pattern from `akar-duckdb`.

## Why It Matters

Cloud-hosted agents keep memory tables in object stores. `azure_scan` reads these tables directly in Cypher, and COPY to Azure writes them back — making Azure Blob a first-class storage tier alongside local files for memory archives.