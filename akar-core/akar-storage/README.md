# Akar Storage

Page-based columnar storage engine with buffer management, WAL, compression, indexing,
and data readers. Uses **Apache Arrow/Parquet 60.0.0**.

## Buffer Manager

Clock eviction policy, composite-key page mapping `(file_name, page_num)`, configurable pool size with dirty-page flushing.

## Column & ColumnChunk

Page-aligned column with append/scan/get_value via BufferManager I/O, tag-aware compression integration.

**ColumnChunk** — In-memory buffer for `NODE_GROUP_SIZE` (4096) values, with append/flush_to_column/scan operations.

## NodeGroup / NodeTable / RelTable

**NodeGroup** — Collection of ColumnChunks with append_row, flush, scan across groups with auto-creation on overflow.

**NodeTable/RelTable** — NodeGroup-based storage with insert_row, delete_row, update_cell, scan_column. RelTable uses CSR adjacency lists (label-keyed) + per-column property storage.

## WAL (Write-Ahead Log)

Insert/Delete/Update/Commit/Rollback/ColumnWrite records. **Row data is now carried in WAL records** (P60.2) for recovery fidelity — committed SQL rows are recovered from column mirrors (their WAL records carry row data); the per-commit persist is skipped when a checkpoint is imminent (P60.1).

## Checkpoint

Durable column-mirror persist → WAL flush → dirty page flush → truncate WAL cycle.

## Compression

Constant, Boolean, IntegerBitpacking (int8–int64), and Float compression. Tag-aware (compresses payload, never the discriminant byte).

## CSV Reader

Full CSV parser with header detection, custom delimiter/quote/escape, type coercion for all 28+ Value variants, error reporting with line numbers.

## Parquet Reader

Parquet file reader using Apache Arrow/Parquet crates (v60.0.0). Row group reading, Arrow->Akar type mapping (Int64->INT64, Utf8->STRING, etc.).

## Indexing

- **ART (Adaptive Radix Tree)** — Primary key index for node tables
- **SecondaryIndex** — Hash-based secondary indexes on non-unique columns (FR-002). State machine: `Building` → `Available` → `Dropped`. Backed by `OnDiskHashIndex<K>` with L1 cache. Lookups support single-key (`lookup`) and multi-key (`lookup_all`) for non-unique columns.
- **VectorIndexTable** — HNSW index persistence via BufferManager (see `akar-vector`)

## dirty_columns

`TableCatalog::dirty_columns` tracks which columns have been modified since last checkpoint, enabling selective column-mirror persist during checkpoint (only dirty columns are rewritten).

## Tests

349 tests covering: buffer manager, column/chunk operations, NodeGroup/Table operations, WAL record round-trips, checkpoint cycles, compression, CSV/Parquet readers, ART index, SecondaryIndex state machine, vector index persistence, and dirty_columns tracking.