# Kuzu Processor

Physical operator execution pipeline — the runtime that executes query plans.

**Physical operators (9+):**
- `PhysicalScan` — reads column-major table data into DataChunks
- `PhysicalFilter` — evaluates expressions, produces selection vectors
- `PhysicalProjection` — selects/orders columns
- `PhysicalOrderBy` — multi-key sorting (all Value types, NULLs last)
- `PhysicalLimit` — chunk-aware LIMIT/OFFSET
- `PhysicalHashJoin` — hash join with Value-keyed hash table (all types)
- `PhysicalAggregate` — scalar and GROUP BY aggregates (COUNT, SUM, AVG, MIN, MAX)
- `PhysicalCopyFrom` — CSV/Parquet file loading into tables
- `PhysicalDelete` — row deletion from node tables
- `PhysicalSet` — property updates on matched rows
- `PhysicalUnwind` — list expression expansion into rows

**Expression evaluation:**
- `ExpressionEvaluator` module with scalar function dispatch
- Type coercion and null propagation

**Pipeline:** `QueryProcessor::execute(&[LogicalOperator])` → flattens plan → runs physical operators → returns `Vec<DataChunk>`.

**Tests:** 28
