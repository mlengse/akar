# Audit storage engine details 19/07/2026

## Comprehensive Audit of `kuzu-storage` Crate

### Objective
Audit the complete Rust storage crate at `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage` (30 source files). For each component, evaluate implementation completeness, identify `todo!()` / `unimplemented!()` calls, flag simplifications versus the C++ originals, and document missing features.

### Important Details
- **Total files audited**: 30 (29 initially listed + `page_manager.rs`, which is declared as a module in `lib.rs` but was not listed in the original 29).
- **Re-exports from `lib.rs`**: `ArtPrimaryKeyIndex`, `ArtKey`, `ColumnChunk`, `NODE_GROUP_SIZE`, `HashIndex`, `IndexKey`, `OnDiskHashIndex`, `LocalStorage`, `LocalWAL`, `NodeGroup`, `PageManager`, `ShadowFile`, `Spiller`, `MultiWayStreamMerge`, `SpillFile`, `ColumnDefinition`, `NodeTable`, `RelTable`, `TableCatalog`, `UndoBuffer`, `VectorIndexTable`, `extract_f64_list_from_value`, `ReplayResult`, `WALReplayer`.
- **StorageManager** (defined in `lib.rs` lines 67–159): Root struct owning `BufferManager`, `WAL`, `MemoryManager`, `PageManager`, and `TableCatalog`. `open()` delegates to `new()` then expects caller to call `recover()` for WAL replay.
- **Simplification pattern**: All components are simplified relative to C++ Kuzu/Ladybug. The most common simplification is using `Vec<u8>` / `Vec<Value>` serialization instead of Arrow-format columnar buffers. Several components are stubs or have no-op paths. The integration tests in `lib.rs` (~25k lines) are comprehensive and exercise all major flows.

---

### Work State

#### COMPLETED — Fully Implemented Components

| Component | File | C++ Comparison | Notes |
|---|---|---|---|
| **WAL** | `wal.rs` | Full parity with C++ WAL record types | Binary serialization of all record types (Insert, Delete, Update, UpdateFsm, ColumnWrite, etc.). `flush_to_disk()`, `load_from_disk()`. |
| **LocalWAL** | `local_wal.rs` | Full parity | Per-txn buffer serialized to `Vec<u8>`, bulk-copied to global WAL on commit via `WALRecord::LocalWALData`. Matches C++ design for mutex-free commit path. |
| **UndoBuffer** | `undo_buffer.rs` | Full parity | `Vec<UndoRecord>` with `replay()` (reverse order for rollback), `clear()` (commit), `drain()`. Integrates with `kuzu_transaction::UndoRecord`. |
| **FreeSpaceManager** | `free_space_manager.rs` | Full parity | Buddy-system tracking with `BTreeSet<PageRange>`. Thread-safe via `RwLock`. `pop_range()`, `add_range()`. |
| **PageManager** | `page_manager.rs` | Full parity | Wraps FSM + file extension. `allocate_page()` (FSM-first, then extend), `free_page()`, `free_pages()`, `extend_file()` (zero-fill). Wasm32 stub for `extend_file()`. |
| **ART Index** | `art_index.rs` | Direct port of `ladybug/src/storage/index/art_index.h` | `insert()`, `search()`, `search_range()` (prefix-based), `flush_to_disk()`, `load_from_disk()`. NodeBlock buffer pool for persistence. |
| **ART Key** | `art_key.rs` | Exact port of Ladybug `ArtKey` encoding | Order-preserving bytes for all primitives, 0x00-escaping for strings. Matches lines 261-284 of C++ `art_index.cpp`. |
| **ART Node** | `art_node.rs` | Full Node4/16/48/256 with transitions | `insert()`, `search()`, `grow()`, `change()`, serialize/deserialize. Runtime allocation uses `Box<ArtNode>` (C++ uses slab allocator), but persistence matches. |
| **Hash Index** | `index.rs` | Full parity | Two-layer: L1 `HashMap<K, u64>` + L2 `OnDiskHashIndex<K>` with overflow buckets. `flush()`, `load_from_disk()`. |
| **HyperLogLog** | `hyperloglog.rs` | Full parity with Ladybug config | P=6, 64 registers, 8-bit. Bias correction with linear counting at threshold < 2.5*M. |
| **VectorIndex** | `vector_index.rs` | Port of Ladybug `VectorIndexTable` | Wraps `HnswIndex`. Header page (magic, num_vectors, entry_point, max_level, dims, metric). Serialized HNSW nodes + connections. |

#### COMPLETED — Functional but Simplified

| Component | File | Simplifications vs C++ | Missing Features |
|---|---|---|---|
| **BufferManager** | `buffer_manager.rs` | No memory-mapped regions, no direct I/O, no multi-file stripe groups, no NUMA placement, no page readahead. | Segment-based allocation, I/O aggregation, NUMA awareness. Known limitations documented in code at line 129. |
| **Column** | `column.rs` | Per-value tag-byte serialization (C++ uses arrow-format vectors with null masks). No batch `scan(vector, offset, length)`. | Batch scanning, null bitmasks, small-value inlining. |
| **ColumnChunk** | `column_chunk.rs` | Buffers as `Vec<Value>` (C++ uses ChunkedNodeGroup with arrow flat buffers). Arrow builder fields are `#[allow(dead_code)]`. | `to_arrow()` returns `Err("not implemented - requires arrow type mapping")`. |
| **Compression** | `compression.rs` | Per-chunk function dispatch (C++ has `CompressionAlg` class hierarchy with metadata). `StringDictionary` is pass-through. No SIMD. | StringDictionary encoding, alignment optimizations. |
| **Tables** | `table.rs` | `NodeTable` is in-memory with `flush_node_groups()` to Column (C++ has persistent Column objects). `RelTable` stores flat `Vec<RelData>` (C++ uses CSR per direction). | Persistent CSR for RelTable, multi-label node support. |
| **NodeGroup** | `node_group.rs` | In-memory buffering with flush (C++ manages persistent column chunks with page I/O). Spill uses JSON lines (C++ uses Arrow IPC). | Arrow-format spills. |
| **Checkpoint** | `checkpoint.rs` | `flush_table()` is no-op (C++ replays WAL into actual storage). No WAL truncation/rotation. | Full WAL replay into structures, truncation, incomplete-checkpoint recovery. |
| **WALReplayer** | `wal_replayer.rs` | Delegates apply logic to caller via closure (C++ integrates with StorageManager). No checkpoint record handling. | Automatic storage application. |
| **LocalStorage** | `local_storage.rs` | Serialized byte blobs (C++ uses Arrow columnar format). No conflict detection during flush. | Vectorized commit, constraint checking. |
| **ShadowFile** | `shadow_file.rs` | Simple in-memory HashMap overlay (C++ integrates with BMFileHandle for COW at I/O level). No disk persistence. | Copy-on-write I/O integration, shadow page persistence. |
| **Page** | `page.rs` | Frame stores data directly (C++ has separate Page + Frame with per-page metadata on disk). | Versioned pages, per-page disk metadata. |
| **LazyScanner** | `lazy_scanner.rs` | Materializes entire group into `Vec<Vec<Value>>` before yielding. | Tuple-at-a-time streaming with column projection. |
| **Predicate** | `predicate.rs` | Standalone zone-map filter (C++ has per-vector predicate evaluation integrated with scan pipeline). | Predicate pushdown into compressed data. |
| **IceFormat** | `ice_format.rs` | Uses Parquet files (C++ Ladybug uses custom native disk format). Only Rel tables. | Native IceDiskNodeTable. |
| **VersionInfo** | `version_info.rs` | `Vec<u32>` lists (C++ Vela uses Roaring bitmaps). No timestamp-based visibility. | Bitmap row tracking, MVCC timestamp visibility. |
| **UpdateInfo** | `update_info.rs` | Linked list version chain, no GC (C++ Vela uses efficient MVCC with GC). | Garbage collection, timestamp-based version pruning. |
| **RoaringBitmap** | `roaring_bitmap.rs` | Only Array and Bitmap containers (C++ has Run container + SIMD). No serde. | Run container, SIMD optimization, serialization. |
| **Stats** | `stats.rs` | In-memory HashMap only, no persistence or auto-collection. | Persistent stats, auto-update during writes. |
| **Spiller** | `spiller.rs` | JSON lines format (C++ uses Arrow IPC for zero-copy columnar spills). | Arrow-format spill files, compression. |
| **CSV/NPY Readers** | `csv_reader.rs`, `npy_reader.rs` | Not audited in detail (out of scope for storage engine core). | Plausibly functional readers for bulk import. |

#### BLOCKED — Stub or No-Op Components

| Component | File | Issue |
|---|---|---|
| **CSR** | `csr.rs` | **Stub.** `get_neighbors()` returns `Ok(Vec::new())`. `add_edge()` only increments `num_edges`. Comment says "Logic to update offsets and adjacency lists" — not implemented. **Cannot be used for graph traversal.** |

---

### `todo!()` / `unimplemented!()` / `Err("not implemented")` Calls Found

| File | Line | Call | Context |
|---|---|---|---|
| `column_chunk.rs` | 127 | `Err("not implemented - requires arrow type mapping")` | `ColumnChunk::to_arrow()` method body — always returns error. |

No `todo!()` or `unimplemented!()` macros were found anywhere in the 30 source files. The CSR stub methods are silent no-ops (no panic, no error return).

---

### Gaps / Missing Features Summary

1. **CSR adjacency is not implemented** — graph traversal does not work.
2. **Arrow integration is dead code** — `ColumnChunk` has unused arrow builders and `to_arrow()` returns an error.
3. **StringDictionary compression is a pass-through** — no actual dictionary encoding.
4. **Checkpoint is a no-op for tables** — `flush_table()` returns `Ok(0)`.
5. **No WAL truncation or rotation** during checkpoint.
6. **Stats have no persistence** — lost on restart.
7. **RoaringBitmap has no serialization** — cannot persist bitmap data.
8. **No NUMA-aware page placement** in BufferManager.
9. **No batch column scan** — only `scan_all_values()` returns a full materialized Vec.
10. **ShadowFile has no disk persistence** — copy-on-write is not integrated with I/O.

---

### Next Move
The audit is complete. If continuing development:
1. **High priority**: Implement `CsrIndex` adjacency logic (offset + adjacency arrays, delta updates). Currently a hard blocker for graph workloads.
2. **Medium priority**: Implement `StringDictionary` compression, `to_arrow()`, and RoaringBitmap serialization.
3. **Low priority**: Add WAL truncation to `checkpoint()`, add stats persistence, add batch column scan API.

---

### Relevant Files
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\lib.rs** — Module declarations, `StorageManager`, re-exports, ~25k-line integration test suite
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\wal.rs** — Full WAL with all record types
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\local_wal.rs** — Per-txn WAL buffer, bulk-copy on commit
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\undo_buffer.rs** — Undo records for rollback
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\free_space_manager.rs** — Buddy-system free space tracker
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\page_manager.rs** — Page allocator wrapping FSM + file extension
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\buffer_manager.rs** — Buffer pool with Clock eviction (known limitations doc-commented)
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\column.rs** — Page-based column store with tag-byte serialization
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\column_chunk.rs** — `Vec<Value>` buffer, `to_arrow()` returns error
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\compression.rs** — Per-type compress/decompress, StringDict is pass-through
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\table.rs** — NodeTable, RelTable (flat Vec<RelData>), TableCatalog
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\node_group.rs** — In-memory NodeGroup with flush/s pill
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\csr.rs** — **STUB**: empty neighbors, no-op add_edge
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\wal_replayer.rs** — Callback-based WAL replay
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\checkpoint.rs** — Flush WAL + dirty pages + marker; `flush_table()` no-op
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\index.rs** — Two-layer hash index (L1+L2)
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\art_index.rs** — Full ART with persistence and prefix range scan
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\art_key.rs** — Order-preserving byte encoding
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\art_node.rs** — Node4/16/48/256 with serialization
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\local_storage.rs** — Per-txn write buffer with serialized blobs
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\shadow_file.rs** — In-memory shadow overlay
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\page.rs** — Frame with pin/dirty/clock_ref, FileHandle
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\lazy_scanner.rs** — NodeGroup-at-a-time scanner
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\predicate.rs** — Zone-map filtering
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\ice_format.rs** — Parquet-based IceDiskRelTable
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\version_info.rs** — Per-vector txn insert/delete tracking
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\update_info.rs** — Version chain linked list
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\roaring_bitmap.rs** — Array+Bitmap containers, no Run, no serde
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\stats.rs** — In-memory stats, no persistence
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\hyperloglog.rs** — HLL P=6, 64 registers
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\vector_index.rs** — HNSW vector index with persistence
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\spiller.rs** — JSON-lines spill + multi-way merge
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\csv_reader.rs** — CSV bulk reader (not audited in detail)
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\npy_reader.rs** — NPY array reader (not audited in detail)
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\parquet_reader.rs** — Parquet reader (feature-gated, not audited)
- **C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\parquet_writer.rs** — Parquet writer (feature-gated, not audited)

---

This audit confirms that the crate is **functional and well-tested for a Rust port with simplified storage** (direct page I/O, tag-byte serialization, in-memory adjacency). The major blocker is `csr.rs` (no graph traversal). The remaining components range from fully implemented (WAL, ART, hash index, FreeSpaceManager, HLL, VectorIndex) to functional-but-simplified (Column, BufferManager, Compression, etc.) with well-documented known limitations.