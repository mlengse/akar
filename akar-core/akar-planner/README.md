# Akar Planner

Logical query plan construction from bound statements.

**Logical operators (60 variants):**
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
- `StandaloneCall` — CALL procedures
- `CopyFrom` / `BatchInsert` — data loading
- `IndexLookup` / `Delete` / `Set` — mutations
- `OptionalMatch` / `OptionalExtend` / `Unwind` / `Foreach` — control flow
- `Merge` / `MergeRel` / `AntiJoin` — advanced joins
- `Explain` / `RecursiveExtend` / `Accumulate` — introspection
- `ExpressionsScan` / `CountRelTable` / `Partitioner` / `PathPropertyProbe` — utilities
- `MultiplicityReducer` / `Skip` / `Insert` / `IndexScan` / `ExtensionClause` — misc
- **DDL operators (12):** `CreateNodeTable`, `CreateRelTable`, `DropTable`, `AlterTable`, `CreateIndex`, `DropIndex`, `CreateVectorIndex`, `CreateSequence`, `DropSequence`, `CreateDml`, `CreateNode`, `CreateRel`, `Extend`, `ExportDatabase`, `ImportDatabase`, `CreateFtsIndex`, `FtsScan`, `EmptyResult`

**Passes through:** LogicalOperator tree with `children_mut()`, `visit_bottom_up()` helpers.

**Tests:** 23