# ADR 004: Storage Engine: Column-Major + Buffer Manager

> **Status:** Accepted | **Date:** 2026-07-07

## Context

Graph database storage perlu mendukung:
1. Scan properties per kolom (columnar access pattern)
2. Insert/update/delete baris
3. MVCC multi-version concurrency
4. Crash recovery via WAL

## Decision

**Column-major storage** dengan **Buffer Manager** (clock eviction) dan **WAL** (write-ahead logging).

### Komponen Utama

| Komponen | File | Fungsi |
|----------|------|--------|
| `BufferManager` | `kuzu-storage/src/buffer_manager.rs` | Page cache dengan clock eviction |
| `PageManager` | `kuzu-storage/src/page_manager.rs` | Alokasi/dealokasi page via FSM |
| `UndoBuffer` | `kuzu-storage/src/undo_buffer.rs` | Rollback safety |
| `WALReplayer` | `kuzu-storage/src/wal_replayer.rs` | Crash recovery, 6 DDL variants |
| `FileHandle` | `kuzu-storage/src/page.rs` | I/O page + FSM integration |
| `NodeTable` | `kuzu-storage/src/table.rs` | Tabel node: column chunks + node groups |
| `RelTable` | `kuzu-storage/src/table.rs` | Tabel rel: CSR adjacency |

### Data Layout

```
NodeTable: [ColumnChunk₀] [ColumnChunk₁] ... [ColumnChunkₙ]
              ↓
         [NodeGroup₀] [NodeGroup₁] ... [NodeGroupₙ]
              ↓
         [page₀] [page₁] ... [pageₙ]  ← BufferManager
```

## Rationale

- **Column-major**: Akses per properti lebih efisien (OLAP pattern untuk graph analytics)
- **Buffer Manager**: Clock eviction sederhana dan efektif untuk workload graph traversal
- **FSM (Free Space Manager)**: Buddy-system untuk mengurangi fragmentasi page
- **WAL + Undo**: MVCC isolation + crash recovery
