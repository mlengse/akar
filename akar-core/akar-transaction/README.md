# Akar Transaction Manager

MVCC-based serializable ACID transaction manager.

**Features:**
- Timestamp-based MVCC with `begin_read()` / `begin_write()`
- Table-level locking with conflict detection
- OCC row-level conflict detection (`RowConflictTracker`)
- Commit timestamp assignment
- Undo buffer for rollback
- Configurable max concurrent writers

**Tests:** 18
