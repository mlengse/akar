# Deep Exploration — akar-migrate

`akar-migrate` is the storage migration CLI for moving from Akar's legacy C++ storage format (Vela) to the current Rust-native format (STORAGE_VERSION = 1). It provides a `diskformat check` command and a `create-migration` tool.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `diskformat check` | Verify current storage format version | `akar-core/akar-migrate/src/` |
| `create-migration` | Generate migration plan from C++ format to Rust format | `akar-core/akar-migrate/src/` |
| Version detection | Read `akar.lock` + column file headers to determine source format | `akar-core/akar-migrate/src/` |

## Design Decisions

- **Explicit migration over automatic upgrade.** The alternative — transparent migration at `Database::new` time — was rejected because silent format changes are dangerous in multi-process environments (two processes with different format versions would corrupt the same DB). Explicit CLI migration gives control.

## Why It Matters

The C++ era is historical but real: pre-0.1.x databases used C++ storage format (noted in ADR-004, though CSR was a stub at that time). `akar-migrate` is the bridge that keeps those databases usable as Akar evolves — a one-time operation that enables the rest of the Rust-native stack.