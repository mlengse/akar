# Akar Planner

Logical query plan construction from bound statements.

**Logical operators (58 variants):**
- `ScanNode` / `ScanRel` — table scans
- `Filter` — predicate evaluation
- `Projection` — column selection
- `OrderBy` — sorting (multi-key, ASC/DESC)
- `Limit` — limit + offset
- `HashJoin` / `CrossProduct` — join strategies
- `Aggregate` — scalar and GROUP BY
- `Flatten` — factorization unwinding
- `Union` — concatenation
- `Intersect` — WCOJ intersect
- `SemiJoin` / `RecursiveExtend` — traversal
- `TableFunctionCall` — extension table functions
- +12 DDL operators

**Passes through:** LogicalOperator tree with `children_mut()`, `visit_bottom_up()` helpers.

**Tests:** 22
