# Deep Exploration — akar-transaction

Transaction management gives Akar multi-statement isolation. The design follows a MVCC snapshot model between storage and the transaction layer: each transaction sees a stable snapshot timestamp, and visibility is decided per row via `VersionInfo`/`VersionVector` structures. Writes are optimistic (OCC) — conflicts are detected per node group at commit, and the whole transaction rolls back if any conflict is found. A multiwriter toggle (AtomicBool + Condvar, introduced in P62 for multi-process support) lets the engine switch between single- and multi-writer behavior.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `TransactionState` | commit_ts / snapshot_ts per transaction | `akar-core/akar-transaction/src/lib.rs` |
| `VersionVector` | Per-thread commit timestamps snapshot | `akar-core/akar-transaction/src/` |
| `VersionInfo` | Per-node-group inserters/deleters HashMaps used for visibility + conflict detection | `akar-core/akar-transaction/src/` |
| `VectorVersionInfo` | Per-vector (1024-row) txn_id → row bitmap for insert/delete | `akar-core/akar-storage/src/version_info.rs` |
| multiwriter toggle | AtomicBool + Condvar enabling multiple writers (P62) | `akar-core/akar-transaction/src/lib.rs` |

## Design Decisions

- **MVCC snapshots, not locks (ADR-005).** Readers never block a writer; a reader sees a stable snapshot. The alternative — shared/exclusive locks — was rejected because agent-memory read loads are query-heavy and latency-sensitive.
- **Single-writer per process enforced by ownership, not a mutex.** The process-level guarantee comes from the `akar.lock` file (`LOCK_FILE_NAME` in `akar-core/akar-main/src/database.rs`) and ownership rules, so an in-memory mutex is not the correctness mechanism. The multiwriter toggle is opt-in for environments that want concurrent writers.
- **OCC conflict set is per node group.** `check_node_conflicts` compares the transaction's VersionInfo against committed rows; only conflicting groups roll back, keeping the common no-conflict path cheap.

## Why It Matters

Commit decides what becomes visible and what writes go to the WAL (`StorageManager::commit` appends version vectors + logical operations). The interplay with storage's `VersionInfo` visibility check is what makes `SELECT` after `DELETE` return "slot null" for soft-deleted rows while PK lookups exclude them (`test_delete_and_set`). This crate is the reason multi-statement workflows — like an agent reading, computing, then writing — behave atomically without heavyweight locking.