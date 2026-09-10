# Akar Server

> **Boundary note (ADR-02, §13):** Akar is an **embedded library** — it ships
> no server product. This crate is the *broker reference* transport (TCP
> framing, auth, idle shutdown) consumed by the **sulur** daemon lifecycle.
> Spawn/adopt, sidecar discovery, health, and all lifecycle tooling
> (`tools/sulur_daemon_ctl.py`, `tools/sulur_doctor.py`) are permanently
> sulur-side. Do not add orchestration, policy, or memory semantics here.

Embedded TCP server mode — multi-process access to a single Akar database (P47).

One process owns the `Database` (and its exclusive file lock) while N client
processes connect over TCP to query the same database.

**Features:**
- `Server::bind(addr, db)` + `Server::start()` — non-blocking accept loop on a background thread
- Length-prefixed JSON framing with `MAX_FRAME_SIZE` guard and partial-frame state machine
- Session bridging via `TransactionManager` — one `Connection` per client; write
  contention surfaces as `WriteConflict`
- Clients never open the database directory — the server holds every file lock on their
  behalf (`Database::connect_tcp` in `akar-main`)
- `Server::shutdown()` and `local_addr()` (for ephemeral port `0`)
- Embedded single-process (no server) unchanged

**True shared-storage multi-process writers** remain out of scope: the storage
format (durable column mirrors + `BufferManager` mmap + `.ovf` sidecars) assumes
a single owner.

**Usage:**
```rust
use std::sync::Arc;
use akar_main::{Database, SystemConfig};
use akar_server::Server;

let db = Arc::new(Database::new("./my_db", SystemConfig::default())?);
let mut server = Server::bind("127.0.0.1:9876", db)?;
server.start()?;
```

**Tests:** 12 (concurrent write+read, crash client, DDL visibility, read-only
enforcement, embedded unchanged) + 5 unit tests in `akar-main` (`remote.rs`
frame/response).
