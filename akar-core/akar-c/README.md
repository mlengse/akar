# Akar C — C ABI Bindings

FFI bindings for the Akar embedded graph database, exposing a minimal C-compatible API over the Rust `akar-main` crate.

## Purpose

Provides a stable C ABI (`extern "C"`) so Akar can be embedded in non-Rust projects (C, C++, Python ctypes, etc.) without pulling in the Rust toolchain at the consumer side.

## Build Targets

| Target | Artifact | Notes |
|--------|----------|-------|
| `cdylib` | `akar_c.dll` / `libakar_c.so` / `libakar_c.dylib` | Dynamic library for dynamic linking |
| `staticlib` | `akar_c.lib` / `libakar_c.a` | Static archive for static linking |
| `lib` (rlib) | `libakar_c.rlib` | For Rust-only downstream crates |

Build with:

```bash
cargo build --release -p akar-c
```

Artifacts appear under `akar-core/target/release/`.

## Non-crates.io Status

This crate is **not published to crates.io** (`publish = false` in `Cargo.toml`). It is:

- Listed in `SKIP_CRATES` in `akar/tools/release.py`
- Excluded from the 33-crate crates.io publish order
- Built and tested as part of the Akar workspace
- Distributed only via GitHub Releases (prebuilt binaries) or as a workspace dependency

## Public C API

### Configuration

```c
typedef struct {
    uint64_t buffer_pool_size;
    uint64_t max_num_threads;
    bool enable_compression;
    bool read_only;
    uint64_t max_db_size;
    bool auto_checkpoint;
    uint64_t checkpoint_threshold;
} akar_system_config;

akar_system_config akar_default_system_config(void);
```

### Database Lifecycle

```c
typedef struct { void *_database; } akar_database;
typedef enum { AkarSuccess = 0, AkarError = 1 } akar_state;

akar_state akar_database_init(
    const char *database_path,
    akar_system_config config,
    akar_database *out_database
);

void akar_database_destroy(akar_database *database);
```

### Connection Management

```c
typedef struct { void *_connection; } akar_connection;

akar_state akar_connection_init(
    akar_database *database,
    akar_connection *out_connection
);

void akar_connection_destroy(akar_connection *connection);
```

### Query Execution

```c
typedef struct {
    void *_query_result;
    bool _is_owned_by_cpp;  // always false; result is Rust-owned
} akar_query_result;

akar_state akar_connection_query(
    akar_connection *connection,
    const char *query,
    akar_query_result *out_query_result,
    char **error_message  // optional; caller must free with akar_error_message_free
);

void akar_error_message_free(char *msg);
void akar_query_result_destroy(akar_query_result *query_result);
```

### Result Iteration

The `akar_query_result` is opaque. Use the Rust `akar-main` API for full result access, or wrap with a custom C iterator if needed.

## Safety

- All FFI functions are `unsafe extern "C"` and wrapped with `catch_unwind` to prevent panics from crossing the FFI boundary (which would be UB).
- The caller owns all heap allocations returned via out-parameters (`error_message`, `out_database`, etc.) and must call the corresponding `_destroy`/`_free` function.
- Never pass pointers to `free()` / `delete` across the FFI boundary — the Rust allocator owns them.

## Thread Safety

- `akar_database` is thread-safe (`Arc<Database>` internally). Multiple `akar_connection` handles can be created from the same database and used concurrently.
- Each `akar_connection` is `!Send + !Sync` (wraps a `Connection` which uses interior mutability). Use one connection per thread, or serialize access with a mutex on the C side.

## Versioning

The crate version follows the Akar workspace version (`version.workspace = true`). The C ABI is versioned alongside the Rust crate; breaking changes to the C structs or function signatures require a major version bump.

## License

GPL-3.0-or-later — same as the Akar workspace. The C ABI does not grant additional licensing rights; linking this library into a proprietary application still requires compliance with GPLv3.

## See Also

- **Akar-main Rust API**: [`../akar-main/README.md`](../akar-main/README.md)
- **Repository Specification**: [`../../SPEC.md`](../../SPEC.md) — §13.2 C FFI
- **Migration Guide**: [`../../MIGRATION.md`](../../MIGRATION.md) — C++ → Rust migration