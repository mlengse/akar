# Kuzu Common

Core type system and utilities for the Kuzu database engine.

**Types:** `LogicalTypeID` (37 variants), `PhysicalTypeID` (17 variants), `Value`, `InternalID`, date/time types.

**Vectors:** `ValueVector` — typed columnar data buffer with null mask. `DataChunk` — batch of vectors.

**Infrastructure:** Memory manager, task system (rayon thread pool), file system abstraction, serialization.

**Tests:** 25
