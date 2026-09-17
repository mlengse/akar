# Deep Exploration — akar-server

`akar-server` is the broker reference transport for the sulur daemon: a TCP server that multiplexes multiple client sessions over a single database, using length-prefixed JSON frames. It runs full `db.log()` + FTS-IVF initialization on connect, handles Heartbeat/RunQuery/Prepare/Substitute/Rollback, and exposes dream endpoints.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `Server::bind(addr, db)` | Create server, bind to address | `akar-core/akar-server/src/lib.rs` |
| `Server::start()` | Accept TCP connections, spawn session handler threads | `akar-core/akar-server/src/lib.rs` |
| `Server::shutdown()` / `local_addr()` | Lifecycle management | `akar-core/akar-server/src/lib.rs` |
| message framing | Length(4 bytes LE) + JSON payload | `akar-core/akar-server/src/lib.rs` |
| handshake | db.log() + FTS-IVF init on Connect | `akar-core/akar-server/src/lib.rs` |
| heartbeat | 200ms keep-alive; Heartbeat/HeartbeatResponse | `akar-core/akar-server/src/lib.rs` |
| commands | Connect / StartSession / RunQuery / Prepare / Substitute / Execute / Rollback / EndSession | `akar-core/akar-server/src/lib.rs` |
| dream endpoints | `/dream/start` (mode=...) + `/dream/stop` | `akar-core/akar-server/src/lib.rs` |
| `RemoteDatabase` | Client-side handle (sends commands, receives Arrow records) | `akar-core/akar-server/src/remote.rs:449` |
| tests | 12 tests (handshake, command routing, timeout, shutdown) | `akar-core/akar-server/src/lib.rs` |

## Design Decisions

- **Single database, multiple sessions.** All sessions share one `Database` instance (internal MVCC handles isolation). The alternative — one database per connection — was rejected because memory graphs must be visible across sessions for multi-agent coordination.
- **Length-prefixed JSON (not gRPC).** A simple length-payload framing is easier to test, debug, and embed than gRPC for a single-process broker; the trade-off (no streaming, message-by-message) fits Cypher's request-response semantics.
- **Dream control endpoints in the broker.** Sulur can schedule sleep cycles via HTTP-like `/dream/start` and `/dream/stop`, decoupling the daemon's orchestration schedule from the Python harness.

## Why It Matters

`akar-server` is the bridge between the in-process embedded engine and sulur's multi-process operation. The ADR-02 §13 boundary identifies it as the "broker reference transport for the sulur daemon" — its correctness determines whether sulur's heartbeat-based scheduler stays alive and whether multi-process sessions don't conflict on the shared database lock.