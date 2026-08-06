# ADR 003: Optimizer Pass Ordering

> **Status:** Accepted | **Date:** 2026-07-07 | **Last Updated:** 2026-08-07

## Context

Query optimizer memiliki 22 pass (15 flat + 7 tree) saat ADR ditulis. Urutan eksekusi pass mempengaruhi kualitas final plan. **Update 2026-08-07:** saat ini 24 pass (18 flat + 6 tree) — `SIPOptimization` (tree pass) **dihapus di P48.16** karena semi-mask yang di-inject-nya tidak pernah diterapkan saat eksekusi (fast path Arrow scan tak membaca `semi_mask`; jalur legacy cek kolom yang salah; mask baru di-insert setelah scan build berjalan). `AggregateFusion`, `SortElision`, `ExpressionInline` (flat) ditambahkan setelah ADR ini. Urutan prinsip (flat dulu, tree terakhir) tetap berlaku.

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
4. `CorrelatedSubqueryUnnesting` — unnest correlated subquery
5. `AggKeyDependency` — hapus grouping key redundant
6. `CardinalityEstimation` — annotate row count estimates

## Rationale

- Flat passes diprioritaskan karena mengubah bentuk plan (restructuring)
- Tree passes lebih mahal (traversal rekursif), dijalankan setelah struktur stabil
- Join reordering (flat pass 6) harus sebelum factorization (tree pass 1)
- Cardinality estimation dijalankan terakhir karena butuh plan final
