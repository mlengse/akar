# Akar Search Extension

Search fusion and hybrid recall for the Akar database engine.

**Functions:**
- `rrf_fuse` — Reciprocal Rank Fusion (RRF) for merging multiple ranked result sets
- `hybrid_search` — combine vector + full-text search results
- `multi_perspective_recall` — multi-channel recall with automatic RRF deduplication

**Usage pattern:**
```rust
use akar_search::rrf_fuse_ref;

let fused = rrf_fuse_ref(&ranked_lists, k);
```

**Tests:** 23
