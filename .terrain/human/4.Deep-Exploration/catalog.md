# Deep Exploration — akar-catalog

The catalog is Akar's schema authority. Every node table, rel table, and column — their names, types, and the name↔id mappings used everywhere else — live here. It is the in-memory `TableCatalog` that binder and planner rely on for resolution, and it is serialized to disk as `catalog.json` so a reopened database restores its schema exactly.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `TableCatalog` | In-memory registry of tables, columns, indexes, vector index metadata | `akar-core/akar-catalog/src/lib.rs` |
| `SystemCatalog` | Serialized schema snapshot (table/column/index definitions) persisted as `catalog.json` | `akar-core/akar-catalog/src/` |
| `TableMetadata` | Per-table name, id, columns, primary key column, vector index registration | `akar-core/akar-catalog/src/` |
| `refresh_vector_indexes_for_tables` | Rebuilds vector index metadata after table reload (wired from `akar-main`) | `akar-core/akar-catalog/src/lib.rs` |

## Design Decisions

- **In-memory catalog + `catalog.json` on disk.** Chosen so that metadata lookups are lock-free reads on a hot struct, while schema survives restarts. Alternative: derive schema from column files at startup — rejected because rel/table semantics (labels, PK column, directionality) are not recoverable from raw pages.
- **Name↔id indirection.** All DML references tables by integer id internally; names are only resolved at bind time. This is why `ALTER TABLE ... RENAME` (used by spell-name migrations) touches only the catalog and not stored row data.

## Why It Matters

Binding (`akar-core/akar-binder/src/bound_statement.rs`) resolves pattern labels against these tables, planning (`akar-core/akar-planner/src/plan.rs`) asks for primary keys and column indexes, and the extension framework (`ExtensionContext::get_table_info` in `akar-core/akar-extension/src/context.rs:11-69`) hands extensions the same metadata. Adding a table via `CREATE NODE TABLE` (for example) funnels through `database.rs` DDL execution and updates the catalog before any column file exists.