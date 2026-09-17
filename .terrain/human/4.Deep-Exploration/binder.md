# Deep Exploration — akar-binder

The binder is the semantic checking layer. It walks the parsed AST and resolves every node/rel label, column name, and expression against the catalog's schema, annotating types and rejecting invalid queries (unknown column, type mismatch, missing PRIMARY KEY) before planning ever runs. Its output is a `BoundStatement` — a typed, schema-resolved tree that the planner consumes directly.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `BoundStatement` | Typed, schema-resolved statement tree | `akar-core/akar-binder/src/bound_statement.rs` |
| `BoundExpression` | Resolved expression with inferred type + column references | `akar-core/akar-binder/src/` |
| pattern binding | Resolve `(n:Label)` and `[r:RTYPE]` into table ids | `akar-core/akar-binder/src/` |
| type inference | Propagate types through expressions (arithmetic, functions, casts) | `akar-core/akar-binder/src/` |
| param validation | Confirm `$param` types compatible with declared types | `akar-core/akar-binder/src/` |

## Design Decisions

- **Bind-time schema resolution.** Node/rel names are resolved to integer table ids here, once per statement, so planner/processor never pay string-lookup costs. The alternative — leaving names unresolved until execution — was rejected for both speed and error reportability (users get "unknown table" before any work is done).
- **BoundStatement is the contract to planner.** Keeping binding strictly separate from planning (`akar-core/akar-planner/src/plan.rs`) mirrors Kuzu's tiered frontend, making the two stages independently testable.

## Why It Matters

Binder correctness prevents whole classes of runtime bugs: a query over a non-existent label, a filter on a column not in the table, an aggregate mixing scalar and vector semantics. It also gates prepared statements — `prepare` returns the bound plan, and `execute` only fills in `$param` values (`akar-core/akar-main/src/connection/query.rs:317-448`).