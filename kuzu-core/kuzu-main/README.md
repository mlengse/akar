# Kuzu Main

Public API for the Kuzu database engine.

**`Database`** — Main entry point. Initializes all subsystems (storage, transaction manager, catalog, function registry, extensions).

**`Connection`** — Query execution. Full pipeline: `parse → bind → plan → optimize → execute`.

**`QueryResult`** — Result encapsulation with row/column counts, display formatting.

**`SystemConfig`** — Configuration (buffer pool size, threads, compression, etc.).

**Tests:** 17 integration tests
