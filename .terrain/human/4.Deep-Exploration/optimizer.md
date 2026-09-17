# Deep Exploration — akar-optimizer

The optimizer rewrites a logical plan into a physical plan through 24 ordered passes. The first 18 run over the flat plan (before joins are assembled into a DAG), the remaining 6 over the tree structure. Passes include predicate pushdown (`filter_pushdown`, `operator_pruning`, `bloom_filter_pushdown`), projection pruning (`column_pruning`, `projection_pushdown`), index selection (`index_lookup_scan`, `order_by_index_scan`, `primary_key_lookup`, `index_lookup_join`), and tree-level rewrites (`aggregate_fusion`, `order_by_elision`, `sort_elision`, `expression_inline`).

## Key Components

| Pass group | Examples | Purpose |
|-----------|----------|---------|
| Predicate pushdown | `filter_pushdown`, `operator_pruning`, `bloom_filter_pushdown`, `is_undefined_to_null` | Push filters/limitations toward scans, drop dead branches |
| Projection pruning | `column_pruning`, `projection_pushdown`, `get_factorization` | Only materialize needed columns; factor common subexpressions |
| Index & join selection | `index_lookup_scan`, `order_by_index_scan`, `primary_key_lookup`, `index_lookup_join`, `eliminate_cross_product` | Replace scans with index probes; avoid cartesian joins |
| Ordering | `order_by_combine`, `mark_order_by`, `order_by_elision`, `sort_elision` | Reuse existing orderings; drop redundant sorts |
| Aggregation fusion | `aggregate_fusion` (tree) | Fuse compatible aggregations |
| Inlining | `expression_inline` (tree) | Inline trivial expressions into parent |
| SIMD vector similarity | (planner+processor) | `VectorSimilarityScan` rewrite, post-filter removed at P71.4 |

Key source: `akar-core/akar-optimizer/src/lib.rs` (pass registry), pass list maintained in `akar-core/akar-optimizer/README.md`. Note: `SIPOptimization` was removed (P48.16) and `AggregateFusion`, `SortElision`, `ExpressionInline` were added (ADR-003), so the pass set is live and versioned.

## Design Decisions

- **Fixed ordered pass list (ADR-003).** Chosen over a Volcano-style cost-based optimizer. Fixed order is deterministic, testable, and easy to debug. The cost is some missed cross-pass opportunities that a rule engine would find — accepted for parity with Kuzu and simplicity.
- **Flat-then-tree split.** Flat passes (18) run before the tree is formed; tree passes (6) run afterwards, where fusion/elision require parent-child context.
- **Vector scan is not an optimizer pass.** The ANN rewrite is decided in the planner (`akar-core/akar-planner/src/planner.rs:211`) so the optimizer stays generic; the resulting physical op is `VectorSimilarityScan` (`akar-core/akar-processor/src/processor/vector_similarity_scan.rs`).

## Why It Matters

Optimizer output is the physical plan that determines performance — the difference between an indexed lookup and a full scan, between a sort and an index order probe, between filtering rows before versus after a join. Twenty-four passes give the engine most of what a commercial cache-based optimizer buys, at a fraction of the complexity.