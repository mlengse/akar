# Optimizer (akar-optimizer)

**Module path:** `akar-core/akar-optimizer/`
**Role:** Core domain — the query-rewrite brain between plan and execution.

---

## Overview

The optimizer is where a naive logical plan becomes a clever one. The planner has produced a plan that *works*; the optimizer then applies two phases of rewrites to make it *fast*: 19 flat passes over the operator list, then 7 tree passes over each operator subtree. Think of it as an editor who takes a draft manuscript and applies a sequence of well-understood copy-edits — push filters down toward the scans where they cost less, fold constant expressions, detect "top-K" requests and give them a faster operator, spot a vector-similarity predicate and swap in an HNSW scan, or rewrite FTS predicates onto the base-table scan.

`Optimizer` (`akar-optimizer/src/optimizer.rs:15-18`) holds the two pass vectors; `Optimizer::new()` registers the standard chain (asserted to be exactly 26 passes by a test at `optimizer.rs:203`), and `with_stats_and_fts()` adds storage-backed cardinality estimation plus an optional FTS selectivity estimator. This is the stage where Akar exceeds its C++ ancestor: 26 passes vs. C++'s 17.

## Core functions

1. **Register passes** — `Optimizer::new()` (`optimizer.rs:21-81`) builds the chain of 19 flat + 7 tree passes.
2. **Stats-aware optimizer** — `with_stats_and_fts(stats, fts_estimator)` (`optimizer.rs:93-132`) wires `CardinalityEstimation::new(Some(stats)).with_fts_estimator(...)` (P108.2).
3. **Optimize** — `Optimizer::optimize(&self, operators)` (`optimizer.rs:134-152`) runs flat passes (Phase 1) then tree passes over each top-level operator (Phase 2).
4. **Pass traits** — flat passes implement `OptimizationPass::apply(&[LogicalOperator]) -> Vec<LogicalOperator>` (`passes/mod.rs:20-23`); tree passes implement `TreeOptimizationPass::apply_tree(&mut LogicalOperator)` (`passes/mod.rs:30-35`).

**Flat passes** (`optimizer.rs:22-63`): RemoveUnnecessaryOperators, ExtendFilterPushDown, FilterPushDown, PredicatePushDown, ProjectionPushDown, ConstantFolding, AggregateDetection, JoinOptimization, TopKOptimization, VectorSimilarityDetection, ArtRangeScanDetection, LimitPushDown, CommonSubexpressionElimination, OrderByPushDown, UnwindDedup, CountRelTable, AggregateFusion, SortElision, ExpressionInline.

**Tree passes** (`optimizer.rs:64-79`): FactorizationRewriting, ForeignJoinPushDown, AccHashJoinOptimization, CorrelatedSubqueryUnnesting, AggKeyDependency, CardinalityEstimation, FtsPredicatePushdown.

## Key components

| Component/type | File path | One-line responsibility |
|---|---|---|
| `Optimizer` | `src/optimizer.rs:15-18` | Pass-chain orchestrator (flat + tree vectors) |
| `OptimizationPass` trait | `src/passes/mod.rs:20-23` | Flat rewrite over operator list |
| `TreeOptimizationPass` trait | `src/passes/mod.rs:30-35` | In-place bottom-up tree rewrite |
| `FilterPushDown` | `src/passes/flat/filter_pushdown.rs:11-42` | Move filters toward scans |
| `PredicatePushDown` | `src/passes/flat/predicate_pushdown.rs:15` | Fold predicates onto `ScanNode` |
| `ExtendFilterPushDown` | `src/passes/flat/extend_filter_pushdown.rs:34` | Hoist source-property filters above `Extend` before hops (F3) |
| `VectorSimilarityDetection` | `src/passes/flat/vector_similarity.rs:47` | Detect `cos > thr ORDER BY DESC LIMIT k` → `VectorSimilarityScan` (HNSW read) |
| `ArtRangeScanDetection` | `src/passes/flat/art_range_scan.rs:23` | Detect PK range scans on the ART index |
| `TopKOptimization` | `src/passes/flat/top_k.rs:9` | Merge ORDER BY + LIMIT into `TopK` |
| `AggregateFusion` | `src/passes/flat/aggregate_fusion.rs:19` | Merge consecutive aggregates with same GROUP BY |
| `CardinalityEstimation` | `src/passes/tree/cardinality.rs:25` | Annotate operators with estimated row counts |
| `FtsPredicatePushdown` | `src/passes/tree/fts_predicate_pushdown.rs:21` | Route `USING FTS INDEX` predicates onto the base-table scan (P108) |
| `FtsCardinalityEstimator` trait | `src/fts_estimate.rs:15-18` | Estimate FTS match counts for cardinality |

## Internal data flow

```mermaid
flowchart LR
    A["Vec<LogicalOperator><br/>from planner"] --> B["Phase 1: flat passes<br/>each apply(&result)"]
    B --> C["Phase 2: tree passes<br/>apply_tree per top-level op"]
    C --> D["CardinalityEstimation<br/>StatsStore + FTS estimator"]
    D --> E["optimized plan<br/>to akar-processor"]
```

`CardinalityEstimation` consults `StatsStore` (when configured) and the optional FTS selectivity estimator to stamp `cardinality` estimates on operators; without stats it falls back to static heuristics (`EQUALITY_PREDICATE_SELECTIVITY = 0.01`, `passes/tree/cardinality.rs:15`), and without an FTS estimator FTS scans are estimated at full table size (`optimizer.rs:91-92`).

## Key interfaces & extension points

`OptimizationPass` and `TreeOptimizationPass` are the two stable extension points: implement either trait and insert it in `Optimizer::new()`/`with_stats_and_fts()`. Plugin-in stats: `with_stats(stats)` (`optimizer.rs:84`) consumes the read-only `StatsStore` (`akar-storage/src/stats`). Two passes are documented **active** rewrites of note: `VectorSimilarityDetection` (P71.4, with in-file safety invariants at `vector_similarity.rs:1-37`) and `FtsPredicatePushdown` (P108) — both reshape expensive join-time work into index reads. Several passes are documented **NO-OP by design** (CommonSubexpressionElimination, OrderByPushDown, AggregateFusion) because the rewrites they'd perform are not provably correct under UNION concat semantics without new operators — see the SPEC.

## Interactions with other modules

| Module | Direction | Interface used | Note |
|---|---|---|---|
| akar-planner | input | `LogicalOperator` (59 variants) | Plan source |
| akar-storage | uses | `StatsStore`, table metadata | Cardinality, ART/vector reasoning |
| akar-processor | consumer | Optimized plan | Processor maps to physical ops |
| akar-function/vector | indirect | Cosine-similarity shape | Signals `VectorSimilarityDetection` |

## Performance & concurrency notes

All passes are single-threaded sequential rewrites (no threading inside the optimizer). The per-file `Pass N` doc comments reflect the pass design numbers, not the executable order in `Optimizer::new()` — rely on `pass_names()` as ground truth. `CardinalityEstimation` costs nothing when no `StatsStore` is configured (static-heuristics path), keeping default planning cheap.

## Implementation highlights

- **Vector ANN detection (P71.4):** pattern-matches `ScanNode(pred=cos>thr) + OrderBy(cos DESC) + Limit(k)` and rewrites to `[VectorSimilarityScan(column, query_vector, k), Filter(>thr)]`, preserving downstream Projection/ORDER BY/LIMIT. This single pass is what makes vector search "just work" from ordinary Cypher.
- **FTS-aware planning (P108):** the binder attaches `USING FTS INDEX` to MATCH, the planner emits `FtsScan`, and `FtsPredicatePushdown` re-routes it onto the base-table scan so index read and predicate land together (`passes/tree/fts_predicate_pushdown.rs:21`).
- Ladybug micro-optimizations (`ladybug.rs`: OrderByPushDown `:18`, UnwindDedup `:121`, CountRelTable `:166`) are bundled here.
- The pass-count test `test_optimizer_registers_all_passes` (`optimizer.rs:203`) pins the 26-pass invariant, so a new pass can't silently break the documented chain.