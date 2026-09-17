# Deep Exploration — akar-c

`akar-c` provides a panic-safe C ABI for calling Akar from C, C++, Swift, or any language with FFI. Eight functions cover the full lifecycle: connect → query → prepare → execute → rollback → close, plus error/leak handling.

## Key Components

| Function | Signature | Source |
|----------|-----------|--------|
| `akar_connect` | `akar_connect(path) → *mut AkarDatabase` | `akar-core/akar-c/src/lib.rs` |
| `akar_query` | `akar_query(db, query) → *mut c_char` | `akar-core/akar-c/src/lib.rs` |
| `akar_prepare` | `akar_prepare(db, sql) → *mut AkarConnection` | `akar-core/akar-c/src/lib.rs` |
| `akar_execute` | `akar_execute(conn, stmt, params) → *mut c_char` | `akar-core/akar-c/src/lib.rs` |
| `akar_rollback` | `akar_rollback(conn)` | `akar-core/akar-c/src/lib.rs` |
| `akar_close` | `akar_close(db)` | `akar-core/akar-c/src/lib.rs` |
| `akar_last_error` | `akar_last_error() → *const c_char` | `akar-core/akar-c/src/lib.rs` |
| `akar_free_result` | `akar_free_result(result)` | `akar-core/akar-c/src/lib.rs` |

## Design Decisions

- **`catch_unwind` everywhere.** Every public function wraps its body in `catch_unwind`; any Rust panic sets `LAST_ERROR` and returns null/void. The caller never sees a panic crossing the FFI boundary.
- **`akar_last_error` returns `CStr`.** Calling `akar_last_error()` retrieves the last panic message (or null if none), which C code can log with `fprintf(stderr, ...)`.

## Why It Matters

The C ABI is the widest-supported binding: it unlocks mobile (Swift), native plugins (Rust/Ruby/Java via JNI), and legacy C++ codebases. For agent memory, it is the integration point for non-Python hosts that want to embed Akar as a shared library.