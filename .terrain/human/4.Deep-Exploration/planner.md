# Deep Exploration — akar-planner

The planner turns a bound statement into a logical plan: a DAG built from 59 logical operator variants (ScanNodeTable, Filter, Projection, HashJoin, OrderBy, Aggregation, Distinct, Limit, Copy, StandaloneCall, VectorSimilarityScan, ...). It also performs one critical rewrite that touches the memory feature set: converting a `cosine_similarity(col, q) >= thr ORDER BY ... LIMIT k` predicate into a dedicated vector-similarity scan plan.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `LogicalPlan` | DAG of 59 logical operator types | `akar-core/akar-planner/src/plan.rs` |
| `LogicalExpression` | Expressions represented at plan level | `akar-core/akar-planner/src/` |
| VectorSimilarityDetection | Rewrites `cosine_similarity(x,y) >/= thr` matches into VectorSimilarityScan(HNSW) sub-plans | `akar-core/akar-planner/src/planner.rs:211` |
| join ordering / pattern matching | Match → node table scans + rel hash joins | `akar-core/akar-planner/src/` |

## Design Decisions

- **Logical plan as the optimizer's input.** Keeping planning separate from optimization allows the 24-pass optimizer (`akar-core/akar-optimizer/src/`) to traverse and rewrite a stable form. This mirrors Kuzu's architecture and keeps parity (ADR-003).
- **Vector rewrite lives at plan time.** The alternative — a post-filter over a full ANN scan — was rejected because the HNSW graph already returns the k nearest rows; filtering after is redundant work. Encoding that decision in the planner (vs optimizer) keeps optimizer passes generic and lets the processor pick the dedicated physical op (`akar-core/akar-processor/src/processor/vector_similarity_scan.rs`).

## Why It Matters

Query shape decisions are made here: whether a MATCH becomes a node scan + filter scan or an index lookup; whether `cosine_similarity` becomes an HNSW search; whether COPY routes to local storage or a Lakehouse extension. Errors in planning produce wrong or dramatically slower plans. Parity tests compare planned plans against Kuzu's expected plan shapes.