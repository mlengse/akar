# ADR 004: Storage Engine: Column-Major + Buffer Manager

> **Status:** Accepted | **Date:** 2026-07-07 | **Last Updated:** 2026-07-19

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
| `BufferManager` | `akar-storage/src/buffer_manager.rs` | Page cache dengan clock eviction |
| `PageManager` | `akar-storage/src/page_manager.rs` | Alokasi/dealokasi page via FSM |
| `UndoBuffer` | `akar-storage/src/undo_buffer.rs` | Rollback safety |
| `WALReplayer` | `akar-storage/src/wal_replayer.rs` | Crash recovery, 6 DDL variants |
| `FileHandle` | `akar-storage/src/page.rs` | I/O page + FSM integration |
| `NodeTable` | `akar-storage/src/table.rs` | Tabel node: column chunks + node groups |
| `RelTable` | `akar-storage/src/table.rs` | Tabel rel: flat Vec<RelData> (CSR **stub** — `get_neighbors()` return empty) |

### Data Layout

```
NodeTable: [ColumnChunk₀] [ColumnChunk₁] ... [ColumnChunkₙ]
              ↓
         [NodeGroup₀] [NodeGroup₁] ... [NodeGroupₙ]
              ↓
         [page₀] [page₁] ... [pageₙ]  ← BufferManager
```

### Catatan: Status Implementasi

Beberapa komponen masih berupa stub/simplifikasi (per 2026-07-19):
- **CSR adjacency** (`akar-storage/src/csr.rs`) — `get_neighbors()` return `Ok(vec![])`, belum ada offset/adjacency arrays. RelTable menyimpan `Vec<RelData>` flat sebagai fallback.
- **Checkpoint** (`akar-storage/src/checkpoint.rs`) — `flush_table()` adalah no-op, tidak benar-benar mem-persist data ke disk.
- **BufferManager** — tidak ada memory-mapped regions, NUMA placement, atau page readahead.
- **StringDictionary compression** — pass-through (tidak ada encoding aktual).

## Rationale

- **Column-major**: Akses per properti lebih efisien (OLAP pattern untuk graph analytics)
- **Buffer Manager**: Clock eviction sederhana dan efektif untuk workload graph traversal
- **FSM (Free Space Manager)**: Buddy-system untuk mengurangi fragmentasi page
- **WAL + Undo**: MVCC isolation + crash recovery
