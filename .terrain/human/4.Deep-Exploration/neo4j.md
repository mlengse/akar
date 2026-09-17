# Deep Exploration — akar-neo4j

`akar-neo4j` provides migration assistance from existing Neo4j graphs: it parses Cypher dump exports (the `cypher-shell` dump format) and imports nodes/relationships into Akar's relational tables. Test coverage includes `test_cypher_import` (11 tests total).

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| Cypher dump parser | Parse `CREATE (n:Label {props})` + `MATCH (a) CREATE (a)-[r:TYPE]->(b)` from dump text | `akar-core/akar-neo4j/src/lib.rs` |
| node/rel importer | Translate parsed nodes/rels into `CREATE NODE TABLE` + inserts | `akar-core/akar-neo4j/src/` |
| `LOAD EXTENSION "akar-neo4j"` | Registration entry | `akar-core/akar-neo4j/src/lib.rs` |

## Design Decisions

- **Format-level compatibility, not driver-level.** Importing the dump format (rather than talking to a live Neo4j instance) removes auth/version coupling and mirrors how migration usually happens (one-shot dump). The alternative — Bolt protocol client — was rejected for scope.

## Why It Matters

Akar markets itself as a drop-in reimplementation (parity with Kuzu, ADR-001). For teams migrating memory stores off Neo4j, `akar-neo4j` is the on-ramp: dump from Neo4j, scan into Akar, and the rest of the toolchain (vector index, FTS, dream consolidation) works on the migrated graph immediately.