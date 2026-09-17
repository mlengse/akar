# Deep Exploration — akar-search

`akar-search` fuses the vector and FTS channels into a single ranked result set. It defines hybrid recall over the two recall backends (HNSW ANN from `akar-vector`, BM25 from `akar-fts`) and merges channel scores with a weighted Reciprocal Rank Fusion pipeline. The fused pipeline also carries reranker scores so a downstream reranker (from `akar-ml`) can refine ordering.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `hybrid_search` | Fan-out to vector + FTS, weighted RRF fusion | `akar-core/akar-search/src/hybrid.rs` |
| `RerankFusedPipeline` | Combines own table + fused score + reranker score + original row | `akar-core/akar-search/src/fused.rs` |
| `ScoredRecordBatch` | Ranked result batches | `akar-core/akar-search/src/` |
| Python entry | `knn_fused_score(conn, table, col, q, k, alpha)` | `akar-core/akar-python/src/functions/knn_fused_score.rs` |

## Design Decisions

- **Weighted RRF over raw scores.** Reciprocal rank fusion merges two differently-scaled rankers (ANN distance vs BM25 score) without calibration. `alpha` trades vector vs lexical weight per use case, letting agents prefer semantic recall (high alpha) or exact-phrase recall (low alpha).
- **Fusion is a pipeline, not a one-off join.** `RerankFusedPipeline` keeps original rows + per-channel scores + fused score through the pipeline so a reranker can reorder — enabling the "retrieve-50-rank-5" cascade used by hybrid recall.

## Why It Matters

Single-channel recall misses. An agent that only uses vector search loses exact-token hits; only FTS loses paraphrase recall. Hybrid recall is the recommended memory-facing path, and both `knn_fused_score` (Python) and the C4/W8 workflow depend on it. Fused scores are also exposed back to callers for logging/telemetry.