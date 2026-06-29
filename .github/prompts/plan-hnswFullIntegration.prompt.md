## Plan: HNSW Full Integration — Parser, Catalog, Persistence, VectorSimilarityScan

**TL;DR** Integrate the existing in-memory `HnswIndex` (`kuzu-vector`) into the full Kuzu pipeline: `CREATE VECTOR INDEX` DDL → catalog registration → BufferManager-backed persistence → `VectorSimilarityScan` physical operator for ANN queries. Three query syntaxes, three population strategies — all supported.

---

## Phase 1: Parser — `CREATE VECTOR INDEX` DDL

### Files to modify
- `kuzu-core/kuzu-parser/src/ast.rs` — add `CreateVectorIndex` struct + `Statement::CreateVectorIndex`
- `kuzu-core/kuzu-parser/src/cypher.pest` — add grammar rules
- `kuzu-core/kuzu-parser/src/parser.rs` — add `parse_create_vector_index()` and dispatch

### Steps
1. **AST struct** (`ast.rs`):
   ```rust
   pub struct CreateVectorIndex {
       pub index_name: String,
       pub table_name: String,
       pub column_name: String,
       pub metric: String,   // "cosine" | "euclidean" | "l2" | "dot"
       pub dimensions: u64,
   }
   ```
   Add `Statement::CreateVectorIndex(CreateVectorIndex)` variant.

2. **Grammar** (`cypher.pest`):
   ```pest
   create_vector_index = {
       "CREATE" ~ "VECTOR" ~ "INDEX" ~ identifier ~ "ON"
       ~ "(" ~ identifier ~ "." ~ identifier ~ ")"
       ~ "WITH" ~ "(" ~ vector_index_options ~ ")"
   }
   vector_index_options = { vector_index_option ~ ("," ~ vector_index_option)* }
   vector_index_option = {
       metric_option | dimensions_option
   }
   metric_option = { "metric" ~ "=" ~ ("cosine" | "euclidean" | "l2" | "dot") }
   dimensions_option = { "dims" ~ "=" ~ integer }
   ```
   Add `create_vector_index` to `ddl_statement` alternation.

3. **Parser** (`parser.rs`): Add `parse_create_vector_index(pair)` that extracts name, table_name, column_name, metric, dimensions from PEG pairs. Dispatch from `parse_ddl()`.

### Verification
- `parse("CREATE VECTOR INDEX idx ON (items.embedding) WITH (metric=cosine, dims=128)")` → `Statement::CreateVectorIndex(...)`
- `parse("CREATE VECTOR INDEX my_idx ON (docs.vec) WITH (metric=l2, dims=256)")` → parses correctly
- Error on missing/invalid metric or missing dims

---

## Phase 2: Catalog Registration

### Files to modify
- `kuzu-core/kuzu-catalog/src/lib.rs` — add `VectorIndexEntry`, `CatalogEntry::VectorIndex`, catalog methods
- `kuzu-core/kuzu-binder/src/bound_statement.rs` — add `BoundCreateVectorIndex`
- `kuzu-core/kuzu-binder/src/binder.rs` — add `bind_create_vector_index()`, dispatch in `bind()`

### Steps
1. **Catalog entry** (`kuzu-catalog/src/lib.rs`):
   ```rust
   pub struct VectorIndexEntry {
       pub index_id: u64,
       pub name: String,
       pub table_name: String,
       pub column_name: String,
       pub metric: String,
       pub dimensions: u64,
   }
   ```
   Add `CatalogEntry::VectorIndex(VectorIndexEntry)` variant. Add `is_vector_index()`, `as_vector_index()` helpers. Add `create_vector_index()`, `drop_vector_index()`, `list_vector_indexes()` methods on `Catalog`.

2. **Bound statement** (`kuzu-binder/src/bound_statement.rs`):
   ```rust
   pub struct BoundCreateVectorIndex {
       pub index_name: String,
       pub table_name: String,
       pub column_name: String,
       pub metric: String,
       pub dimensions: u64,
   }
   ```
   Add `BoundStatement::BoundCreateVectorIndex(BoundCreateVectorIndex)`.

3. **Binder** (`kuzu-binder/src/binder.rs`): Add `bind_create_vector_index()`:
   - Validate the referenced table exists in catalog
   - Validate the referenced column exists and is a `FLOAT[]` / `DOUBLE[]` type (or similar)
   - Call `catalog.create_vector_index(...)` to register
   - Return `BoundStatement::BoundCreateVectorIndex(...)`

4. **Storage side**: Add `VectorIndexTable` or similar to `kuzu-storage/src/table.rs` (or a new `kuzu-storage/src/vector_index.rs`):
   - Wraps `HnswIndex` (from `kuzu-vector`)
   - Adds persistence layer (BufferManager-backed pages)
   - `TableCatalog` gets a new `vector_indexes: DashMap<u64, VectorIndexTable>` map

### Verification
- Catalog round-trip: create → lookup by name → metadata matches
- Binder: validates table exists, column exists
- Error on duplicate index name, missing table, missing column

---

## Phase 3: Persistence via BufferManager

### Files to modify
- `kuzu-core/kuzu-storage/src/vector_index.rs` (new) — `VectorIndexTable` with BM-backed HNSW
- `kuzu-core/kuzu-storage/src/lib.rs` — `StorageManager` methods for vector index
- `kuzu-core/kuzu-storage/src/table.rs` — `TableCatalog` additions
- `kuzu-core/kuzu-storage/src/index.rs` — reference pattern (`OnDiskHashIndex`)

### Steps
1. **New file `kuzu-storage/src/vector_index.rs`**:
   - `VectorIndexTable` struct:
     ```rust
     pub struct VectorIndexTable {
         pub index_id: u64,
         pub name: String,
         pub table_name: String,
         pub column_name: String,
         pub hnsw: HnswIndex,
         page_count: u64,
         file_name: String,
         dirty: bool,
     }
     ```
   - Follow `OnDiskHashIndex` persistence pattern:
     - **Page layout**: Header page (metric, dims, num_vectors, entry_point, max_level) + data pages (serialized nodes + connections)
     - **`save(&mut self, bm: &mut BufferManager)`**: Serialize all in-memory HNSW state to pages, mark dirty, unpin, flush
     - **`load(&mut self, bm: &mut BufferManager)`**: Register file, read header page, deserialize nodes+connections, rebuild in-memory HNSW
     - **`insert(&mut self, vector: Vec<f64>, id: usize)`**: Delegate to `self.hnsw.insert()`, mark dirty
     - **`search(&self, query: &[f64], k: usize)` → `Vec<(f64, usize)>`**: Delegate to `self.hnsw.search()`

2. **StorageManager** (`kuzu-storage/src/lib.rs`):
   - Add `create_vector_index(...)` → creates `VectorIndexTable`, registers with BM, returns index_id
   - Add `get_vector_index(...)` → lookup by name/id
   - Add `drop_vector_index(...)` → removes from catalog, frees resources
   - Add `vector_indexes()` → accessor for `TableCatalog`

3. **TableCatalog** (`kuzu-storage/src/table.rs`):
   - Add `vector_indexes: DashMap<u64, VectorIndexTable>`
   - Add `vector_index_name_to_id: DashMap<String, u64>`
   - Add `next_vector_index_id: AtomicU64`
   - Add `create_vector_index()`, `get_vector_index_by_name()`, etc.

4. **Dependencies**: `kuzu-storage` needs to depend on `kuzu-vector` (or the HNSW types move to `kuzu-common`). **Recommendation**: Move `HnswIndex` and `DistanceMetric` to a shared location (`kuzu-common` or new `kuzu-hnsw` crate) so both `kuzu-storage` and `kuzu-vector` can use it without circular deps.

### Verification
- Create vector index → file registered with BM → persists properly
- Insert vectors → mark dirty → flush → file on disk has correct content
- Load from disk → rebuild HNSW → search returns correct results
- Round-trip test: create → insert → save → load → search same results

---

## Phase 4: Connection DDL Wiring

### Files to modify
- `kuzu-core/kuzu-main/src/connection.rs` — add `BoundCreateVectorIndex` to `handle_ddl()` and `is_write_statement()`

### Steps
1. In `handle_ddl()`:
   ```rust
   BoundStatement::BoundCreateVectorIndex(idx) => {
       let id = self.database.storage_manager.create_vector_index(
           &idx.index_name,
           &idx.table_name,
           &idx.column_name,
           &idx.metric,
           idx.dimensions,
       )?;
       // Auto-populate: scan table and insert existing vectors
       if let Some(table) = self.database.storage_manager.table_catalog()
           .get_node_table_by_name(&idx.table_name) {
           let col_idx = table.columns.iter()
               .position(|c| c.name == idx.column_name).unwrap();
           // Iterate rows, extract f64[] values, insert into HNSW
           for row_id in 0..table.num_rows {
               if let Some(val) = table.get_value(col_idx, row_id) {
                   if let Ok(vec) = extract_f64_list(&val) {
                       let mut vindex = self.database.storage_manager
                           .get_vector_index_by_name(&idx.index_name)?;
                       vindex.insert(vec, row_id);
                   }
               }
           }
       }
       Ok(Some(QueryResult::success_message(
           format!("Vector index '{}' created", idx.index_name)
       )))
   }
   ```
2. Add `BoundCreateVectorIndex` to `is_write_statement()` match.
3. On node INSERT (in connection or processor), also insert vector into the corresponding vector index.

### Verification
- `CREATE VECTOR INDEX idx ON (items.embedding) WITH (metric=cosine, dims=3)` → success message
- Index creates empty (no data) or auto-populated depending on table state
- Index appears in catalog listing

---

## Phase 5: Physical Operator `VectorSimilarityScan`

### Files to modify
- `kuzu-core/kuzu-planner/src/logical_operator.rs` — add `VectorSimilarityScan` variant and struct
- `kuzu-core/kuzu-planner/src/planner.rs` — produce operator from bound query
- `kuzu-core/kuzu-processor/src/physical_operator.rs` — add `PhysicalVectorSimilarityScan` struct + `PhysicalOperatorExec` impl
- `kuzu-core/kuzu-processor/src/processor.rs` — dispatch logical→physical

### Steps
1. **Logical operator** (`logical_operator.rs`):
   ```rust
   pub struct LogicalVectorSimilarityScan {
       pub index_name: String,
       pub index_id: u64,
       pub query_vector: Vec<f64>,
       pub top_k: u64,
       pub table_name: String,
       pub cardinality: u64,
   }
   ```
   Add `LogicalOperator::VectorSimilarityScan(LogicalVectorSimilarityScan)`.

2. **Physical operator** (`physical_operator.rs`):
   ```rust
   pub struct PhysicalVectorSimilarityScan {
       pub index_name: String,
       pub index_id: u64,
       pub query_vector: Vec<f64>,
       pub top_k: u64,
       pub table_name: String,
       pub table_catalog: Option<Arc<TableCatalog>>,
       pub vector_index_table: Option<Arc<Mutex<VectorIndexTable>>>,
   }
   ```
   `execute()`:
   - Access the `VectorIndexTable` via `table_catalog`
   - Call `hnsw.search(&query_vector, top_k)` → `Vec<(f64, usize)>`
   - For each result `(distance, row_id)`, look up the corresponding row from `NodeTable`
   - Build output `DataChunk` with columns: all table columns + distance column
   - Return chunk

3. **Processor** (`processor.rs`): Map `LogicalOperator::VectorSimilarityScan` → `PhysicalVectorSimilarityScan`, resolve vector index from `TableCatalog`, execute.

### Verification
- Create table with vector column, insert data, create vector index
- CALL or MATCH query returns top-K nearest neighbors with distances
- Edge cases: empty index, K=0, K greater than vector count
- Results ordered by distance ascending

---

## Phase 6: Query Syntax Integration (All Three Approaches)

### 6a: `CALL vector_similarity_scan(...)` table function
- Register a `CustomTable` function in `kuzu-vector` (or `kuzu-function`)
- `vector_similarity_scan(table_name, column_name, query_vector, top_k)` → returns rows
- Use `LogicalOperator::TableFunctionCall` path (already exists!)
- Processor's `execute_table_function` dispatches to the `CustomTable` callback, which searches the index and produces a DataChunk

### 6b: WHERE with distance function + ORDER BY + LIMIT
- Already works today! The 4 scalar functions (cosine_similarity, etc.) are registered
- Users can write: `MATCH (n:Items) WHERE cosine_similarity(n.embedding, $query) > 0.9 RETURN n ORDER BY cosine_similarity(n.embedding, $query) DESC LIMIT 10`
- This uses brute-force scan (no index acceleration by default)
- **Index-aware optimization**: Add an optimizer pass that detects the pattern `distance_fn(n.column, $param) < threshold` + `ORDER BY` + `LIMIT K` and rewrites it to use `VectorSimilarityScan`

### 6c: Dedicated MATCH syntax
- Add new PEG grammar for similarity match (e.g., `<->` operator)
- Parse into a new `Clause` variant or expression type
- Bind and plan as `VectorSimilarityScan`
- **Deferred to later phase**: Complex syntax extension; Phase 6a + 6b cover the practical use cases

### Steps
1. **6a**: Add `vector_similarity_scan` as a `CustomTable` function in `kuzu-vector/src/lib.rs`:
   ```rust
   context.register_table_function(
       "vector_similarity_scan",
       TableFunction::CustomTable {
           name: "vector_similarity_scan".into(),
           execute: Arc::new(|args: &[Value], output: &mut DataChunk| {
               // args: [table_name, column_name, query_vector, top_k]
               // Lookup index, search, fill output chunk
               Ok(())
           }),
       },
   );
   ```

2. **6b**: Create a new optimizer pass `VectorSimilarityDetection` in `kuzu-optimizer`:
   - Scans logical plan for: Filter(distance_fn) → OrderBy → Limit K
   - When detected, replaces with `VectorSimilarityScan`
   - Falls back to brute-force if not applicable

### Verification
- `CALL vector_similarity_scan('items', 'embedding', [1,2,3], 5)` returns correct rows
- Distance-sorted results match HNSW search output
- Optimizer correctly rewrites compatible query patterns

---

## Phase 7: Automatic Index Population on INSERT

### Files to modify
- `kuzu-core/kuzu-storage/src/table.rs` — hook after row insertion
- `kuzu-core/kuzu-processor/src/physical_operator.rs` — `PhysicalCreateDml` or similar
- `kuzu-core/kuzu-main/src/connection.rs` — DML handling

### Steps
1. After each `INSERT` of a node row that has a vector column with an associated index, also insert the vector into the HNSW index.
2. **Approach**: In `Connection::query()` or the processor, after DML execution completes, scan the affected table's vector indexes and update them with any new rows.
3. **Simpler approach**: In `handle_ddl` for `BoundCreateVectorIndex`, auto-populate from existing table data (already covered in Phase 4). For new inserts, the `PhysicalCreateDml` can update the index inline.

### Verification
- INSERT new row with vector → index automatically updated
- Subsequent similarity search includes the new row
- INSERT without vector column → no index interaction

---

## Dependency Graph

```
Phase 1 (Parser) ──→ Phase 2 (Catalog) ──→ Phase 4 (Connection)
                                                    │
Phase 3 (Persistence) ──────────────────────────────┤
                                                    │
Phase 5 (VectorSimilarityScan) ◄────────────────────┘
        │
        └──→ Phase 6a (CALL function) ── parallel with Phase 6b (Optimizer pass)
                                                    │
                                                    └──→ Phase 7 (Auto-populate)
```

**Parallelism**: Phase 1, 2, 3 can be developed in parallel since they are independent. Phase 4 depends on all three. Phase 5 depends on Phase 3. Phase 6a/6b depend on Phase 5. Phase 7 depends on Phase 4+5.

---

## Relevant Files (Full Paths)

| File | Purpose |
|------|---------|
| `kuzu-core/kuzu-parser/src/ast.rs` | Add `CreateVectorIndex` struct + `Statement` variant |
| `kuzu-core/kuzu-parser/src/cypher.pest` | Add `create_vector_index` PEG rule |
| `kuzu-core/kuzu-parser/src/parser.rs` | Add `parse_create_vector_index()` + dispatch from `parse_ddl()` |
| `kuzu-core/kuzu-catalog/src/lib.rs` | Add `VectorIndexEntry`, `CatalogEntry::VectorIndex`, catalog CRUD methods |
| `kuzu-core/kuzu-binder/src/bound_statement.rs` | Add `BoundCreateVectorIndex` struct + `BoundStatement` variant |
| `kuzu-core/kuzu-binder/src/binder.rs` | Add `bind_create_vector_index()` + dispatch in `bind()` |
| `kuzu-core/kuzu-storage/src/vector_index.rs` | **New**: `VectorIndexTable` with BM-backed persistence wrapping `HnswIndex` |
| `kuzu-core/kuzu-storage/src/table.rs` | Add vector index maps to `TableCatalog` |
| `kuzu-core/kuzu-storage/src/lib.rs` | Add `create_vector_index()`, `get_vector_index()` to `StorageManager` |
| `kuzu-core/kuzu-storage/src/index.rs` | Reference pattern (`OnDiskHashIndex` flush/rebuild) |
| `kuzu-core/kuzu-main/src/connection.rs` | Add `BoundCreateVectorIndex` to `handle_ddl()` and `is_write_statement()` |
| `kuzu-core/kuzu-planner/src/logical_operator.rs` | Add `VectorSimilarityScan` variant + `LogicalVectorSimilarityScan` struct |
| `kuzu-core/kuzu-planner/src/planner.rs` | Produce `VectorSimilarityScan` from bound query |
| `kuzu-core/kuzu-processor/src/physical_operator.rs` | Add `PhysicalVectorSimilarityScan` struct + `PhysicalOperatorExec` impl |
| `kuzu-core/kuzu-processor/src/processor.rs` | Dispatch logical→physical for `VectorSimilarityScan` |
| `kuzu-core/kuzu-vector/src/lib.rs` | Register `vector_similarity_scan` table function (Phase 6a) |
| `kuzu-core/kuzu-vector/src/hnsw.rs` | Existing `HnswIndex` — may need small refactors for persistence hooks |
| `kuzu-core/kuzu-optimizer/src/lib.rs` | Add `VectorSimilarityDetection` optimizer pass (Phase 6b) |

---

## Verification (End-to-End)

1. **Parser tests**: All CREATE VECTOR INDEX variants parse correctly
2. **Catalog tests**: Vector index entries created, looked up, listed
3. **Persistence tests**: Round-trip save/load via BufferManager
4. **DDL execution**: `CREATE VECTOR INDEX` via `Connection::query()` returns success
5. **SIMD scan**: `CALL vector_similarity_scan(...)` returns top-K correct results
6. **Optimizer**: Distance+ORDER BY+LIMIT pattern correctly rewritten to use index
7. **Auto-populate**: Existing table data indexed on CREATE; new inserts update index
8. **Edge cases**: Empty index, zero vectors, 1D vectors, high-dimensional vectors

---

## Further Considerations

1. **Circular deps avoidance**: `kuzu-storage` depends on `kuzu-vector`'s `HnswIndex`. Currently `kuzu-vector` depends on `kuzu-common`, `kuzu-function`, `kuzu-catalog`, `kuzu-extension`. Adding `kuzu-storage` dep would create cycle. **Recommendation**: Move `HnswIndex`/`DistanceMetric` into a new `kuzu-hnsw` crate (depends only on `kuzu-common`), then both `kuzu-storage` and `kuzu-vector` depend on it.

2. **Phase 6c (dedicated MATCH syntax)**: Deferred — the `CALL` function (6a) and optimizer rewrite (6b) cover practical use cases without grammar changes. Add later if needed.
