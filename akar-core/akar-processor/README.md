# Akar Processor

Physical operator execution pipeline — the runtime that executes query plans.

**Physical operators (48 structs):**
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
- `PhysicalIntersect` — WCOJ intersect execution
- `PhysicalRecursiveExtend` — recursive path traversal
- `PhysicalCountRelTable` — CSR metadata COUNT

**Expression evaluation:**
- Arrow-native evaluation (`evaluate_to_arrow` + `boolean_array_to_selection`)
- Parallel aggregation via `AggregateHashTable`
- Parallel hash join via `JoinHashTable`
- `BlockMergeSort` + `RadixSort` for ORDER BY
- `BinaryHeap` O(n log k) TopK

**Pipeline:** `QueryProcessor::execute(&[LogicalOperator])` → flattens plan → runs physical operators → returns `Vec<DataChunk>`.

**Tests:** 18
