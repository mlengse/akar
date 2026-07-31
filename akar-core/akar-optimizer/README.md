# Akar Optimizer

Query optimization passes that transform the logical plan for better performance.

**Flat passes (6):**
- `RemoveUnnecessary` — eliminates redundant operators
- `FilterPushDown` — pushes filters closer to scan nodes
- `ProjectionPushDown` — eliminates unused columns early
- `ConstantFolding` — evaluates constant expressions at plan time
- `JoinOptimization` — removes redundant join conditions
- `TopK` — converts OrderBy + Limit to an efficient TopK scan

**Tree passes (2):**
- `FactorizationRewriting` — inserts Flatten operators before HashJoin/Projection/Aggregate/OrderBy/Limit for correct factorization. Ported from C++ (153 lines).
- `CardinalityEstimation` — annotates operators with estimated row counts using static selectivity heuristics. Ported from C++ (120 lines).

**Join order enumeration:**
- `reorder_joins_greedy()` — tree-based greedy reordering by cardinality
- `reorder_joins_greedy_first()` — flat-list entry point

**Tests:** 42
