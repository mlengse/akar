# Deep Exploration — akar-json

A JSON extension crate that registers scalar functions for extracting and inspecting JSON in Cypher queries. Simple, focused, and entirely opt-in via `LOAD EXTENSION "akar-json"`.

## Key Components

| Function | Purpose | Source |
|----------|---------|--------|
| `json_valid` / `json_validity` | Validate JSON strings | `akar-core/akar-json/src/lib.rs` |
| `json_extract` (and variants) | Extract a value by path | `akar-core/akar-json/src/lib.rs` |
| `json_keys` | List keys of a JSON object | `akar-core/akar-json/src/lib.rs` |
| `json_structure` | Show structural outline | `akar-core/akar-json/src/lib.rs` |

## Design Decisions

- **Separated from the core function registry.** JSON is optional surface; keeping it behind an extension means the core build stays lean and the extension can evolve independently (e.g. adding serialization functions without touching `akar-function`).
- **Extension registration only.** The crate contains no execution logic of its own — it only registers functions that the processor evaluates through the normal expression path.

## Why It Matters

Memory payloads are frequently JSON-shaped (tool outputs, conversation metadata). `json_extract` in WHERE/RETURN clauses is the cheapest way to filter agent memories by semantic keys without normalizing every document into columns.