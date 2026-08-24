# Akar Common

Core type system and utilities for the Akar database engine.

**Types:** `LogicalTypeID` (37 variants including Decimal, UUID, struct types),
`PhysicalTypeID` (19 variants), `Value` (30 variants: Null, Bool, Int8–Int64,
UInt8–UInt64, Int128/UInt128, Float, Double, String, Blob, Date, Timestamp variants, Interval,
InternalID, Json, Union, List, Map, Struct), date/time types.

**Vectors:** `ValueVector` — typed columnar data buffer with null mask and
get_value/set_value for all 30 types. `DataChunk` — batch of vectors for pipeline
execution.

**Infrastructure:** Memory manager (tracking), task system (rayon thread pool), file
system abstraction, binary serialization for Value/LogicalType.

**Tests:** 36
