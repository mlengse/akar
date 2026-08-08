# Akar Storage

Page-based columnar storage engine with buffer management, WAL, compression, indexing, and data readers.

**Buffer Manager:** Clock eviction policy, composite-key page mapping `(file_name, page_num)`, configurable pool size with dirty-page flushing.

**Column:** Page-aligned column with append/scan/get_value via BufferManager I/O, tag-aware compression integration.

**ColumnChunk:** In-memory buffer for `NODE_GROUP_SIZE` (4096) values, with append/flush_to_column/scan operations.

**NodeGroup:** Collection of ColumnChunks — append_row, flush, scan across groups with auto-creation on overflow.

**NodeTable/RelTable:** NodeGroup-based storage with insert_row, delete_row, update_cell, scan_column. RelTable uses CSR adjacency lists (label-keyed) + per-column property storage.

**WAL:** Write-ahead log with Insert/Delete/Update/Commit/Rollback/ColumnWrite records.

**Checkpoint:** WAL flush → dirty page flush → truncate WAL cycle.

**Compression:** Constant, Boolean, IntegerBitpacking (int8–int64), and Float compression. Tag-aware (compresses payload, never the discriminant byte).

**CSV Reader:** Full CSV parser with header detection, custom delimiter/quote/escape, type coercion for all 28+ Value variants, error reporting with line numbers.

**Parquet Reader:** Parquet file reader using Apache Arrow/Parquet crates (v53). Row group reading, Arrow→Akar type mapping (Int64→INT64, Utf8→STRING, etc.).

**Index:** Generic hash index with collision resolution.

**Tests:** 328
