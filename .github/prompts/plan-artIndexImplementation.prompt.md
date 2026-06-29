## Plan: ART Index Implementation for Kuzu Rust

**TL;DR** — Port the Adaptive Radix Tree (ART) primary key index from LadybugDB's C++ to Kuzu Rust (`kuzu-core`). ART enables **range scans on PK columns** (e.g. `WHERE p.id >= 10 AND p.id < 20`) via order-preserving byte-encoded keys with Node4/16/48/256 growth stages. The implementation spans 8 crates across parser, catalog, storage, binder, planner, optimizer, and processor — following the same integration patterns already established by `VectorIndex` (HNSW) and `HashIndex`.

**Approach:** Build incrementally from bottom-up:
1. Core ART data structure in `kuzu-storage` (ArtKey encoding + Node types + ArtPrimaryKeyIndex)
2. Persistence layer via BufferManager (checkpoint/restore)
3. Catalog registration (IndexType enum, NodeTableEntry index_type field)
4. SQL interface (parser grammar + binder + planner → DDL returns empty)
5. Query acceleration (optimizer pass + physical operator for range scan rewriting)

---

## Steps

### Phase A: Core ART Data Structure (kuzu-storage)

**A1** `kuzu-storage/src/art_key.rs` **(NEW)**
- `ArtKey` struct — wraps `Vec<u8>` (order-preserving byte encoding)
- Encoding functions (from `Value`):
  - `encode_int64(v)` → big-endian with sign flip (MSB XOR 1<<63)
  - `encode_int32(v)` → same pattern with 31-bit sign flip
  - `encode_uint64(v)` → plain big-endian
  - `encode_float64(v)` → IEEE 754 with sign flip for +0/-0 ordering
  - `encode_string(s)` → escape bytes 0x00→0x0100, append 0x00 terminator
  - `encode_date/encode_timestamp(v)` → delegate to int64 encoding
- `fn from_value(v: &Value) -> ArtKey` — dispatch by `LogicalTypeID`
- Template from C++ `ArtKey::encode()` in `ladybug/src/storage/index/art_index.cpp` lines 261–284

**A2** `kuzu-storage/src/art_node.rs` **(NEW)**
- `ArtNode` struct with 4 kind variants (comparable to C++ `Node` inner struct in `art_index.h` lines 95–145):
  - `Node4 { prefix: Vec<u8>, keys: [u8; 4], children: [Option<Box<ArtNode>>; 4], offsets: Vec<u64>, overflow_offsets: Vec<u64> }`
  - `Node16 { prefix, keys: [u8; 16], children: [Option<Box<ArtNode>>; 16], offsets, overflow_offsets }`
  - `Node48 { prefix, child_index: [u8; 256], children: [Option<Box<ArtNode>>; 48], offsets, overflow_offsets }`
  - `Node256 { prefix, children: [Option<Box<ArtNode>>; 256], offsets, overflow_offsets }`
- Growth transitions: NODE4→NODE16 at 5 children, NODE16→NODE48 at 17 children, NODE48→NODE256 at 49 children
- Methods: `get_child(byte)`, `insert_child(byte, node)`, `remove_child(byte)`, `grow()`, `shrink()`
- Arena allocation: `NodeBlock` with `NODE_BLOCK_CAPACITY = 16 * 1024` nodes per block
- C++ reference: `Node::getChild()`, `Node::insertChild()`, `Node::removeChild()` in `art_index.cpp`

**A3** `kuzu-storage/src/art_index.rs` **(NEW)**
- `ArtPrimaryKeyIndex` struct:
  ```rust
  pub struct ArtPrimaryKeyIndex {
      root: ArtNode,
      node_blocks: Vec<NodeBlock>,
      num_allocated_nodes: u64,
      num_nodes_by_kind: [u64; 4],
      // Persistence
      file_name: String,
      page_count: u64,
      dirty: bool,
  }
  ```
- Core operations (ported from C++ `ArtPrimaryKeyIndex` in `art_index.cpp`):
  - `insert(key: ArtKey, row_offset: u64)` — insert leaf with offset, handle overflow offsets for duplicates
  - `lookup(key: &ArtKey) -> Option<u64>` — exact match traversal
  - `delete(key: &ArtKey)` — remove leaf, cleanup empty nodes
  - `range_scan(lower: Option<&ArtKey>, upper: Option<&ArtKey>, max_results: u64) -> Vec<u64>` — collect all offsets within [lower, upper] via DFS with bound pruning (C++ `collectRange()` lines 924–985)
  - `len() -> usize` — count entries
- `fn from_value() -> ArtKey` conversion helper dispatches by `LogicalTypeID`

### Phase B: Persistence (kuzu-storage)

**B1** ART serialization/deserialization in `kuzu-storage/src/art_index.rs`
- `serialize_tree() -> Vec<u8>` — recursive serialization of ART nodes using varint encoding (C++ `serializeTree()`)
- `deserialize_tree(data: &[u8]) -> ArtNode` — recursive reconstruction (C++ `loadTree()` template)
- Varint encoding: `write_art_varint()`/`read_art_varint()` — LEB128-style (C++ `writeArtVarUint`/`readArtVarUint` in `art_index_disk_utils.h`)
- Page layout (following `VectorIndexTable` pattern from `vector_index.rs`):
  - Header page: magic number, root offset, total entries, num_pages
  - Data pages: serialized tree bytes
- `save(bm: &mut BufferManager)` — serialize to pages
- `load(bm: &mut BufferManager)` — deserialize from pages

**B2** Register ART index in `kuzu-storage/src/lib.rs`
- Re-export `ArtKey`, `ArtPrimaryKeyIndex`
- Add `art_index` module

### Phase C: Catalog Integration (kuzu-catalog)

**C1** `kuzu-catalog/src/lib.rs` — add `IndexType` enum
```rust
#[derive(Debug, Clone, PartialEq)]
pub enum IndexType {
    Hash,
    Art,
}
```

**C2** Add index info to `NodeTableEntry`
- New fields: `index_type: Option<IndexType>`, `index_name: Option<String>`
- New method: `has_art_index() -> bool`

**C3** Add catalog methods
- `create_index(table_name, index_name, index_type, column_name) -> Result`
- `drop_index(table_name, index_name) -> Result`
- `get_index_info(table_name) -> Option<(IndexType, String)>`

### Phase D: Storage Integration (kuzu-storage)

**D1** `kuzu-storage/src/table.rs` — Add `art_index` to `NodeTable`
```rust
pub struct NodeTable {
    // ... existing fields ...
    pub art_index: Option<ArtPrimaryKeyIndex>,
}
```

**D2** Integrate with `insert_row()` — when `art_index` is `Some`, auto-insert PK into ART after successful row insert (alongside existing hash_index update)

**D3** Integrate with `TableCatalog`
- `create_art_index(table_name, index_name, pk_column)` — creates ArtPrimaryKeyIndex, backfills existing rows
- `drop_art_index(table_name)` — removes the index
- `get_art_index(table_name) -> Option<&ArtPrimaryKeyIndex>`

**D4** `NodeTable::lookup_by_pk_range()` — new method: delegates to `art_index.range_scan()`
```rust
pub fn lookup_by_pk_range(&self, lower: Option<&Value>, upper: Option<&Value>, max: u64) -> Vec<u64>
```

### Phase E: SQL Interface (Parser → Binder → Planner)

**E1** `kuzu-parser/src/ast.rs` — Add DDL AST types
```rust
pub enum Statement {
    // ... existing ...
    CreateIndex(CreateIndex),
    DropIndex(DropIndex),
}

pub struct CreateIndex {
    pub index_type: String,       // "ART" | "HASH"
    pub index_name: String,
    pub table_name: String,
    pub variable: String,         // e.g., "n" in FOR (n:Person)
    pub property: String,         // e.g., "id" in ON (n.id)
    pub conflict_action: Option<String>, // IF NOT EXISTS
}

pub struct DropIndex {
    pub index_name: String,
    pub table_name: String,
}
```

**E2** `kuzu-parser/src/cypher.pest` — Add grammar rules
```pest
ddl_statement = {
    // ... existing ...
    | create_index
    | drop_index
}

create_index = {
    "CREATE" ~ index_type ~ "INDEX" ~ if_not_exists? ~ identifier
    ~ "FOR" ~ "(" ~ identifier ~ ":" ~ identifier ~ ")"
    ~ "ON" ~ "(" ~ identifier ~ "." ~ identifier ~ ")"
}

index_type = { "ART" | "HASH" }
if_not_exists = { "IF" ~ "NOT" ~ "EXISTS" }
drop_index = { "DROP" ~ "INDEX" ~ identifier ~ "ON" ~ identifier }
```

**E3** `kuzu-parser/src/parser.rs` — Add `parse_create_index()` and `parse_drop_index()`
- Pattern follows `parse_create_vector_index()` (lines 162–214)
- Extract index_type, index_name, table_name, variable, property from grammar pairs
- Return `Statement::CreateIndex(...)` or `Statement::DropIndex(...)`

**E4** `kuzu-binder/src/bound_statement.rs` — Add bound DDL types
```rust
pub enum BoundStatement {
    // ... existing ...
    BoundCreateIndex(BoundCreateIndex),
    BoundDropIndex(BoundDropIndex),
}

pub struct BoundCreateIndex {
    pub index_type: IndexType,
    pub index_name: String,
    pub table_name: String,
    pub column_name: String,
}

pub struct BoundDropIndex {
    pub index_name: String,
    pub table_name: String,
}
```

**E5** `kuzu-binder/src/binder.rs` — Add `bind_create_index()` and `bind_drop_index()`
- Pattern follows `bind_create_vector_index()`: validate name/table/column, register with catalog
- `bind_create_index()`: validate table exists in catalog, column exists, PK constraint, register index
- `bind_drop_index()`: validate index exists, remove from catalog

**E6** `kuzu-planner/src/planner.rs` — No-op planning (DDL)
- `BoundCreateIndex` and `BoundDropIndex` return `Ok(Vec::new())` — same pattern as all other DDL statements (line 22: `_ => Ok(Vec::new())`)

### Phase F: Query Acceleration (Optimizer → Processor)

**F1** `kuzu-planner/src/logical_operator.rs` — Add `LogicalArtIndexRangeScan`
```rust
pub enum LogicalOperator {
    // ... existing ...
    ArtIndexRangeScan(LogicalArtIndexRangeScan),
}

pub struct LogicalArtIndexRangeScan {
    pub table_name: String,
    pub table_id: u64,
    pub lower_bound: Option<Value>,
    pub upper_bound: Option<Value>,
    pub lower_inclusive: bool,
    pub upper_inclusive: bool,
    pub cardinality: u64,
}
```

**F2** `kuzu-optimizer/src/passes.rs` — Add `ArtRangeScanDetection` pass (registered as Pass 9, after VectorSimilarityDetection)

Pattern detection: `ScanNode + Filter(predicate on PK column with ART index)`  
Where predicate is one of:
- `p.pk >= lower AND p.pk < upper` → full range
- `p.pk >= lower` → lower-only
- `p.pk < upper` → upper-only
- `p.pk = value` → single point (delegates to lookup, not range)

Rewrite to: `ArtIndexRangeScan(table_name, lower_bound, upper_bound, ...)`

This mirrors `VectorSimilarityDetection` (lines 321–385 in `passes.rs`) — it looks at the flat operator list, recognizes a pattern, and replaces 2-3 operators with 1 index scan operator.

Key differences from VectorSimilarity:
- No OrderBy+Limit requirement — just ScanNode+Filter(comparison on PK)
- Extracts comparison bounds (`>=`, `<=`, `<`, `>`, `=`) from BinaryOp expressions
- Validates column is PK and table has ART index (via catalog lookup)
- `satisfiesLowerBound`/`satisfiesUpperBound` semantics — follows C++ `collectRange()` bound checking

**F3** Register `ArtRangeScanDetection` in `kuzu-optimizer/src/optimizer.rs` — add to both `Optimizer::new()` and `Optimizer::with_stats()` pass lists

**F4** `kuzu-processor/src/physical_operator.rs` — Add `PhysicalArtIndexRangeScan`
```rust
pub struct PhysicalArtIndexRangeScan {
    pub table_name: String,
    pub table_id: u64,
    pub lower_bound: Option<Value>,
    pub upper_bound: Option<Value>,
    pub lower_inclusive: bool,
    pub upper_inclusive: bool,
    pub table_catalog: Option<Arc<TableCatalog>>,
}

impl PhysicalOperatorExec for PhysicalArtIndexRangeScan {
    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // 1. Get NodeTable from catalog
        // 2. Convert bounds to ArtKey via ArtKey::from_value()
        // 3. Call node_table.lookup_by_pk_range(lower, upper, max)
        //    → delegates to art_index.range_scan()
        // 4. Fetch columns for matched row IDs
        // 5. Build DataChunk
    }
}
```

Pattern follows `PhysicalVectorSimilarityScan` exactly (processor.rs lines 70–81 + `physical_operator.rs` lines where it dispatches).

**F5** `kuzu-processor/src/processor.rs` — Add `LogicalOperator::ArtIndexRangeScan` dispatch (alongside other scan variants)
```rust
LogicalOperator::ArtIndexRangeScan(ars) => {
    let scan = PhysicalArtIndexRangeScan { ... };
    let result = scan.execute(current.clone())?;
    intermediate_result = Some(result);
}
```

### Phase G: NodeTable Range Lookup Integration

**G1** `kuzu-storage/src/table.rs` — `NodeTable::lookup_by_pk_range()`
```rust
pub fn lookup_by_pk_range(
    &self,
    lower: Option<&Value>,
    upper: Option<&Value>,
    max_results: u64,
) -> Vec<u64> {
    match &self.art_index {
        Some(idx) => {
            let lower_key = lower.map(|v| ArtKey::from_value(v));
            let upper_key = upper.map(|v| ArtKey::from_value(v));
            idx.range_scan(lower_key.as_ref(), upper_key.as_ref(), max_results)
        }
        None => Vec::new(), // fallback: optimizer shouldn't have produced this
    }
}
```

### Phase H: Tests & Verification

**H1** Unit tests in `kuzu-storage/src/art_index.rs`:
- `test_art_key_encoding_int64` — verify order-preserving encoding: 0 → 1 → MAX_INT → MIN_INT ordering
- `test_art_key_encoding_string` — verify lexicographic ordering with escape sequences
- `test_art_insert_lookup` — basic insert + exact lookup
- `test_art_range_scan_basic` — range scan on sorted data with `[lower, upper)`
- `test_art_range_scan_open_bounds` — unbounded lower/upper
- `test_art_node_growth` — insert 100 keys, verify node type transitions
- `test_art_delete` — insert + delete + verify gone
- `test_art_duplicate_handling` — overflow offsets for duplicate PK values (secondary index use)

**H2** Integration tests in `kuzu-main`:
- `test_create_art_index_ddl` — `CREATE ART INDEX idx FOR (p:Person) ON (p.id)` via Cypher
- `test_art_range_scan_query` — `MATCH (p:Person) WHERE p.id >= 10 AND p.id < 20 RETURN p.name`
- `test_art_vs_hash_consistency` — ensure ART and HASH produce same results for equality lookups
- `test_art_auto_insert` — verify ART index auto-updates on CREATE/INSERT

**H3** Regression tests: `cargo test --workspace` — all 550+ existing tests must pass

---

## Relevant Files

| File | What to do |
|------|-----------|
| `kuzu-core/kuzu-storage/src/art_key.rs` | **NEW** — ArtKey encoding/decoding |
| `kuzu-core/kuzu-storage/src/art_node.rs` | **NEW** — Node4/16/48/256 types with arena allocation |
| `kuzu-core/kuzu-storage/src/art_index.rs` | **NEW** — ArtPrimaryKeyIndex (insert/lookup/delete/range_scan/save/load) |
| `kuzu-core/kuzu-storage/src/lib.rs` | Add `pub mod art_key; pub mod art_node; pub mod art_index;` re-exports |
| `kuzu-core/kuzu-storage/src/table.rs` | Add `art_index: Option<ArtPrimaryKeyIndex>` to `NodeTable`. Add `lookup_by_pk_range()`. Update `insert_row()` to auto-index. |
| `kuzu-core/kuzu-catalog/src/lib.rs` | Add `IndexType` enum. Add `index_type`, `index_name` to `NodeTableEntry`. Add `create_index()`/`drop_index()`/`get_index_info()` cat methods. |
| `kuzu-core/kuzu-parser/src/ast.rs` | Add `Statement::CreateIndex`, `Statement::DropIndex`, `CreateIndex` struct, `DropIndex` struct |
| `kuzu-core/kuzu-parser/src/cypher.pest` | Add `create_index`, `drop_index`, `index_type` grammar rules |
| `kuzu-core/kuzu-parser/src/parser.rs` | Add `parse_create_index()`, `parse_drop_index()`. Register in `parse_ddl()`. |
| `kuzu-core/kuzu-binder/src/bound_statement.rs` | Add `BoundCreateIndex`, `BoundDropIndex`, `BoundStatement` variants |
| `kuzu-core/kuzu-binder/src/binder.rs` | Add `bind_create_index()`, `bind_drop_index()`. Register in `bind()`. |
| `kuzu-core/kuzu-planner/src/logical_operator.rs` | Add `LogicalArtIndexRangeScan` struct + enum variant |
| `kuzu-core/kuzu-planner/src/planner.rs` | No-op for DDL (already handled by `_ => Ok(vec![])`) |
| `kuzu-core/kuzu-optimizer/src/passes.rs` | Add `ArtRangeScanDetection` pass. Extract comparison bounds from `BinaryOp(Gt/Lt/Ge/Le/Eq, prop_access, constant)`. |
| `kuzu-core/kuzu-optimizer/src/optimizer.rs` | Register `ArtRangeScanDetection` in both constructors. Update pass count test. |
| `kuzu-core/kuzu-processor/src/physical_operator.rs` | Add `PhysicalArtIndexRangeScan` struct + `PhysicalOperatorExec` impl |
| `kuzu-core/kuzu-processor/src/processor.rs` | Add `LogicalOperator::ArtIndexRangeScan` dispatch |
| `kuzu-core/kuzu-storage/src/vector_index.rs` | **Reference** — persistence pattern (save/load via BufferManager header+data pages) |
| `kuzu-core/kuzu-storage/src/index.rs` | **Reference** — HashIndex pattern, NodeTable integration |
| `ladybug/src/storage/index/art_index.h` | **Reference** — C++ ART class definition |
| `ladybug/src/storage/index/art_index.cpp` | **Reference** — C++ ART implementation |
| `ladybug/src/include/storage/index/art_index_disk_utils.h` | **Reference** — C++ serialization utilities |

---

## Verification

1. **Unit tests** — `cargo test -p kuzu-storage -- art` — 8+ new tests for ArtKey encoding, ART ops, node growth, persistence roundtrip
2. **Integration tests** — `cargo test -p kuzu-main -- art` — 4+ tests for Cypher DDL, range scan queries, consistency
3. **Full regression** — `cargo test --workspace` — all 550+ existing tests must pass, zero clippy warnings
4. **Build** — `cargo build --workspace` — clean build, no errors
5. **Manual verification**: Range scan query `MATCH (p:Person) WHERE p.id >= 100 AND p.id <= 200 RETURN p.name` should return correct subset without full table scan

---

## Scope Boundaries

**Included:**
- ART index for PK range scans (primary use case)
- Order-preserving byte encoding for all Kuzu primitive types (Int64, Int32, UInt64, Float64, String, Date, Timestamp)
- Persistence via BufferManager (save/load, checkpoint/restore)
- Cypher grammar: `CREATE ART INDEX name FOR (n:Label) ON (n.prop)` and `DROP INDEX name ON table`
- Optimizer pass that rewrites `ScanNode + Filter(inequality on PK with ART index)` → `ArtIndexRangeScan`
- Physical operator that executes the range scan and fetches columns

**Excluded (deferred):**
- ART as secondary index (non-PK columns) — the C++ supports it but initial scope is PK-only
- Concurrent thread safety (`Mutex` wrapping) — initial version is single-threaded, matching HashIndex pattern
- HASH index via DDL (`CREATE HASH INDEX`) — grammar parses it but `HASH` is the default; ART is the new option
- Multi-column ART indexes — C++ supports single-column only, match that
- Dedicated `physical_operator.rs` file change for non-range ART filter patterns (e.g., `>` only or `<` only) — covered by the same optimizer pass

---

## Decisions

1. **Arena allocation via Vec<NodeBlock>** over raw pointer arithmetic — Rust safety. Each NodeBlock holds fixed-size nodes in a `Vec<Node>`. Matches C++ `NodeBlock` pattern.
2. **Box<ArtNode> for children** over raw pointers — Rust ownership model. The Box indirection is needed for recursive types and matches C++ Node* semantics.
3. **`Vec<u64>` for overflow offsets** instead of linked list — matches C++ `overflowOffsets: unique_ptr<vector<offset_t>>` pattern for duplicate PK values.
4. **Single-threaded initially** — same as `HashIndex`. Thread safety can be added with `Mutex` wrapping later when concurrent ART operations are needed.
5. **`ArtKey::from_value(&Value)` as dispatch** rather than C++'s typed `ValueVector` approach — the Rust `Value` enum already holds type info, making dispatch simpler.
