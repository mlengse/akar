# Deep Exploration — akar-processor

The processor is Akar's execution engine: ~50 physical operator structs that turn a physical plan into Arrow-typed result chunks. It handles scan/join/aggregate/sort (parallelized), DML (insert/delete/update), DDL sync (FTS index creation and row catch-up), function/standalone-call dispatch, and the physical vector-similarity scan. It is the largest single behavioral surface after storage.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| Physical plan execution | Dispatch physical operators, stream DataChunks | `akar-core/akar-processor/src/plan.rs` |
| `PhysicalCopyFrom` | COPY FROM CSV/Parquet/Lakehouse with spill | `akar-core/akar-processor/src/physical/write_ops/copy.rs` |
| `PhysicalCreateFtsIndex` / FTS sync | FTS DDL + row catch-up after index creation | `akar-core/akar-processor/src/physical/write_ops/ddl_fts.rs`, `fts_sync.rs` |
| `PhysicalFtsScan` | FTS query execution (BM25) | `akar-core/akar-processor/src/` |
| Extend column pruning | Prune `PhysicalExtend` output to referenced columns (identity `id`/`_id` always kept); safety caps `AKAR_MAX_EXTEND_ROWS` (5M) / `AKAR_MAX_CROSS_ROWS` (100k) against OOM (F6) | `akar-core/akar-processor/src/processor/extend_prune.rs`, `physical/write_ops/recursiveextend.rs`, `physical/join_ops.rs` |
| `PhysicalVectorSimilarityScan` | HNSW read from SQL, K-NN retrieval | `akar-core/akar-processor/src/processor/vector_similarity_scan.rs` |
| `PhysicalDelete` / `PhysicalSet` | Soft-delete + column update via `row_id_column_index()` | `akar-core/akar-processor/src/physical/` |
| StandaloneCall | `CALL`/`LOAD EXTENSION` dispatch to extensions | `akar-core/akar-processor/src/processor/standalone_call.rs` |

## Design Decisions

- **Arrow-typed kernels for expression/aggregation/join.** Using Arrow-format chunks as the runtime wire format gives SIMD-friendly kernels and cheap output-to-bindings conversion (`get_as_arrow`, etc.). Alternative: custom row format — rejected for ecosystem interop.
- **Parallel operators inside one query.** Sort/aggregate/hash-join operators spawn per-thread work via `TaskManager`, but each query is single-threaded at the plan level. This is a simpler concurrency model than intra-query DAG parallel scheduling.
- **FTS catch-up is synchronous.** The flagged fix `test_fts_catches_up_rows_after_index` (rows inserted after `CREATE FTS INDEX` are searchable; soft-deleted rows stop matching) is enforced in `fts_sync.rs` — a deliberate synchronous design over a background rebuild.
- **DML uses real row-id resolution.** A prior defect read column 0 as row-id; the fix resolves the PK/`_id` column index properly (`row_id_column_index()`), encoded by `test_delete_and_set`.
- **Extend output is column-pruned for memory safety.** `PhysicalExtend` by default duplicates every input/rel/dest column per produced edge — gigabytes for a full relationship-table scan (F6: 4.3 GB RSS on a 19 MB DB). `collect_extend_prune`/`keep_column` (`processor/extend_prune.rs`) now drop all columns not referenced downstream (identity `{var}.id`/`{var}._id` always retained); the pruning is conservative — any unanalyzable tail operator (joins, unions, flatten, writes, ...) disables it. Where pruning can't apply, `AKAR_MAX_EXTEND_ROWS` (default 5M) and `AKAR_MAX_CROSS_ROWS` (default 100k) safety caps turn what used to be an allocator OOM abort into a clean error.

## Why It Matters

Processor is where design decisions become measurable: a wrong physical plan shows up as a slow query; a DML row-id bug shows up as corrupted deletes. Together with storage it holds the 1,890-test gate green, making it the highest-leverage crate for performance and correctness work.