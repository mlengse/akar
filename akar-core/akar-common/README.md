# Akar Common

Core type system and utilities for the Akar database engine.

**Types:** `LogicalTypeID` (37 variants including Decimal, UUID, struct types),
`PhysicalTypeID` (19 variants), `Value` (28 variants: Null, Bool, Int8–Int64,
UInt8–UInt64, Int128, Float, Double, String, Blob, Date, Timestamp variants, Interval,
InternalID, List, Map, Struct), date/time types.

**Vectors:** `ValueVector` — typed columnar data buffer with null mask and
get_value/set_value for all 28 types. `DataChunk` — batch of vectors for pipeline
execution.

**Infrastructure:** Memory manager (tracking), task system (rayon thread pool), file
system abstraction, binary serialization for Value/LogicalType.

**Tests:** 24
