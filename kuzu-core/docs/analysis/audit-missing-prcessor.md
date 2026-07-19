# Audit missing processor operators 19/07/2026

## Complete Physical Operator Audit: `kuzu-processor` Crate

### Repository Structure

The crate is located at:
```
C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-processor\
```

**Module tree:**
```
src/
  lib.rs
  physical_operator.rs          -- re-exports everything from physical/*
  expression_evaluator.rs
  physical/
    mod.rs                      -- re-exports all submodules
    types.rs                    -- traits + core types (PhysicalOperatorExec, NodeSemiMask, etc.)
    common.rs                   -- shared utilities (store_value_in_vector, value_cmp, value_hash)
    missing_ops.rs              -- PhysicalAccumulate, PhysicalUnion, ResultCollector, DummySink, Profile, Partitioner
    misc.rs                     -- PhysicalEmptyResult, PhysicalMultiplicityReducer, PhysicalSkip,
                                   PhysicalUnionAllScan, PhysicalInsert, PhysicalExtensionClause
    join_ops.rs                 -- PhysicalCrossProduct, PhysicalSemiJoin, PhysicalAntiJoin,
                                   PhysicalIntersect, JoinHashTable, PhysicalHashJoin
    batch_insert.rs             -- PhysicalBatchInsert
    index_lookup.rs             -- PhysicalIndexLookup
    scan_filter/
      mod.rs
      scan.rs                   -- PhysicalScan
      scanrel.rs                -- PhysicalScanRel
      filter.rs                 -- PhysicalFilter
      projection.rs             -- PhysicalProjection
      limit.rs                  -- PhysicalLimit
      primarykeyscan.rs         -- PhysicalPrimaryKeyScan
      flatten.rs                -- PhysicalFlatten
      pathpropertyprobe.rs      -- PhysicalPathPropertyProbe
    write_ops/
      mod.rs
      set.rs                    -- PhysicalSet
      delete.rs                 -- PhysicalDelete
      foreach.rs                -- PhysicalForeach
      vectorsimilarityscan.rs   -- PhysicalVectorSimilarityScan
      copyfrom.rs               -- PhysicalCopyFrom + PhysicalArtIndexRangeScan
      physicalexplain.rs        -- PhysicalExplain
      recursiveextend.rs        -- PhysicalRecursiveExtend, PhysicalCreateNode, PhysicalCreateRel, PhysicalExtend
      ddl_fts.rs                -- PhysicalCountRelTable, PhysicalCreateFtsIndex, PhysicalFtsScan
      packedextend.rs           -- PhysicalPackedExtend
      standalonecall.rs         -- PhysicalStandaloneCall
      insert.rs                 -- PhysicalInsertNode, PhysicalInsertRel
      merge.rs                  -- PhysicalMerge
      unwind.rs                 -- PhysicalUnwind
    order_aggregate/
      mod.rs
      orderby.rs                -- PhysicalOrderBy
      topk.rs                   -- PhysicalTopK
      radixsort.rs              -- radix_sort_indices, is_radix_eligible (helpers)
      blockmergesort.rs         -- BlockMergeSorter (helper for PhysicalOrderBy)
      aggregate.rs              -- PhysicalAggregate, parse_aggregate_function
      aggregatehashtable.rs     -- AggregateHashTable (helper for PhysicalAggregate)
      splitaggregation.rs       -- SharedAggregateState, PhysicalAggregateScan, PhysicalAggregateFinalize
  processor/
    mod.rs                      -- QueryProcessor + helpers
    chunk_helpers.rs
    join_helpers.rs
    union_helpers.rs
    projection_helper.rs
    plan_serializer.rs
    mapper/
      mod.rs                    -- PlanMapper dispatcher
      map_scan.rs
      map_join.rs
      map_projection.rs
      map_aggregate.rs
      map_update.rs
      map_ddl.rs
```

---

### Complete Implementation Matrix

Below is every `LogicalOperator` variant (60 total) from `kuzu-planner` vs. its Rust physical operator status.

| # | LogicalOperator Variant | Physical Rust Struct | Status | Notes |
|---|---|---|---|---|
| **SCANS** | | | | |
| 1 | `ScanNode` | `PhysicalScan` in `scan_filter/scan.rs` | **FULLY IMPLEMENTED** | Arrow-array fast path + Vec<Value> fallback; supports semi-mask SIP, FTS query, and predicate push-down |
| 2 | `ScanRel` | `PhysicalScanRel` in `scan_filter/scanrel.rs` | **FULLY IMPLEMENTED** | Reads column-major data from rel tables; direction metadata carried |
| 3 | `VectorSimilarityScan` | `PhysicalVectorSimilarityScan` in `write_ops/vectorsimilarityscan.rs` | **FULLY IMPLEMENTED** | HNSW index search, top-K nearest neighbors, output includes distance column |
| 4 | `ArtIndexRangeScan` | `PhysicalArtIndexRangeScan` in `write_ops/copyfrom.rs` | **FULLY IMPLEMENTED** | ART index range scan, chunked output |
| 5 | `IndexLookup` | `PhysicalIndexLookup` in `index_lookup.rs` | **FULLY IMPLEMENTED** | Point lookup via ART index on PK column |
| 6 | `ExpressionsScan` | -- (inline in `map_scan.rs`) | **STUB** | Returns empty `DataChunk`; correlated variable scanning not wired up |
| 7 | `PathPropertyProbe` | `PhysicalPathPropertyProbe` in `scan_filter/pathpropertyprobe.rs` | **FULLY IMPLEMENTED** | Resolves properties on path-typed results from node/rel tables |
| **FILTER / PROJECTION** | | | | |
| 8 | `Filter` | `PhysicalFilter` in `scan_filter/filter.rs` | **FULLY IMPLEMENTED** | Expression evaluator + legacy bool-mask path; Arrow-native selection vector |
| 9 | `Projection` | `PhysicalProjection` in `scan_filter/projection.rs` | **FULLY IMPLEMENTED** | Column-index extraction; falls through to expression evaluator for computed columns |
| 10 | `Flatten` | `PhysicalFlatten` in `scan_filter/flatten.rs` | **FULLY IMPLEMENTED** | Pass-through (simplified from C++ factorization rewrite) |
| 11 | `Unwind` | `PhysicalUnwind` in `write_ops/unwind.rs` | **FULLY IMPLEMENTED** | Expands list expression into rows |
| 12 | `SemiMasker` | `PhysicalSemiMasker` in `types.rs` | **FULLY IMPLEMENTED** | Collects node offsets into semi-mask for SIP optimization |
| **SORT / LIMIT / AGGREGATE** | | | | |
| 13 | `OrderBy` | `PhysicalOrderBy` in `order_aggregate/orderby.rs` | **FULLY IMPLEMENTED** | Full sort with BlockMergeSort for large data, radix-sort for Int64 keys |
| 14 | `TopK` | `PhysicalTopK` in `order_aggregate/topk.rs` | **FULLY IMPLEMENTED** | BinaryHeap fused ORDER BY + LIMIT (O(n log k)) |
| 15 | `Limit` | `PhysicalLimit` in `scan_filter/limit.rs` | **FULLY IMPLEMENTED** | Offset + limit with slice/truncation |
| 16 | `Skip` | `PhysicalSkip` in `misc.rs` | **FULLY IMPLEMENTED** | Row offset pass-through |
| 17 | `Aggregate` | `PhysicalAggregateScan` + `PhysicalAggregateFinalize` in `order_aggregate/splitaggregation.rs` | **FULLY IMPLEMENTED** | Two-phase: sharded parallel accumulation + merge; supports COUNT, SUM, AVG, MIN, MAX, COLLECT, STDDEV, VARIANCE, PERCENTILE |
| 18 | `CountRelTable` | `PhysicalCountRelTable` in `write_ops/ddl_fts.rs` | **FULLY IMPLEMENTED** | Direct edge count from rel table metadata |
| **JOINS** | | | | |
| 19 | `HashJoin` | `PhysicalHashJoin` in `join_ops.rs` | **FULLY IMPLEMENTED** | Parallel build with `JoinHashTable`, ahash acceleration, semi-mask support |
| 20 | `SemiJoin` | `PhysicalSemiJoin` in `join_ops.rs` | **FULLY IMPLEMENTED** | Hash-set based; left-columns only output |
| 21 | `AntiJoin` | `PhysicalAntiJoin` in `join_ops.rs` | **FULLY IMPLEMENTED** | Inverse of SemiJoin |
| 22 | `Intersect` | `PhysicalIntersect` in `join_ops.rs` | **FULLY IMPLEMENTED** | Multiple build hash tables, pairwise intersection |
| 23 | `CrossProduct` | `PhysicalCrossProduct` in `join_ops.rs` | **FULLY IMPLEMENTED** | Cartesian product, row-by-row expansion |
| 24 | `OptionalMatch` | -- (inline in `map_join.rs`) | **FULLY IMPLEMENTED** | Uses `merge_optional_chunks` helper |
| **DML / UPDATES** | | | | |
| 25 | `Set` | `PhysicalSet` in `write_ops/set.rs` | **FULLY IMPLEMENTED** | Updates cells in node/rel tables |
| 26 | `Delete` | `PhysicalDelete` in `write_ops/delete.rs` | **FULLY IMPLEMENTED** | Row/edge deletion with detach support |
| 27 | `CreateNode` | `PhysicalInsertNode` in `write_ops/insert.rs` | **FULLY IMPLEMENTED** | Row-level node creation |
| 28 | `CreateRel` | `PhysicalInsertRel` in `write_ops/insert.rs` | **FULLY IMPLEMENTED** | Row-level rel creation |
| 29 | `Extend` | `PhysicalExtend` in `write_ops/recursiveextend.rs` | **FULLY IMPLEMENTED** | Adjacency-based extend with rel + dest node property lookups |
| 30 | `Merge` | `PhysicalMerge` in `write_ops/merge.rs` | **FULLY IMPLEMENTED** | Match-or-create with ON MATCH/ON CREATE SET |
| 31 | `Insert` | `PhysicalInsert` in `misc.rs` | **FULLY IMPLEMENTED** | Row-level insertion (legacy path) |
| 32 | `CopyFrom` | `PhysicalCopyFrom` in `write_ops/copyfrom.rs` | **FULLY IMPLEMENTED** | CSV/Parquet file loading, batch insert |
| 33 | `BatchInsert` | `PhysicalBatchInsert` in `batch_insert.rs` | **FULLY IMPLEMENTED** | Pre-collected row/rel batch insertion |
| **RECURSIVE / PATH** | | | | |
| 34 | `RecursiveExtend` | `PhysicalRecursiveExtend` in `write_ops/recursiveextend.rs` | **FULLY IMPLEMENTED** | BFS (unweighted) + Dijkstra (weighted) traversal; path reconstruction; WALK/TRAIL/ACYCLIC semantics |
| 35 | `PackedExtend` | `PhysicalPackedExtend` in `write_ops/packedextend.rs` | **FULLY IMPLEMENTED** | Multi-rel extend with CSR/flat neighbor output |
| **DDL** | | | | |
| 36 | `CreateNodeTable` | -- (returns empty chunk in `map_ddl.rs`) | **STUB — No-Op** | Returns empty success chunk; no actual catalog creation |
| 37 | `CreateRelTable` | -- (returns empty chunk in `map_ddl.rs`) | **STUB — No-Op** | Same as above |
| 38 | `DropTable` | -- (returns empty chunk in `map_ddl.rs`) | **STUB — No-Op** | Same as above |
| 39 | `AlterTable` | -- (returns empty chunk in `map_ddl.rs`) | **STUB — No-Op** | Same as above |
| 40 | `CreateIndex` | -- (returns empty chunk in `map_ddl.rs`) | **STUB — No-Op** | Same as above |
| 41 | `DropIndex` | -- (returns empty chunk in `map_ddl.rs`) | **STUB — No-Op** | Same as above |
| 42 | `CreateVectorIndex` | -- (returns empty chunk in `map_ddl.rs`) | **STUB — No-Op** | Same as above |
| 43 | `CreateSequence` | -- (returns empty chunk in `map_ddl.rs`) | **STUB — No-Op** | Same as above |
| 44 | `DropSequence` | -- (returns empty chunk in `map_ddl.rs`) | **STUB — No-Op** | Same as above |
| 45 | `CreateDml` | -- (returns empty chunk in `map_ddl.rs`) | **STUB — No-Op** | Same as above |
| 46 | `ExportDatabase` | -- (returns empty chunk in `map_ddl.rs`) | **STUB — No-Op** | Same as above |
| 47 | `ImportDatabase` | -- (returns empty chunk in `map_ddl.rs`) | **STUB — No-Op** | Same as above |
| **FTS** | | | | |
| 48 | `CreateFtsIndex` | `PhysicalCreateFtsIndex` in `write_ops/ddl_fts.rs` | **FULLY IMPLEMENTED** | Tokenizes, builds macro tables (docs, terms, appears_in) with BM25-ready stats |
| 49 | `FtsScan` | `PhysicalFtsScan` in `write_ops/ddl_fts.rs` | **FULLY IMPLEMENTED** | BM25 scoring, ranked (doc_id, score) output |
| **MISC** | | | | |
| 50 | `EmptyResult` | `PhysicalEmptyResult` in `misc.rs` | **FULLY IMPLEMENTED** | Returns empty vector |
| 51 | `MultiplicityReducer` | `PhysicalMultiplicityReducer` in `misc.rs` | **FULLY IMPLEMENTED** | Row dedup using HashSet<debug_string> |
| 52 | `Union` | `PhysicalUnion` in `missing_ops.rs` (unused) | **INLINE in mapper** | Handled directly in `PlanMapper::map_and_execute` rather than via PhysicalUnion struct |
| 53 | `Accumulate` | `PhysicalAccumulate` in `missing_ops.rs` | **FULLY IMPLEMENTED** | Materializes all input into a single contiguous chunk |
| 54 | `Partitioner` | `Partitioner` in `missing_ops.rs` | **FULLY IMPLEMENTED** | Splits chunks into morsels for parallel processing |
| 55 | `Explain` | `PhysicalExplain` in `write_ops/physicalexplain.rs` | **FULLY IMPLEMENTED** | Serializes plan tree to string |
| 56 | `Foreach` | `PhysicalForeach` in `write_ops/foreach.rs` | **FULLY IMPLEMENTED** | Iterates over list elements and executes sub-plans |
| 57 | `TableFunctionCall` | -- (delegated in `processor/mod.rs`) | **PARTIALLY IMPLEMENTED** | Only `vector_similarity_scan` has custom handling; other table functions error via `execute_table_function()` |
| 58 | `StandaloneCall` | `PhysicalStandaloneCall` in `write_ops/standalonecall.rs` | **FULLY IMPLEMENTED** | Dispatches to registered handler |
| 59 | `ExtensionClause` | `PhysicalExtensionClause` in `misc.rs` | **FULLY IMPLEMENTED** | Logs INSTALL/LOAD/UNINSTALL actions |
| 60 | `PrimaryKeyScan` | `PhysicalPrimaryKeyScan` in `scan_filter/primarykeyscan.rs` | **FULLY IMPLEMENTED** | Batched point lookup via hash index |

---

### Summary Counts

| Category | Count |
|---|---|
| **LogicalOperator variants total** | **60** |
| **Fully implemented physical operators** | **45** (75%) |
| **Partially implemented / stubs** | **3** (5%) |
| **No-op stubs (DDL returning empty chunk)** | **12** (20%) |

### Stubs / Missing Implementation Details

**1. DDL operators -- 12 no-op stubs** (in `map_ddl.rs`, lines 47-73):
All of these return either a single-row empty chunk or a zero-row empty chunk with no side effects:
- `CreateNodeTable`
- `CreateRelTable`
- `DropTable`
- `AlterTable`
- `CreateIndex`
- `DropIndex`
- `CreateVectorIndex`
- `CreateSequence`
- `DropSequence`
- `CreateDml`
- `ExportDatabase`
- `ImportDatabase`

**2. `ExpressionsScan`** (in `map_scan.rs`, line 175-177):
Simply returns `Ok(vec![DataChunk::new(vec![], vec![])])`. The C++ version (`LogicalExpressionsScan`) reads correlated variables from an outer accumulate context -- this is not wired up in Rust.

**3. `TableFunctionCall`** (in `processor/mod.rs`, `execute_table_function`):
- Only `vector_similarity_scan` has a concrete implementation
- `ScanCsv`, `ScanParquet`, `ScanJson`, `ListTables`, `ShowColumns`, `CurrentSetting` all return an error: "cannot be executed dynamically (no callback)"
- Other `Custom` table functions also error with "no registered handler"

**4. `Union`** - The `PhysicalUnion` struct exists in `missing_ops.rs` but its `execute()` is a no-op (`Ok(Vec::new())`). Union logic is handled inline in the mapper's `map_and_execute` method via `merge_union_chunks`.

### No `todo!()` or `unimplemented!()` calls exist in any physical operator code. The only `unreachable!()` found is in the `expression_evaluator.rs` helper (a match arm guard), not in operators.

---

### Key Architectural Observations

1. **Three operator dispatch categories exist for operators without dedicated structs:**
   - DDL no-ops (12 operators) -- `map_ddl.rs` returns empty chunks
   - Inline execution (Union, OptionalMatch, ExpressionsScan) -- handled directly in the mapper
   - Delegated execution (TableFunctionCall) -- routed through `QueryProcessor::execute_table_function`

2. **The physical operator execution model uses `PhysicalOperatorExec` trait** with a single `execute(input: Vec<DataChunk>) -> OperatorResult` method, unlike C++ Kuzu which uses a push-based pipeline model with `SourceOperator` / `SinkOperator` / `Operator` hierarchy.

3. **Parallelism** is achieved via `rayon` in `JoinHashTable::build_parallel()` and `AggregateHashTable::aggregate_parallel()`, plus the `Partitioner` operator splits work into morsels.

4. **The `missing_ops.rs` file name is misleading** -- it actually contains *implemented* operators (PhysicalAccumulate, Partitioner, Profile, etc.), not stubs for missing C++ operators.