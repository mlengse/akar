## 2025-09-13 - Single-pass vector cosine calculation optimization
**Learning:** In vector operations and HNSW distance computations, calculating dot product and vector norm sums using separate iterator passes (`.iter().zip()`, `.iter().map()`, etc.) causes multiple cache and memory traversal sweeps over array slices. Consolidating dot product and magnitude square accumulators into a single loop pass provides ~2.95x speedup for cosine distance and similarity calculations on high-dimensional vectors (e.g. 1536-dim embeddings).
**Action:** Always combine vector product and norm calculations into single-pass loops in distance/similarity metrics or hot path vector algorithms to reduce iteration overhead and improve cache locality.

## 2025-09-13 - O(1) HNSW node lookup optimization via HashMap
**Learning:** Using `BTreeMap` to index graph nodes in `HnswIndex` introduced $O(\log N)$ tree traversal overhead on every node lookup during greedy graph descent and beam search inner loops. Switching to `HashMap` and caching `&HnswNode` references during greedy descent reduces node lookup overhead from $O(\log N)$ to $O(1)$.
**Action:** Use `HashMap` instead of `BTreeMap` for graph node storage in vector indexes and HNSW graphs, and cache node references across loop iterations in hot graph traversal algorithms.

## 2026-09-15 - HNSW beam search pre-allocation and slice iteration optimization
**Learning:** In HNSW graph search and batch vector similarity metrics, default collection allocations (`HashSet::new()`, `BinaryHeap::new()`, `.clone()`) trigger multiple dynamic heap re-allocations during beam expansion. Pre-allocating visited sets and candidate heaps with `with_capacity(ef * 2)` and iterating over connection slices instead of cloning `Vec<usize>` eliminates reallocations and vector copies on hot search paths. Additionally, popping max-heap results and reversing the slice extracts sorted nearest neighbours in $O(N)$ without general sorting overhead.
**Action:** Pre-allocate candidate heaps and visited sets based on `ef` beam size, pass connection slices by reference during neighbor updates, and extract max-heap search candidates via pop-and-reverse.

## 2026-09-16 - Fast-path HNSW neighbor connection updates
**Learning:** During HNSW graph construction, updating neighbor reverse connections before maximum connection degree (`max_conn`) is reached does not require recomputing distances across all existing neighbors. Adding a fast-path append avoids up to 32 vector distance calculations per neighbor update. Graph-traversal visited sets should use a fast integer hasher (AHashSet from `ahash`) rather than the SipHash-backed default.
**Action:** Fast-path connection updates on nodes below degree capacity during graph construction, and use `AHashSet` for visited node sets in graph search algorithms.

## 2026-09-17 - Zero-allocation HNSW neighbor selection and in-place pruning
**Learning:** HNSW beam search (`search_layer_0`) already returns nearest candidates pre-sorted in ascending distance order. Performing additional sorting in `select_neighbors_simple` causes redundant allocations and $O(N \log N)$ sorting per insertion layer. Furthermore, when a neighbor node reaches connection degree capacity (`max_conn`), updating its connections by allocating a new vector and sorting all $M+1$ candidates is wasteful; finding the max-distance connection in a single linear pass and replacing it in-place eliminates heap allocations during graph construction.
**Action:** Rely on pre-sorted beam search results for neighbor selection and perform in-place max-distance replacements when connection degree capacity is reached.

## 2026-09-18 - Hash-based list membership lookup optimization
**Learning:** Performing list membership checks (`ListOp::HasAny` and `ListOp::HasAll`) using nested linear loops (`Vec::contains`) results in $O(N \times M)$ runtime complexity. Adding a lightweight non-owning reference wrapper (`ValueRef`) implementing `Hash` and `Eq` for `Value` enables building a `hashbrown::HashSet` on lists larger than 8 elements. This reduces search time from $O(N \times M)$ to $O(N + M)$, speeding up large list membership checks by over 50x-100x while avoiding dynamic allocation overhead on small lists (<= 8 elements).
**Action:** Use `HashSet` with non-owning reference wrappers for set operations over complex enum values (such as `Value`) when list sizes exceed small linear search thresholds (e.g., > 8 elements).

## 2026-09-19 - Generation-stamped scratch buffer optimization in graph propagation
**Learning:** In graph activation algorithms (`compute_spread_activation`), instantiating `BTreeMap<usize, f64>` on every propagation hop triggers dynamic heap allocations and $O(\log K)$ tree node insertion/search overhead per edge. Hoisting scratch vectors (`next_act_scratch`, `stamp`, `touched`, `next_frontier`) outside the hop loop and using generation-stamping (`stamp[w] == hop`) reduces node aggregation to $O(1)$ indexed array lookup. Sorting `touched` preserves deterministic node ID iteration order without tree structure overhead.
**Action:** Hoist scratch buffers outside iteration loops and use generation-stamped arrays with sorted node indices for graph propagation and frontier-expansion algorithms.

## 2026-09-20 - PageRank buffer reuse and precomputed degree lookups
**Learning:** In iterative graph algorithms like PageRank power iteration, re-allocating iteration state vectors (`vec![base; n]`) and performing graph neighbor slice lookups (`csr.neighbors(i)`) on every iteration introduces significant heap allocation churn and redundant $O(N)$ graph degree checks per iteration. Precomputing node degrees and dangling node indices once before entering power iterations, combined with buffer reuse (`.fill(base)`) and $O(1)$ buffer swapping (`std::mem::swap`), eliminates all heap allocations during power iteration loops and avoids $O(N)$ slice lookups for dangling node checks.
**Action:** Precompute static graph topology properties (degrees, dangling nodes) outside iterative graph loops, hoist state vectors outside the iteration loop, and use `std::mem::swap` to update iteration state in $O(1)$ time.

## 2026-09-21 - Scratch buffer hoisting and invariant hoisting in biased random walks
**Learning:** In Node2Vec and biased random walk generation, allocating a candidate `weights` vector (`Vec::with_capacity(neighbors.len())`) and recomputing division constants (`1.0 / p`, `1.0 / q`) or re-querying `csr.neighbors(prev)` inside the walk step loop causes millions of transient heap allocations and redundant CSR lookup sweeps per node. Hoisting scratch vectors outside step loops (`weights.clear()`), precomputing inverse probabilities, pre-fetching `csr.neighbors(prev)` once per step, and precomputing row offsets (`u * dim`, `v * dim`) in SGD embedding updates eliminates allocation churn and speeds up walk sampling and SGD training.
**Action:** Hoist scratch buffers, inverse bias metrics, and CSR slice references outside inner random walk step loops, and precompute array index offsets in SGD matrix updates.

## 2026-09-22 - O(1) dense vector component ID assignment in graph algorithms
**Learning:** In graph algorithms like WCC, Louvain, and Spanning Forest, mapping component roots or community IDs ($0 \dots N-1$) to sequential 0-based IDs using `HashMap<usize, usize>` introduces $O(N)$ hash computation and dynamic table allocation overhead. Using a dense mapping vector `vec![usize::MAX; n]` replaces hash table lookups with $O(1)$ indexed array lookups, yielding ~3x speedup on component mapping in large graphs (e.g. 1M nodes). Direct slice access over CSR adjacency (`&csr.adjacency[start..end]`) further eliminates iterator call overhead.
**Action:** Use dense vectors (`vec![usize::MAX; n]`) for ID re-mapping in graph algorithms where source IDs are within $[0, N)$, and iterate directly over CSR offsets and adjacency slices instead of constructing iterator wrappers.

## 2026-09-23 - Zero-allocation Brandes' betweenness centrality via visited-state cleanup
**Learning:** In Brandes' algorithm for betweenness centrality (`compute_betweenness_centrality`), re-instantiating `stack`, `sigma`, `dist`, `delta`, `q`, and `pred: Vec<Vec<usize>>` on every source node $s \in 0 \dots N$ triggers $O(N^2)$ dynamic vector allocations. Hoisting scratch vectors outside the source iteration loop and cleaning state (`pred[w].clear()`, `sigma[w] = 0.0`, `dist[w] = -1`, `delta[w] = 0.0`) on visited nodes during reverse BFS stack popping eliminates all $O(N^2)$ heap allocations while avoiding full $O(N)$ buffer resets between iterations.
**Action:** Hoist state buffers outside per-source iteration loops in all-pairs path algorithms, and reset state lazily during stack/frontier unwinding to achieve zero-allocation inner traversals.

## 2026-09-24 - Flat CSR neighbor consolidation and canonical forward triangle counting
**Learning:** In triangle counting (`compute_triangle_count`), allocating per-node neighbor vectors (`Vec<Vec<usize>>`) triggers $N+1$ dynamic heap allocations. Furthermore, unconstrained two-pointer list intersections visit each triangle 3 times and compare elements $\le u$. Consolidating deduplicated sorted adjacency into a single flat array (`flat_adj` and `offsets`) reduces allocations from $N+1$ to 2. Restricting intersection scans to canonical $w > u > v$ ordering discovers each triangle exactly once, shortens sub-slice intersection scans by $>50\%$, and eliminates trailing division operations, yielding >2x speedup (~51% runtime reduction).
**Action:** Consolidate graph neighbor lists into a single contiguous flat slice with offset arrays to eliminate per-node allocations, and restrict intersection traversals to canonical $w > u > v$ orderings in subgraph listing algorithms.

## 2026-10-03 - HashSet-based vector version visibility check optimization
**Learning:** In MVCC node-group tracking (`VectorVersionInfo`), storing row indices in `Vec<u32>` required $O(N)$ linear scans (`contains`) on visibility checks (`is_visible`). Replacing `Vec<u32>` with `HashSet<u32>` in transaction insert and delete maps turns row membership lookups into $O(1)$ operations, achieving up to 49% improvement in row visibility evaluation for larger vector tracking sets without changing visibility lookup semantics.
**Action:** Use `HashSet<u32>` for transaction row visibility maps in vector-level MVCC tracking to guarantee $O(1)$ row presence lookups.

## 2026-10-04 - Single-pass degree precomputation and buffer hoisting in Louvain
**Learning:** In Louvain community detection (`compute_louvain_weighted`), degree vector precomputation and graph weight summation ($m$) originally traversed CSR neighbors twice in separate loops. Fusing degree precomputation and $m$ summation into a single pass over direct CSR slices eliminates an entire $O(|V| + |E|)$ graph sweep. Furthermore, hoisting iteration scratch buffers (`order`, `moves`, `claimed_stamp`) outside the pass loop and consolidating dual neighbor aggregation passes into a single linear CSR slice iteration reduces Louvain execution time by ~20%.
**Action:** Compute node degrees and total graph weight $m$ in a single pass over direct CSR slices, hoist pass scratch buffers outside iterative community detection loops, and consolidate multi-pass neighbor aggregations into single linear CSR slice sweeps.

## FreeSpaceManager `total_free_pages` Allocation Avoidance

- **Problem:** `FreeSpaceManager::total_free_pages()` cloned each `BTreeSet` free list in order to iterate over items via `.clone().into_iter()`, causing excessive heap allocation and overhead during free space calculation.
- **Solution:** Direct iteration over references (`.iter()`) under the read lock guard avoids all cloning and heap allocations.
- **Impact:** Reduced execution time of `total_free_pages()` from ~12.76 µs to ~4.26 µs (~66.5% speedup / 3x performance boost).

## 2026-10-05 - Disjoint slice iteration and initial-step short-circuiting in Node2Vec
**Learning:** In Node2Vec random walks and SGD embedding updates, checking `prev_neighbors` on step 1 performed redundant $O(d_{\text{start}}^2)$ linear scans because `prev_neighbors == neighbors` on step 1 (`prev == current`). Short-circuiting with `prev == current` eliminates this search. Additionally, in SGD embedding updates, indexed array iteration (`embeddings[u_off + i]`) causes repeated bounds checking and multiplication inside hot loops. Splitting `embeddings` into disjoint mutable slices via `split_at_mut` enables direct slice iteration (`.zip()`) with zero bounds checks, allowing LLVM SIMD auto-vectorization across embedding dimensions.
**Action:** Short-circuit step-1 neighbor lookups in random walk algorithms and use `split_at_mut` for disjoint vector slice operations in SGD embedding update loops.

## 2026-10-06 - Flat primitive vectors and pre-allocated heaps in Dijkstra traversal
**Learning:** Using `Vec<Option<f64>>` and `Vec<Option<usize>>` in Dijkstra shortest path algorithms (`weighted_shortest_path`) doubles the memory layout footprint (16 bytes vs 8 bytes) and incurs `Option` discriminant matching overhead on every edge relaxation. Replacing `Option` vectors with flat primitive slices (`Vec<f64>` initialized with `f64::INFINITY` and `Vec<usize>` initialized with `usize::MAX`) combined with `BinaryHeap` capacity pre-allocation (`with_capacity(n.min(1024))`) and direct CSR slice access eliminates heap re-allocation churn and halves memory footprint during Dijkstra graph traversal.
**Action:** Use flat primitive vectors with sentinel values (`f64::INFINITY`, `usize::MAX`) and pre-allocated binary heaps for graph search algorithms, converting to `Option` wrappers only at public API boundaries if necessary.

## 2026-10-07 - Fast-path unbiased random walk sampling in Node2Vec
**Learning:** In Node2Vec random walk generation (`generate_walks`), default unbiased parameters ($p=1.0, q=1.0$) caused $O(\text{degree})$ weight calculations, buffer allocation, and predecessor neighbor scans per walk step. Fast-pathing unbiased walks with direct $O(1)$ uniform neighbor index selection while preserving RNG draw counts eliminates all transition weight computations and vector operations in the default mode without breaking determinism or state sequences.
**Action:** Detect uniform weight conditions ($p=1.0, q=1.0$) in graph random walk algorithms to fast-path direct $O(1)$ index selection and bypass neighbor transition scans.
