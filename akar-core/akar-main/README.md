# Akar Main

Public API for the Akar database engine.

**`Database`** — Main entry point. Initializes all subsystems (storage, transaction manager, catalog, function registry, extensions, stats store).

**`Connection`** — Query execution. Full pipeline: `parse → bind → plan → optimize → execute`. Handles DDL, DML, COPY FROM, and prepared statements.

**`QueryResult`** — Result encapsulation with row/column counts, display formatting, iterator over columns.

**`PreparedStatement`** — Parameterized query preparation and execution with `$param` syntax.

**`SystemConfig`** — Configuration (buffer pool size, threads, compression, auto-checkpoint, read-only mode).

**`Database::connect_tcp`** — Remote client for embedded server mode (length-prefixed JSON framing).

**Tests:** 68 unit + 293 integration
