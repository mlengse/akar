# Plan: Fix warnings, remove `unsafe`, resolve TODOs in Rust code

## TL;DR
Address 5 compiler warnings in `kuzu-core`, eliminate `unsafe` surface area in `ladybug/tools/rust_api/src/` by encapsulating raw pointer dereferences into safe helper methods, and resolve 5 TODO comments across the Rust API.

---

## Phase 1: Fix Compiler Warnings (`kuzu-core`)

### Step 1.1 — Fix unused variable `row_count`
- **File**: `kuzu-core/kuzu-storage/src/lib.rs:974`
- **Problem**: `let row_count = { ... count }` — the block assigns to `row_count` but it's never read afterward
- **Fix**: Prefix with underscore: `let _row_count =`
- Keeps the block's side effects intact while suppressing the warning

### Step 1.2 — Fix 4 unused imports
- **File**: `kuzu-core/kuzu-main/src/connection.rs`
- **Lines**: 1790, 1986, 2037, 2084
- **Problem**: `use kuzu_common::types::Value;` is unused in 4 test modules: `call_tests`, `foreach_tests`, `var_length_path_tests`, `subquery_tests`
- **Fix**: Remove the unused import from each module

### Verification
```
cargo build --workspace   # zero warnings
```

---

## Phase 2: Remove `unsafe` in ladybug Rust API (`ladybug/tools/rust_api/`)

### Problem
11 repeated `unsafe { (*self.conn.get()).pin_mut() }` calls in `connection.rs` + 1 in `database.rs:Connection::new()`. A code comment says _"Turning this into a function just causes lifetime issues"_ — but this can be resolved with proper lifetime elision.

### Step 2.1 — Add safe helper on `Connection`
- **File**: `ladybug/tools/rust_api/src/connection.rs`
- Add method:
  ```rust
  /// Returns a pinned mutable reference to the underlying C++ Connection.
  ///
  /// # Safety
  /// The C++ Connection is internally synchronized via a mutex, so producing
  /// multiple `Pin<&mut ...>` references from shared `&self` access is sound.
  fn conn_pin_mut(&self) -> Pin<&mut ffi::Connection<'a>> {
      unsafe { (*self.conn.get()).pin_mut() }
  }
  ```
- The returned `Pin<&mut ...>` borrows from `&self` via lifetime elision

### Step 2.2 — Add safe helper on `Database`
- **File**: `ladybug/tools/rust_api/src/database.rs`
- Add:
  ```rust
  pub(crate) fn db_pin_mut(&self) -> Pin<&mut ffi::Database> {
      unsafe { (*self.db.get()).pin_mut() }
  }
  ```

### Step 2.3 — Replace all unsafe dereferences in `connection.rs`
- Replace all 12 occurrences of `unsafe { (*self.conn.get()).pin_mut() }` with `self.conn_pin_mut()`
- Replace the 1 `unsafe { (*database.db.get()).pin_mut() }` with `database.db_pin_mut()`
- Locations: `new()`, `get_max_num_threads_for_exec()`, `prepare()`, `query()`, `query_as_arrow()`, `create_arrow_table()`, `create_arrow_rel_table()`, `create_arrow_rel_table_csr()`, `drop_arrow_table()`, `execute()`, `interrupt()`, `set_query_timeout()`

### Step 2.4 — Add SAFETY comments to `unsafe impl Send/Sync`
- `connection.rs:119-120` — Document: C++ Connection is mutex-synchronized
- `database.rs:14-15` — Document: C++ Database is internally synchronized
- `query_result.rs:14` — Document: `ffi::QueryResult` holds no non-Send state
- These `unsafe impl` blocks **cannot be removed** (UnsafeCell is !Send/!Sync by default) but should be documented

### Step 2.5 — SAFETY comments on Arrow FFI calls in `query_result.rs`
- `import_i64_array()` at line 132: `unsafe { arrow::ffi::from_ffi_and_data_type(...) }` — add comment explaining the C++ side guarantees a valid ArrowArray
- `ArrowIterator::next()` at line 220: `unsafe { arrow::ffi::from_ffi(...) }` — same treatment
- These are inherent to the Arrow C Data Interface and cannot be eliminated

### Verification
```
cd ladybug/tools/rust_api && cargo build   # zero warnings, zero errors
```

---

## Phase 3: Resolve TODOs

### Step 3.1 — connection.rs:172: Generic QueryResult
- **TODO**: Make `QueryResult<T>` generic for compile-time type safety
- **Decision**: **Deferred**. This is a major API redesign affecting all downstream consumers
- **Fix**: Update comment to reference a future tracking issue

### Step 3.2 — query_result.rs:168: Return Result instead of unwrapping
- **TODO**: `Iterator::next()` uses `value.try_into().unwrap()`
- **Fix**: Add `QueryResult::next_result(&mut self) -> Option<Result<Vec<Value>, Error>>` method that returns results without panicking
- The `Iterator` impl keeps panicking (trait constraint), but consumers can use `next_result()` for fallible iteration

### Step 3.3 — value.rs:244: Enforce type of contents in List/Array
- **TODO**: `List(LogicalType, Vec<Value>)` variant has a type hint but doesn't validate contents match
- **Fix**: Add `Value::new_list(LogicalType, Vec<Value>) -> Result<Value, Error>` constructor that validates each element's type matches the declared `LogicalType`
- The existing `TryInto<UniquePtr<ffi::Value>>` impl should also validate

### Step 3.4 — value.rs:691: Better error message for unsupported types
- **TODO**: Catch-all arm `x => panic!("Unsupported type {x:?}")` panics instead of returning an error
- **Fix**: Change `TryFrom<&ffi::Value>` impl to return `Err(Error::UnsupportedType(...))` instead of panicking
- **Dependency**: Add `UnsupportedType(String)` variant to `Error` enum in `error.rs`

### Step 3.5 — value.rs:1067: Test equivalence to value inside a query
- **TODO**: Tests only check round-trip through Rust → C++ → Rust, not equivalence with value produced inside a Cypher query
- **Fix**: For each type test, add a companion query that returns that type directly from Cypher (e.g., `RETURN 42`) and assert the returned value equals the Rust-constructed value

### Verification
```
cd ladybug/tools/rust_api && cargo test   # all tests green
```

---

## Files to Modify (Summary)

| File | Changes |
|------|---------|
| `kuzu-core/kuzu-storage/src/lib.rs` | Prefix `row_count` → `_row_count` |
| `kuzu-core/kuzu-main/src/connection.rs` | Remove 4 unused `use kuzu_common::types::Value;` |
| `ladybug/tools/rust_api/src/connection.rs` | Add `conn_pin_mut()` helper; replace 12 unsafe derefs; add SAFETY docs on Send/Sync; update TODO comment |
| `ladybug/tools/rust_api/src/database.rs` | Add `db_pin_mut()` helper; add SAFETY docs on Send/Sync |
| `ladybug/tools/rust_api/src/query_result.rs` | Add SAFETY comments on `from_ffi`; add `next_result()` method; add SAFETY comment on Send |
| `ladybug/tools/rust_api/src/value.rs` | Add `Value::new_list()` constructor; change `TryFrom` panic → `Err`; add test assertions |
| `ladybug/tools/rust_api/src/error.rs` | Add `UnsupportedType(String)` variant |

---

## Key Decisions

1. **`unsafe impl Send/Sync`** — Retained with SAFETY docs. These are soundness assertions that cannot be removed because `UnsafeCell` is !Send/!Sync by default.
2. **`arrow::ffi::from_ffi*`** — Kept as `unsafe` with SAFETY docs. The Arrow C Data Interface is inherently unsafe; safety depends on the C++ side providing valid data.
3. **Generic `QueryResult<T>` TODO** — Deferred. This is a major API redesign that would touch all downstream consumers.
