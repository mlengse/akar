# Kuzu Storage

Disk-based columnar storage engine with buffer management, WAL, compression, and indexing.

**Buffer Manager:** Clock eviction policy, page pinning, configurable pool size.

**WAL:** Write-ahead log with Insert/Delete/Update/Commit/Rollback records.

**Compression:** Constant and boolean bitpacking compression.

**Tables:** Node and relationship table abstractions.

**Index:** Generic hash index.

**Tests:** 15
