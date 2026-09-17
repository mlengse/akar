# Deep Exploration — akar-extension

The extension framework is the plug-in mechanism that keeps Akar modular: core query execution, storage, and transactions live in a few crates while optional capabilities (JSON, HTTPFS, Lakehouse, vectors, FTS, algorithms) attach through one trait. An `Extension` registers scalar functions, table functions, and custom execution via `ExtensionContext`; `ExtensionRegistry` accumulates the actions; `LOAD EXTENSION` and `register_builtin_extensions` bring them into a database.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `Extension` trait | `register_extension(ExtensionContext)` — single entry point | `akar-core/akar-extension/src/lib.rs:20-27` |
| `ExtensionContext` | Time, logger, specific_config, catalog info, scratch space | `akar-core/akar-extension/src/context.rs:11-69` |
| `ExtensionRegistry` | Accumulated `ExtensionAction` (RegisterFunction / RegisterTableFunction) | `akar-core/akar-extension/src/registry.rs:12` |
| `CustomScalar` / `CustomTable` / `CustomTableWithGraph` | Execute user functions/tables | `akar-core/akar-extension/src/` |
| built-in registration | `register_builtin_extensions` + `load_all()` | `akar-core/akar-main/src/database.rs:667-773` |

## Design Decisions

- **Single trait, multiple shapes.** One `Extension` entry point handles scalar (custom_scalar), table (custom_table / custom_table_with_graph), and future types — the trait is future-proof rather than specializing early. `CustomTableWithGraph` was added to let GDS closures receive `Option<&dyn GraphDataSource>` (committed 0290a8c), so algorithms can consult the catalog.
- **Registry collects actions, execution is lazy.** `register_extension` appends to a list; functions are invoked only when a query calls them. Bootstrap runs `load_all` at `Database::new` (`database.rs:667-773`).
- **Extensions depend only on common + catalog.** Keeping the dependency surface minimal is what lets the workspace publish 32 crates bottom-up (ADR-002) without serializing everything on storage.

## Why It Matters

The extension boundary is where Akar's feature surface is defined. Removing an extension crate disables its functions while keeping the core intact; adding one is a one-file `register_extension` diff. Vector similarity, FTS, JSON, HTTP, SQLite/Postgres connectors, Lakehouse readers, and the LLM embedding hook all ride this mechanism.