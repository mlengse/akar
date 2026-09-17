# Deep Exploration — akar-wasm

`akar-wasm` exposes Akar to browser and Node.js through WebAssembly bindings. `AkarDatabase::new(db_path)` creates an in-memory (IndexedDB-backed) database; `AkarConnection::query(text)` runs Cypher and returns JSON.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `AkarDatabase` | Wasm-observable database handle | `akar-core/akar-wasm/src/lib.rs` |
| `AkarConnection` | Connection (Clone) for thread-local access | `akar-core/akar-wasm/src/` |
| `QueryResult::to_json()` | Convert result to JSON string | `akar-core/akar-wasm/src/` |
| target | `wasm32-unknown-unknown` only | `Cargo.toml` |

## Design Decisions

- **Full database in the browser.** All computation is local — no server sidecar. This is possible because Akar is pure Rust (no C/C++ FFI to block wasm builds). The alternative (WebSocket to a Rust server) was rejected for latency and deployment complexity.
- **Clone on `AkarConnection`.** Cloning is cheap (shared reference to same database) — matches the pattern where a worker thread clones the connection to run queries while the main thread holds the original.
- **18 tests.** All under wasm-bindgen-test.

## Why It Matters

Browser-based agents can embed their entire memory graph in Wasm, avoiding a network round-trip for every query. The Wasm build is also the strictest compilation gate — if it builds, there's no accidental C dependency.