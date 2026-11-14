# Akar Optimizer

Query optimization passes that transform the logical plan for better performance.

## Pass Structure

**Flat passes (19)** — operate on `&[LogicalOperator]`, applied first to the entire plan:
1. `RemoveUnnecessaryOperators` — eliminates redundant operators
2. `FilterPushDown` — pushes filters closer to scan nodes
3. `PredicatePushDown` — pushes predicates below joins
4. `ProjectionPushDown` — eliminates unused columns early
5. `ConstantFolding` — evaluates constant expressions at plan time
6. `AggregateDetection` — detects aggregation boundaries
7. `JoinOptimization` — cardinality-aware join reordering
8. `TopKOptimization` — converts OrderBy + Limit to efficient TopK scan
9. `VectorSimilarityDetection` — detects vector similarity patterns
10. `ArtRangeScanDetection` — detects ART index range scan patterns
11. `LimitPushDown` — pushes limits closer to scans
12. `CommonSubexpressionElimination` — eliminates duplicate expressions
13. `OrderByPushDown` — pushes ORDER BY below UNION ALL
14. `UnwindDedup` — deduplicates consecutive UNWIND
15. `CountRelTable` — replaces ScanRel+COUNT with CSR metadata
16. `AggregateFusion` — **NO-OP since P52.7** (stub returns operators unchanged; merging chained aggregates is unsound)
17. `SortElision` — eliminates redundant sorts
18. `ExpressionInline` — inlines trivial expressions
19. `ExtendFilterPushDown` — extends filter pushdown through additional operators
20. `FtsPredicatePushdown` — pushes FTS predicates below scans (P108.2)

**Tree passes (7)** — operate on the operator tree in-place via bottom-up traversal (applied after flat passes):
1. `CardinalityEstimation` — annotates operators with estimated row counts using static selectivity heuristics
2. `FactorizationRewriting` — inserts Flatten operators before HashJoin/Projection/Aggregate/OrderBy/Limit for correct factorization
3. `AggregateDetection` — detects aggregates in projections and rewrites to explicit `Aggregate` operators
4. `VectorSimilarityDetection` — detects vector similarity patterns in tree context
5. `ArtRangeScanDetection` — detects ART index range scan patterns
6. `ForeignJoinPushDown` — pushes foreign joins through operators
7. `AccHashJoinOptimization` — optimizes accumulated hash joins

**Total: 26 passes (19 flat + 7 tree)**

## Disabled passes (3)

The following passes exist in the codebase but are **not registered** in the default `Optimizer::new()`:
- `CorrelatedSubqueryUnnesting` — unnests correlated subqueries
- `AggKeyDependency` — removes redundant grouping keys
- `SortElision` (tree variant) — eliminates redundant sorts in tree context

They can be enabled by constructing a custom `Optimizer` with `with_stats_and_fts` and manually registering the passes.

## Key Notes

- **`AggregateFusion` is a NO-OP** — The original pass merged two consecutive `Aggregate` operators with the same GROUP BY into one. This is only valid if the outer's arguments can be evaluated against the inner's RAW input, but they normally reference the inner aggregate's output (e.g., `COUNT(*) AS cnt` followed by `MAX(cnt)`), which disappears after fusion and resolves to NULL. Even for matching GROUP BY keys the fusion is wrong: `[Agg(gb, [SUM(x)]), Agg(gb, [COUNT(*)])]` reduces each name to a single row before the outer COUNT runs, whereas the fused COUNT(*) would count the raw rows. Merging chained aggregates requires rewriting the outer's arguments against a merged output schema — something this flat pass cannot express — so consecutive aggregates are left untouched.

- **Join order enumeration** (external utilities):
  - `reorder_joins_greedy()` — tree-based greedy reordering by cardinality
  - `reorder_joins_greedy_first()` — flat-list entry point

- **FTS selectivity estimator** — Optional `FtsCardinalityEstimator` refines scan cardinality for `USING FTS INDEX` queries (P108.2).

## Tests

80 tests covering: pass registration, individual pass behavior, empty plan handling, plan preservation, and integration with storage-backed cardinality estimation.