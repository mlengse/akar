# Kuzu Catalog

System catalog managing schema metadata — node tables, relationship tables, and their columns.

**Features:**
- `CatalogEntry` with entries for NodeTable and RelTable
- `CatalogColumn` with name, logical type, primary key flag
- CRUD operations: create, get, get_by_name_mut, add_column, drop_column, rename_column, rename_table
- Name-to-ID and ID-to-name bidirectional lookup
- Thread-safe via `Arc<Mutex<Catalog>>`
- Integration with Binder for semantic analysis

**Tests:** 14
