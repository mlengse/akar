# Akar Binder

Semantic analysis and symbol resolution for Cypher queries.

**Features:**
- Binds AST `Statement` to `BoundStatement` with resolved types
- Validates table/column existence against catalog
- Type inference from expressions and schema
- Resolves property accesses to column indices
- Handles: MATCH, OPTIONAL MATCH, RETURN, WITH, WHERE, CREATE, DELETE, SET, ALTER TABLE, COPY FROM, UNION ALL, UNWIND
- DDL validation (PRIMARY KEY, column types, table existence)
- Query parameter binding (`$name` syntax)

**Tests:** 87
