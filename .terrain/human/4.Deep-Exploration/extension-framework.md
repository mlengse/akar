# Extension Framework (akar-extension)

**Module path:** `akar-core/akar-extension/`
**Role:** Core domain (framework) — the plugin surface every domain extension plugs into.

---

## Overview

`akar-extension` is a deliberately tiny framework that lets features attach themselves to the SQL surface. The whole contract is three pieces: an `Extension` trait (`name()` + `load(&ExtensionContext)`), an `ExtensionRegistry` that collects and loads extensions, and an `ExtensionContext` that hands each extension the capability registries — the `FunctionRegistry` (scalar/aggregate/table functions), the Catalog, and the VFS. Every domain extension in the ecosystem (`VectorExtension`, `FtsExtension`, `JsonExtension`, `MarkdownExtension`, `AlgoExtension`, `MlExtension`, HTTPFS, DuckDB, …) implements this one trait.

The design judgment is minimalism: one trait, one registry, one context. That single surface makes adding a feature predictable — write an `Extension`, append it to `register_builtin_extensions`, and its SQL objects exist. The same lifecycle serves built-in and third-party extensions uniformly.

## Core functions

1. **Trait contract** — `Extension` trait with `name()` (`lib.rs:22`) and `load()` (`lib.rs:27+`), `Send + Sync`.
2. **Registry** — `ExtensionRegistry::register` (`registry.rs:24`), `load_all` (`registry.rs:32`), loaded-state tracking / `is_loaded`.
3. **Context accessors** — `ExtensionContext` (`context.rs:11`) exposes `FunctionRegistry`, `Catalog`, `VirtualFileSystemRegistry`; plus `register_scalar_function` / `register_aggregate_function` / `register_table_function`.
4. **Wiring** — `register_builtin_extensions` (`akar-main/src/database.rs:758`) creates each domain extension and pushes it into the registry.

## Key components

| Component/type | File path | One-line responsibility |
|---|---|---|
| `Extension` | `akar-extension/src/lib.rs:20` | The plugin trait |
| `ExtensionRegistry` | `akar-extension/src/registry.rs:12` | Extension holder + `load_all` |
| `ExtensionContext` | `akar-extension/src/context.rs:11` | Capability exposure (functions, catalog, VFS) |
| load registration | `akar-main/src/database.rs:758` | `register_builtin_extensions` |
| consumers | `akar-extension`s | Each domain feature implements `Extension` |

## Internal data flow

```mermaid
flowchart LR
    A["register_builtin_extensions<br/>database.rs:758"] --> B["ExtensionRegistry"]
    B --> C["load_all"]
    C --> D["ext.load(&ExtensionContext)"]
    D --> E["register functions<br/>into FunctionRegistry"]
```

`load_all` iterates the registry, calls each `ext.load(&context)` with the shared `ExtensionContext`, and each extension registers its functions/table-functions into the `FunctionRegistry`. `is_loaded` guards against duplicate registration.

## Key interfaces & extension points

- **Implementing `Extension` is the entire extension hook.** There is nothing else a plugin must do.
- `ExtensionContext` method signatures define what extensions may register (scalar/aggregate/table functions, catalog operations, filesystem).
- Adding a feature = appending to `register_builtin_extensions` (`database.rs`).

## Interactions with other modules

| Module | Direction | Interface used |
|---|---|---|
| akar-function | → | Hosts `FunctionRegistry` / `ScalarFunction` / `TableFunction` types |
| akar-storage | → | Catalog/VFS access via context |
| all domain extensions | ← | Each depends on `ExtensionContext` |
| akar-main | → | Wires the registry at startup (`database.rs`) |

## Performance & concurrency notes

The `Send + Sync` bound means extensions can be shared across parallel processor slices. Registry/Context hold `Arc<Mutex<...>>` around the shared `FunctionRegistry` and Catalog. `is_loaded` prevents duplicate registration.

## Implementation highlights

- **Extremely small surface** — one trait + one registry + one context makes adoption trivial.
- **Uniform lifecycle** — the registry treats built-in and third-party extensions identically.
- **First-class catalog + VFS access**, not just function registration — extensions can reach schema and filesystem capabilities.