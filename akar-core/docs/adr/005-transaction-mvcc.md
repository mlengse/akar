# ADR 005: Transaction: MVCC + Multiwriter

> **Status:** Accepted | **Date:** 2026-07-07

## Context

Kùzu perlu mendukung concurrent read/write transactions dengan isolasi snapshot.

## Decision

**MVCC (Multi-Version Concurrency Control)** dengan **OCC (Optimistic Concurrency Control)** untuk conflict detection, plus **Multiwriter** support via `AtomicBool` + `Condvar`.

### Komponen

| Komponen | File | Fungsi |
|----------|------|--------|
| `Transaction` | `akar-transaction/src/lib.rs` | Txn state: ID, type (READ_ONLY/WRITE), timestamps |
| `TransactionManager` | `akar-transaction/src/lib.rs` | Manajer global: begin, commit, rollback, checkpoint |
| `TransactionContext` | `akar-transaction/src/lib.rs` | Per-connection: AUTO/MANUAL mode, drop guard |
| `UndoRecord` | `akar-storage/src/undo_buffer.rs` | Rollback data: before-image per row |

### Isolation Model

- **READ_ONLY**: Snapshot read pada `commit_ts` saat transaksi dimulai
- **WRITE**: Dirty read dalam transaksi sendiri, OCC conflict detection saat commit
- **AUTO mode**: Auto-commit setelah setiap DDL/DML statement
- **MANUAL mode**: Explicit `BEGIN`/`COMMIT`/`ROLLBACK`

### Multiwriter

```rust
// Concurrent write limit via AtomicBool
concurrent_writes: AtomicBool  // true = allow multiple writers
writer_condvar: Condvar         // block if concurrent_writes == false
```

- Saat `concurrent_writes = false`: single-writer mode, blocking
- Saat `concurrent_writes = true`: multiple writers, OCC conflict detection

## Rationale

- **MVCC**: Read tidak memblokir write, write tidak memblokir read
- **OCC**: Tidak perlu lock manager terpisah, deteksi konflik saat commit
- **Multiwriter toggle**: Fleksibel — bisa single-writer untuk workload insert-heavy, multi-writer untuk OLTP
