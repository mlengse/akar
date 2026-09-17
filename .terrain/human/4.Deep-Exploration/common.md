# Deep Exploration — akar-common

Akar's shared foundation lives in `akar-core/akar-common`. It defines the Value type every column, expression, and result cell is built from, the DataChunk columnar array used throughout execution, memory management primitives that bound the BufferManager, a task pool for parallel operators, and a VFS abstraction that lets storage and extensions read from local disk, HTTP, or Lakehouse URIs through one interface. Because every other crate depends on it, its layout decisions shape the whole engine.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `Value` | Schemaless typed value (null, bool, number, string, list, map, vector, datetime) | `akar-core/akar-common/src/lib.rs` |
| `DataChunk` | Arrow-like columnar batch used between operators and for copy/ingest | `akar-core/akar-common/src/lib.rs` |
| `MemoryManager` | Byte budget per database; BufferManager + table caches report usage into it | `akar-core/akar-common/src/memory.rs` |
| `TaskManager` | Thread pool for parallel sort / aggregate / join kernels | `akar-core/akar-common/src/task.rs` |
| `VFS` | Virtual filesystem abstraction (local read/write; extension URIs) | `akar-core/akar-common/src/` |
| `SelectionVector` | Row-id remapping helpers used by filter projections | `akar-core/akar-common/src/` |

## Design Decisions

- **Single shared Value type across the engine.** Chosen over per-layer value models. The alternative — separate frontend and storage value representations — would force conversions at every layer boundary. One type keeps the whole pipeline (parser → binder → planner → processor → storage) on a common wire format.
- **DataChunk as the chunked-copy primitive.** Physical operators stream `DataChunk`s sized by `vector_size` (default 1024 rows). This mirrors Kuzu's `vector_size` design, giving a bounded memory envelope for huge COPY operations (`SpilledTupleDataChunkState` spills to disk when memory pressure hits).

The crate is a leaf (no dependencies on other Akar crates), which makes it the natural release root and the least volatile in dependency order (ADR-002 bottom-up publish order).

## Why It Matters

Effectively every subsystem is measured in DataChunks and typed by Value. The memory manager created here is what bounds the BufferManager's page cache and each table's index cache (`MemoryManager` in `akar-core/akar-storage/src/lib.rs` fills under it). If a memory footprint regression appears, the chain of blame runs back to the budget carved here.