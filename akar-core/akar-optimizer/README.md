# Akar Optimizer

Query optimization passes that transform the logical plan for better performance.

**Flat passes (18):**
- `RemoveUnnecessaryOperators` — eliminates redundant operators
- `FilterPushDown` — pushes filters closer to scan nodes
- `PredicatePushDown` — pushes predicates below joins
- `ProjectionPushDown` — eliminates unused columns early
- `ConstantFolding` — evaluates constant expressions at plan time
- `AggregateDetection` — detects aggregation boundaries
- `JoinOptimization` — cardinality-aware join reordering
- `TopKOptimization` — converts OrderBy + Limit to an efficient TopK scan
- `VectorSimilarityDetection` — detects vector similarity patterns
- `ArtRangeScanDetection` — detects ART index range scan patterns
- `LimitPushDown` — pushes limits closer to scans
- `CommonSubexpressionElimination` — eliminates duplicate expressions
- `OrderByPushDown` — pushes ORDER BY below UNION ALL
- `UnwindDedup` — deduplicates consecutive UNWIND
- `CountRelTable` — replaces ScanRel+COUNT with CSR metadata
- `AggregateFusion` — fuses aggregate operations
- `SortElision` — eliminates redundant sorts
- `ExpressionInline` — inlines trivial expressions

**Tree passes (6):**
- `FactorizationRewriting` — inserts Flatten operators before
  HashJoin/Projection/Aggregate/OrderBy/Limit for correct factorization. Ported from C++
  (153 lines).
- `ForeignJoinPushDown` — pushes foreign joins through operators
- `AccHashJoinOptimization` — optimizes accumulated hash joins
- `CorrelatedSubqueryUnnesting` — unnests correlated subqueries
- `AggKeyDependency` — removes redundant grouping keys
- `CardinalityEstimation` — annotates operators with estimated row counts using static
  selectivity heuristics. Ported from C++ (120 lines).

**Total: 24 passes (18 flat + 6 tree) — exceeds C++ Ladybug (17).**
> P48.16 (2026-08-07): `SIPOptimization` removed - the injected `SemiMasker` was never
> applied at execution time (the Arrow fast path in `map_scan.rs` never reads a scan-side
> semi-mask; the legacy path checks the wrong column; the mask is only inserted into the
> context after the build-side scan already ran). A correct SIP needs scan-then-collect in
> a single pipeline, which is out of scope here; the kernel `NodeSemiMask` concept is
> retained for a future proper SIP pipeline.

**Join order enumeration:**
- `reorder_joins_greedy()` — tree-based greedy reordering by cardinality
- `reorder_joins_greedy_first()` — flat-list entry point

**Tests:** 80
