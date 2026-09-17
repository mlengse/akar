# Deep Exploration — akar-postgres

`akar-postgres` exposes a PostgreSQL database to Akar queries through the `sql_query` table function, bridging Akar's synchronous execution to Postgres's async client behind a tokio runtime.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `sql_query` | Table function: run SQL against Postgres, stream rows as DataChunks | `akar-core/akar-postgres/src/lib.rs` |
| tokio bridge | `block_on` between processor thread and async Postgres driver | `akar-core/akar-postgres/src/` |

## Design Decisions

- **Async-over-sync bridge.** The processor is synchronous (`block_on`). Wrapping the async driver in a single-threaded runtime is the pragmatic choice — the alternative (making the whole processor async) would ripple through every operator.

## Why It Matters

Akar is often embedded in systems that keep operational data in Postgres. `sql_query` lets Cypher expressions reference live Postgres tables without duplicating data — useful for memory agents that must join their graph memory with application records.