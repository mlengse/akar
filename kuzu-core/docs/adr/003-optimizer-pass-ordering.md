# ADR 003: Optimizer Pass Ordering

> **Status:** Accepted | **Date:** 2026-07-07 | **Last Updated:** 2026-07-19

## Context

Query optimizer memiliki 22 pass (15 flat + 7 tree). Urutan eksekusi pass mempengaruhi kualitas final plan.

## Decision

**Flat passes dulu, baru tree passes.** Flat passes berjalan secara bottom-up pada plan tree; tree passes berjalan secara rekursif dari root.

### Flat Pass Order (15 passes)

1. `RemoveUnnecessaryOperators` — bersihkan dulu
2. `FilterPushDown` — push filter sedekat mungkin ke scan
3. `PredicatePushDown` — merge filter predicate ke dalam scan predicate
4. `ProjectionPushDown` — eliminasi kolom yang tidak digunakan
5. `ConstantFolding` — evaluasi konstanta
6. `AggregateDetection` — deteksi batas agregasi
7. `JoinOptimization` — DP Bushy Trees join reordering
8. `TopKOptimization` — ORDER BY + LIMIT → TopK scan
9. `VectorSimilarityDetection` — deteksi pola vector similarity
10. `ArtRangeScanDetection` — deteksi ART index range scan
11. `LimitPushDown` — push limit ke bawah
12. `CommonSubexpressionElimination` — eliminasi duplikat
13. `OrderByPushDown` — push ORDER BY di bawah UNION ALL
14. `UnwindDedup` — dedup UNWIND berturut-turut
15. `CountRelTable` — ScanRel+COUNT → CSR metadata

### Tree Pass Order (7 passes)

1. `FactorizationRewriting` — sisipkan Flatten untuk hash join
2. `ForeignJoinPushDown` — push foreign join
3. `AccHashJoinOptimization` — optimasi accumulated hash join
4. `SIPOptimization` — Sideways Information Passing
5. `CorrelatedSubqueryUnnesting` — unnest correlated subquery
6. `AggKeyDependency` — hapus grouping key redundant
7. `CardinalityEstimation` — annotate row count estimates

## Rationale

- Flat passes diprioritaskan karena mengubah bentuk plan (restructuring)
- Tree passes lebih mahal (traversal rekursif), dijalankan setelah struktur stabil
- Join reordering (flat pass 6) harus sebelum factorization (tree pass 1)
- Cardinality estimation dijalankan terakhir karena butuh plan final
