# P27: Performance Recommendations

Based on the benchmarking and gap analysis from `BENCHMARK_COMPARISON.md`, here are the recommendations for **P27 (Performance)**:

## 1. JoinHashTable Tuning (High Priority)
The hash join build phase is exhibiting unexpected overhead:
- `join/10k_build_100_probe` (10,000 build rows, 100 probe rows) takes **11.8 ms**, while `join/1k_build_1k_probe` takes **1.44 ms**.
- The nearly 10x slow down for a 10x input size points to potential `O(n^2)` complexity in the bucket insertion or extremely high collision rates.
- **Recommendation**:
  - Investigate the hash function used for the keys (currently using `ahash` or `foldhash`).
  - Examine the bucket chain length during hash table construction. Switch to open addressing (e.g., SwissTable architecture) or explicitly pre-size the buckets based on the incoming `DataChunk` cardinality to avoid reallocations.

## 2. Hybrid Arrow Migration (Medium Priority)
The Phase 2 Arrow-native expression evaluation delivered 10-24x speedups for arithmetic and comparisons, but a remaining bottleneck is the `from_legacy()` conversion:
- Reading a variable from a `ValueVector` into an Arrow array currently incurs a conversion cost (24.5 µs vs 2.1 µs for 10K rows).
- **Recommendation**:
  - Implement Phase 3: Transition `DataChunk` fields to hold native `ArrayRef` from the `arrow` crate directly.
  - This avoids `from_legacy()` completely, making the expression evaluation purely Arrow-native end-to-end, closing the remaining gap to the C++ baseline.

## 3. OrderBy Sort-in-Place Optimization (Low/Medium Priority)
- `order_by/single_key_10k` takes ~1 ms. 
- The current implementation likely collects all rows into a `Vec<Value>` or similar buffer, then sorts, then rebuilds `DataChunk`s.
- **Recommendation**:
  - Implement an index-based `sort_in_place` which produces a sorted array of indices (row pointers).
  - Use Arrow's `take` kernel to materialize the final sorted `DataChunk`s rather than copying values during the sort phase.

## 4. Query Pipeline Caching (Low Priority)
- `query/match_return_all` for 5 rows takes **18.5 µs**, while the raw scan takes **11.9 µs**, meaning the `parse -> bind -> plan -> optimize` overhead is roughly 55%.
- **Recommendation**:
  - Introduce prepared statement caching at the connection level so repeated queries skip the frontend compilation phases.
