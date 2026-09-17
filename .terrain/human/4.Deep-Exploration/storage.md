# Deep Exploration — akar-storage

Akar's durability and performance core. Storage owns column-major page files, a buffer manager that caches and evicts pages, a write-ahead log for crash safety, ART and hash indexes for lookups, CSR adjacency lists for graph traversal, compression, and the CSV/Parquet readers that feed COPY. It is by far the largest and most tested crate (349 tests).

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `StorageManager` | Owns BufferManager + WAL + checkpoint; registers column/index files | `akar-core/akar-storage/src/lib.rs` |
| `BufferManager` | Page cache `(file_name, page_num) → Frame`; clock eviction; mmap or read/write syscalls | `akar-core/akar-storage/src/buffer_manager.rs:163` |
| `WAL` | Append-only log of commit vectors + logical operations; `WAL_VERSION = 2` | `akar-core/akar-storage/src/wal.rs:14` |
| `Column` | Column-major page file `col_{table_id}_{col_idx}` | `akar-core/akar-storage/src/column.rs:150` |
| `ArtPrimaryKeyIndex` | Adaptive Radix Tree PK index, magic `0x4152540000000000` | `akar-core/akar-storage/src/art_index.rs:19` |
| `HashIndex` | Fixed-slot hash table index (`.idx` files) | `akar-core/akar-storage/src/index.rs:212` |
| `persistence` | Overflow (.ovf) sidecar for oversized values | `akar-core/akar-storage/src/persistence.rs:35` |
| `checkpoint` | Flush dirty pages → clear log → clear cache → compact | `akar-core/akar-storage/src/checkpoint.rs:25` |
| `GroupCommit` | Batches WAL appends to amortize fsync | `akar-core/akar-storage/src/group_commit.rs` |
| `CSRAdjacency` | Compressed Sparse Row adjacency for rel lookup / graph algorithms | `akar-core/akar-storage/src/csr.rs` |
| `VersionInfo` / `VectorVersionInfo` | Per-node-group insert/delete visibility for MVCC | `akar-core/akar-storage/src/version_info.rs` |

## Design Decisions

- **Column-major + fixed 4KB pages (ADR-004).** Chosen for analytic scans and cache efficiency. Alternative row-store rejected: graph analytics read whole columns, and property fan-out across node types is better served columnar.
- **BufferManager lives above raw files.** All reads/writes go through `pin`/`unpin` frames; eviction uses a Clock hand. Read-ahead (default on, 8 pages ahead) and mmap vs syscall mode are configurable (`BufferManagerConfig`).
- **WAL + checkpoint (not write-in-place).** Committing is a cheap WAL append; a full buffer flush only happens when WAL size crosses `checkpoint_threshold`. This gives durability with write amplification kept low. `compact()` further compacts WAL version vectors.
- **Overflow sidecars for big values.** Values that don't fit `max_inline_bytes` go to `col_{tid}.ovf`, keeping page footprint fixed (`persistence.rs:55`).
- **Index persistence is page-based.** ART and hash index nodes serialize into the same page machinery, so a single `flush_table` handles schema data + indexes consistently. ART magic header guards against opening a wrong-format file.

## Why It Matters

Every physical operator that touches data hits this crate. The processor's `PhysicalCopyFrom` streams into `SpilledTupleDataChunkState`; reads pin column pages via sequence scans; PK lookups probe the ART tree; graph algorithms consume `CSRAdjacency`. A storage bug shows up as a wrong query result, a crash on checkpoint, or an SSD write-amplification regression — which is why the crate carries more tests than any other.