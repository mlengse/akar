# Kuzu Main

Public API for the Kuzu database engine.

**`Database`** — Main entry point. Initializes all subsystems (storage, transaction manager, catalog, function registry, extensions, stats store).

**`Connection`** — Query execution. Full pipeline: `parse → bind → plan → optimize → execute`. Handles DDL, DML, COPY FROM, and prepared statements.

**`QueryResult`** — Result encapsulation with row/column counts, display formatting, iterator over columns.

**`PreparedStatement`** — Parameterized query preparation and execution with `$param` syntax.

**`SystemConfig`** — Configuration (buffer pool size, threads, compression, auto-checkpoint, read-only mode).

**Tests:** 15 unit + 28 integration
