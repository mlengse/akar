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

**Tree passes (7):**
- `FactorizationRewriting` — inserts Flatten operators before HashJoin/Projection/Aggregate/OrderBy/Limit for correct factorization. Ported from C++ (153 lines).
- `ForeignJoinPushDown` — pushes foreign joins through operators
- `AccHashJoinOptimization` — optimizes accumulated hash joins
- `SIPOptimization` — Sideways Information Passing via SemiMasker
- `CorrelatedSubqueryUnnesting` — unnests correlated subqueries
- `AggKeyDependency` — removes redundant grouping keys
- `CardinalityEstimation` — annotates operators with estimated row counts using static selectivity heuristics. Ported from C++ (120 lines).

**Total: 25 passes (18 flat + 7 tree) — exceeds C++ Ladybug (17).**

**Join order enumeration:**
- `reorder_joins_greedy()` — tree-based greedy reordering by cardinality
- `reorder_joins_greedy_first()` — flat-list entry point

**Tests:** 61
