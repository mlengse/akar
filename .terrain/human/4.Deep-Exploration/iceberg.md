# Deep Exploration — akar-iceberg

Apache Iceberg tables become queryable via `iceberg_scan` / `iceberg_metadata` / `iceberg_snapshots` / `iceberg_merge Snapshots` — exposing snapshots and manifest lists to Cypher through a native metadata.json reader and mini Avro decoder (P57.2).

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `iceberg_scan` | Table function: scan an Iceberg table | `akar-core/akar-iceberg/src/lib.rs` |
| `iceberg_metadata` | Read metadata.json / manifest lists | `akar-core/akar-iceberg/src/` |
| `iceberg_snapshots` | Read snapshot log for incremental reads | `akar-core/akar-iceberg/src/` |
| `iceberg_merge Snapshots` | Merge snapshot files | `akar-core/akar-iceberg/src/` |
| mini Avro decoder | Minimal Avro block-level reader for manifest lists | `akar-core/akar-iceberg/src/` |

## Design Decisions

- **Native metadata reader over catalog API.** The alternative — calling the Iceberg REST catalog — was rejected to avoid a runtime dependency on a catalog server; native reads work offline from local filesystems.
- **`fast_read_path` mode.** For quick exploration, `iceberg_metadata` reads only `metadata.json` without resolving manifests, useful when just schema/properties are needed (schema fetch is ~4x faster).

## Why It Matters

Iceberg is emerging as the universal Lakehouse format. `iceberg_scan` enables Cypher queries over Iceberg snapshots, making existing data lake history visible to agents that analyze usage patterns or trace data lineage in-memory.