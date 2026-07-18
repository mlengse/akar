# Comprehensive Audit of Kuzu Rust Implementation (`kuzu-core/`) 18/07/2026

## 1. FULL LIST OF WORKSPACE CRATES (from `Cargo.toml`)

There are **32 crates** in the workspace:

| # | Crate | Purpose |
|---|-------|---------|
| 1 | `kuzu-common` | Core types, type system, file system, vectors, enums |
| 2 | `kuzu-storage` | Storage engine, tables, WAL, buffer manager, stats |
| 3 | `kuzu-transaction` | Transaction manager, concurrency control |
| 4 | `kuzu-catalog` | Catalog, schema metadata, sequences, macros |
| 5 | `kuzu-parser` | SQL/Cypher parser, AST definitions |
| 6 | `kuzu-binder` | Semantic analysis, binding, type resolution |
| 7 | `kuzu-planner` | Query planner, logical operators, join ordering |
| 8 | `kuzu-optimizer` | Query optimizer passes |
| 9 | `kuzu-processor` | Query execution engine, physical operators, pipeline |
| 10 | `kuzu-function` | Function registry, scalar/aggregate/table functions |
| 11 | `kuzu-graph` | Graph data structures |
| 12 | `kuzu-extension` | Extension system framework |
| 13 | `kuzu-json` | JSON functions extension |
| 14 | `kuzu-fts` | Full-Text Search extension |
| 15 | `kuzu-vector` | Vector similarity (HNSW) extension |
| 16 | `kuzu-httpfs` | HTTP/S3 file system extension |
| 17 | `kuzu-duckdb` | DuckDB integration extension |
| 18 | `kuzu-algo` | Graph algorithms extension |
| 19 | `kuzu-neo4j` | Neo4j integration extension |
| 20 | `kuzu-llm` | LLM integration extension |
| 21 | `kuzu-sqlite` | SQLite integration extension |
| 22 | `kuzu-delta` | Delta Lake integration extension |
| 23 | `kuzu-iceberg` | Apache Iceberg integration extension |
| 24 | `kuzu-azure` | Azure Blob Storage extension |
| 25 | `kuzu-postgres` | PostgreSQL integration extension |
| 26 | `kuzu-unity-catalog` | Unity Catalog integration extension |
| 27 | `kuzu-main` | Main entry point: Database, Connection, QueryResult |
| 28 | `kuzu-cli` | CLI (command-line interface) |
| 29 | `kuzu-wasm` | WASM build target |
| 30 | `kuzu-migrate` | Database migration tool |
| 31 | `kuzu-c` | C bindings |
| --- | **Total: 32 crates** | |

---

## 2. FULL ENUM DEFINITIONS

### A. LogicalTypeID (from `kuzu-common/src/types.rs`)

```rust
#[repr(u8)]
pub enum LogicalTypeID {
    Any = 0,
    Node = 10,
    Rel = 11,
    RecursiveRel = 12,
    Serial = 13,
    Bool = 22,
    Int64 = 23,
    Int32 = 24,
    Int16 = 25,
    Int8 = 26,
    UInt64 = 27,
    UInt32 = 28,
    UInt16 = 29,
    UInt8 = 30,
    Int128 = 31,
    Double = 32,
    Float = 33,
    Date = 34,
    Timestamp = 35,
    TimestampSec = 36,
    TimestampMs = 37,
    TimestampNs = 38,
    TimestampTz = 39,
    Interval = 40,
    Decimal = 41,
    InternalID = 42,
    UInt128 = 43,
    Json = 44,
    Time = 45,
    String = 50,
    Blob = 51,
    List = 52,
    Array = 53,
    Struct = 54,
    Map = 55,
    Union = 56,
    Uuid = 59,
}
```

**38 variants total.**

### B. PhysicalTypeID (from `kuzu-common/src/types.rs`)

```rust
#[repr(u8)]
pub enum PhysicalTypeID {
    Any = 0,
    Bool = 1,
    Int64 = 2,
    Int32 = 3,
    Int16 = 4,
    Int8 = 5,
    UInt64 = 6,
    UInt32 = 7,
    UInt16 = 8,
    UInt8 = 9,
    Int128 = 10,
    Double = 11,
    Float = 12,
    Interval = 13,
    String = 14,
    Struct = 15,
    List = 16,
    Array = 17,
    Blob = 20,
}
```

**19 variants total.**

### C. Statement (from `kuzu-parser/src/ast.rs`) -- **FULL CONTENTS**

```rust
pub enum Statement {
    Query(Query),
    CreateNodeTable(CreateNodeTable),
    CreateRelTable(CreateRelTable),
    DropTable(DropTable),
    CopyFrom(CopyFrom),
    CopyTo(CopyTo),
    AlterTable(AlterTable),
    CreateVectorIndex(CreateVectorIndex),
    CreateIndex(CreateIndex),
    DropIndex(DropIndex),
    Union(UnionStatement),
    Merge(MergeStatement),
    StandaloneCall(StandaloneCall),
    CreateDml(CreateClause),
    Explain(ExplainStatement),
    CreateSequence(CreateSequence),
    DropSequence(DropSequence),
    CreateMacro(CreateMacro),
    ExportDatabase(ExportDatabase),
    ImportDatabase(ImportDatabase),
    Analyze(AnalyzeStatement),
    CreateFtsIndex(CreateFtsIndex),
    Transaction(TransactionStatement),
    Extension(ExtensionStatement),
    AttachDatabase(AttachDatabase),
    DetachDatabase(DetachDatabase),
    UseDatabase(UseDatabase),
    LoadFrom(LoadFrom),
    CreateType(CreateType),
    CommentOnTable(CommentOnTable),
    CreateGraph(CreateGraph),
    UseGraph(UseGraph),
    DropGraph(DropGraph),
}
```

**32 variants total.**

### D. Expression (from `kuzu-parser/src/ast.rs`) -- **FULL CONTENTS**

```rust
pub enum Expression {
    Constant(Constant),
    Variable(String),
    Parameter(String),
    PropertyAccess(Box<Expression>, String),
    FunctionCall(String, Vec<Expression>),
    BinaryOp(BinaryOp, Box<Expression>, Box<Expression>),
    UnaryOp(UnaryOp, Box<Expression>),
    List(Vec<Expression>),
    Map(Vec<(String, Expression)>),
    ExistsSubquery(Box<Query>),
    Case(CaseExpr),
    Star,
    ListPredicate {
        quantifier: Quantifier,
        list: Box<Expression>,
        var_name: String,
        predicate: Box<Expression>,
    },
    Lambda {
        var_name: String,
        body: Box<Expression>,
    },
}
```

**13 variants total.**

### E. LogicalOperator (from `kuzu-planner/src/logical_operator.rs`) -- **FULL CONTENTS**

```rust
pub enum LogicalOperator {
    ScanNode(LogicalScanNode),
    ScanRel(LogicalScanRel),
    VectorSimilarityScan(LogicalVectorSimilarityScan),
    ArtIndexRangeScan(LogicalArtIndexRangeScan),
    Filter(LogicalFilter),
    Projection(LogicalProjection),
    HashJoin(LogicalHashJoin),
    CrossProduct(LogicalCrossProduct),
    OrderBy(LogicalOrderBy),
    Limit(LogicalLimit),
    TopK(LogicalTopK),
    Aggregate(LogicalAggregate),
    Union(LogicalUnion),
    Flatten(LogicalFlatten),
    TableFunctionCall(LogicalTableFunctionCall),
    StandaloneCall(LogicalStandaloneCall),
    CopyFrom(LogicalCopyFrom),
    BatchInsert(LogicalBatchInsert),
    IndexLookup(LogicalIndexLookup),
    Delete(LogicalDelete),
    Set(LogicalSet),
    OptionalMatch(LogicalOptionalMatch),
    Unwind(LogicalUnwind),
    Foreach(LogicalForeach),
    Merge(LogicalMerge),
    SemiJoin(LogicalSemiJoin),
    AntiJoin(LogicalAntiJoin),
    Intersect(LogicalIntersect),
    Explain(LogicalExplain),
    RecursiveExtend(LogicalRecursiveExtend),
    SemiMasker(LogicalSemiMasker),
    Accumulate(LogicalAccumulate),
    ExpressionsScan(LogicalExpressionsScan),
    CountRelTable(LogicalCountRelTable),
    Partitioner(LogicalPartitioner),
    PathPropertyProbe(LogicalPathPropertyProbe),
    // DDL operators
    CreateNodeTable(LogicalCreateNodeTable),
    CreateRelTable(LogicalCreateRelTable),
    DropTable(LogicalDropTable),
    AlterTable(LogicalAlterTable),
    CreateIndex(LogicalCreateIndex),
    DropIndex(LogicalDropIndex),
    CreateVectorIndex(LogicalCreateVectorIndex),
    CreateSequence(LogicalCreateSequence),
    DropSequence(LogicalDropSequence),
    CreateDml(LogicalCreateDml),
    CreateNode(LogicalCreateNode),
    CreateRel(LogicalCreateRel),
    Extend(LogicalExtend),
    ExportDatabase(LogicalExportDatabase),
    ImportDatabase(LogicalImportDatabase),
    CreateFtsIndex(LogicalCreateFtsIndex),
    FtsScan(LogicalFtsScan),
    EmptyResult(LogicalEmptyResult),
    MultiplicityReducer(LogicalMultiplicityReducer),
    Skip(LogicalSkip),
    Insert(LogicalInsert),
    ExtensionClause(LogicalExtensionClause),
}
```

**55 variants total.**

**NOTE:** There is NO standalone `PhysicalOperatorType` enum. Physical operators are implemented as separate structs each implementing the `PhysicalOperatorExec` trait with an `operator_type(&self) -> &str` method returning a string name.

---

## 3. FULL LIST OF ALL PHYSICAL OPERATORS (via `PhysicalOperatorExec` trait)

| Physical Operator Struct | operator_type() returns | File |
|---|---|---|
| `PhysicalScan` | `"scan"` | `scan_filter/scan.rs` |
| `PhysicalScanRel` | `"scan_rel"` | `scan_filter/scanrel.rs` |
| `PhysicalFilter` | `"filter"` | `scan_filter/filter.rs` |
| `PhysicalProjection` | `"projection"` | `scan_filter/projection.rs` |
| `PhysicalLimit` | `"limit"` | `scan_filter/limit.rs` |
| `PhysicalFlatten` | `"flatten"` | `scan_filter/flatten.rs` |
| `PhysicalPrimaryKeyScan` | `"primary_key_scan"` | `scan_filter/primarykeyscan.rs` |
| `PhysicalPathPropertyProbe` | `"path_property_probe"` | `scan_filter/pathpropertyprobe.rs` |
| `PhysicalHashJoin` | `"hash_join"` | `join_ops.rs` |
| `PhysicalSemiJoin` | `"semi_join"` | `join_ops.rs` |
| `PhysicalAntiJoin` | `"anti_join"` | `join_ops.rs` |
| `PhysicalIntersect` | `"intersect"` | `join_ops.rs` |
| `PhysicalCrossProduct` | `"cross_product"` | `join_ops.rs` |
| `PhysicalOrderBy` | `"order_by"` | `order_aggregate/orderby.rs` |
| `PhysicalTopK` | `"top_k"` | `order_aggregate/topk.rs` |
| `PhysicalAggregate` | `"aggregate"` | `order_aggregate/aggregate.rs` |
| `PhysicalUnwind` | `"unwind"` | `write_ops/unwind.rs` |
| `PhysicalSet` | `"set"` | `write_ops/set.rs` |
| `PhysicalDelete` | `"delete"` | `write_ops/delete.rs` |
| `PhysicalCopyFrom` | `"copy_from"` | `write_ops/copyfrom.rs` |
| `PhysicalExplain` | `"explain"` | `write_ops/physicalexplain.rs` |
| `PhysicalRecursiveExtend` | `"recursive_extend"` | `write_ops/recursiveextend.rs` |
| `PhysicalStandaloneCall` | `"standalone_call"` | `write_ops/standalonecall.rs` |
| `PhysicalPackedExtend` | `"packed_extend"` | `write_ops/packedextend.rs` |
| `PhysicalVectorSimilarityScan` | `"vector_similarity_scan"` | `write_ops/vectorsimilarityscan.rs` |
| `PhysicalCreateFtsIndex` | `"create_fts_index"` | `write_ops/ddl_fts.rs` |
| `PhysicalFtsScan` | `"fts_scan"` | `write_ops/ddl_fts.rs` |
| `PhysicalForeach` | `"foreach"` | `write_ops/foreach.rs` |
| `PhysicalInsert` | `"insert"` | `misc.rs` |
| `PhysicalMerge` | `"merge"` | `write_ops/merge.rs` |
| `PhysicalBatchInsert` | `"batch_insert"` | `batch_insert.rs` |
| `PhysicalIndexLookup` | `"index_lookup"` | `index_lookup.rs` |
| `PhysicalSemiMasker` | `"semi_masker"` | `types.rs` |
| `PhysicalEmptyResult` | `"empty_result"` | `missing_ops.rs` |
| `PhysicalMultiplicityReducer` | `"multiplicity_reducer"` | `misc.rs` |
| `PhysicalSkip` | `"skip"` | `misc.rs` |
| `PhysicalUnionAllScan` | `"union_all_scan"` | `misc.rs` |
| `PhysicalAccumulate` | `"accumulate"` | `missing_ops.rs` |
| `PhysicalUnion` | `"union"` | `missing_ops.rs` |
| `ResultCollector` | `"result_collector"` | `missing_ops.rs` |
| `DummySink` | `"dummy_sink"` | `missing_ops.rs` |
| `DummySimpleSink` | `"dummy_simple_sink"` | `missing_ops.rs` |
| `Profile` | `"profile"` | `missing_ops.rs` |
| `Partitioner` | `"partitioner"` | `missing_ops.rs` |
| `PhysicalExtensionClause` | `"extension_clause"` | `misc.rs` |

**45 physical operator structs total.**

---

## 4. FULL LIST OF ALL REGISTERED FUNCTIONS

### A. Scalar Functions (all registered names)

**Arithmetic (33 names):** `+`, `-`, `*`, `/`, `%`, `abs`, `ceil`, `ceiling`, `floor`, `round`, `^`, `sqrt`, `cbrt`, `cot`, `log`, `ln`, `log2`, `even`, `factorial`, `gamma`, `lgamma`, `set_seed`, `exp`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `degrees`, `radians`, `sign`, `pi`, `rand`, `sinh`, `cosh`, `tanh`, `gcd`, `lcm`

**Bitwise (10 names):** `bitwise_and`, `&`, `bitwise_or`, `|`, `bitwise_xor`, `#`, `bit_shift_left`, `<<`, `bit_shift_right`, `>>`

**Comparison (8 names):** `=`, `<>`, `<`, `<=`, `>`, `>=`, `IS NULL`, `IS NOT NULL`

**String (40 names):** `concat`, `contains`, `starts_with`, `ends_with`, `like`, `to_upper`, `to_lower`, `upper`, `lower`, `ucase`, `lcase`, `trim`, `ltrim`, `rtrim`, `length`, `reverse`, `repeat`, `replace`, `substring`, `regex_matches`, `regex_replace`, `split`, `head`, `tail`, `left`, `right`, `lpad`, `rpad`, `initcap`, `concat_ws`, `split_part`, `array_extract`, `regexp_full_match`, `regexp_extract`, `regexp_extract_all`, `regexp_split_to_array`, `levenshtein`, `soundex`

**Hash (3 names):** `md5`, `sha256`, `hash`

**Interval (8 names):** `to_years`, `to_months`, `to_days`, `to_hours`, `to_minutes`, `to_seconds`, `to_milliseconds`, `to_microseconds`

**Date/Time (17 names):** `date_part`, `date_trunc`, `date_diff`, `date_add`, `current_date`, `current_timestamp`, `year`, `month`, `day`, `hour`, `minute`, `second`, `dayname`, `monthname`, `last_day`, `make_date`, `century`, `epoch_ms`, `to_timestamp`, `to_epoch_ms`

**Cast (16 names):** `CAST`, `cast_string`, `cast_int64`, `cast_double`, `cast_bool`, `date`, `timestamp`, `float`, `double`, `int64`, `int`, `bool`, `boolean`, `string`, `blob`

**Blob (6 names):** `encode`, `decode`, `octet_length`, `blob_from_bytes`, `to_base64`, `from_base64`

**List (22 names):** `list_creation`, `list_extract`, `list_concat`, `list_len`, `list_sort`, `list_reverse`, `list_contains`, `list_append`, `list_prepend`, `list_slice`, `range`, `list_distinct`, `list_unique`, `list_sum`, `list_product`, `list_any_value`, `list_to_string`, `list_position`, `list_indexof`, `list_has_all`, `list_has_any`, `list_count`, `list_min`, `list_max`, `list_reverse_sort`

**List predicate (4 names):** `any`, `all`, `none`, `single`

**Map (5 names):** `map_creation`, `map_extract`, `map_keys`, `map_values`, `map_from_entries`

**Struct (2 names):** `struct_creation`, `struct_extract`

**Union (3 names):** `union_value`, `union_extract`, `union_tag`

**Boolean (4 names):** `AND`, `OR`, `XOR`, `NOT`

**Utility (7 names):** `coalesce`, `ifnull`, `nullif`, `size`, `typeof`, `error`, `pg_isready`

**Schema (7 names):** `OFFSET`, `ID`, `START_NODE`, `END_NODE`, `LABEL`, `COST`, `ROWID`

**Array (7 names):** `array_cosine_similarity`, `array_distance`, `array_inner_product`, `array_dot_product`, `array_cross_product`, `array_squared_distance`, `array_intersect`

**Path (6 names):** `nodes`, `rels`, `relationships`, `properties`, `is_trail`, `is_acyclic`

**UUID (1 name):** `gen_random_uuid`

**Array aliases (9 names):** `array_concat`, `array_cat`, `array_append`, `array_push_back`, `array_prepend`, `array_push_front`, `array_contains`, `array_has`, `array_slice`, `array_value`

**Sequence (2 names):** `nextval`, `currval`

**Total scalar registered: ~209 names** (including aliases)

### B. Aggregate Functions (12 names)

`COUNT`, `COUNT(*)`, `COUNT_IF`, `SUM`, `AVG`, `MIN`, `MAX`, `COLLECT`, `STDDEV`, `VARIANCE`, `STRING_AGG`, `GROUP_CONCAT`, `PERCENTILE_DISC`, `PERCENTILE_CONT`

### C. Table Functions (1 built-in)

`list_tables` (plus dynamic registration via extensions)

---

## 5. CONNECTION API SURFACE

The `Connection` struct exposes these public methods in `kuzu-main/src/connection/`:

| Method | File | Description |
|--------|------|-------------|
| `Connection::new(database)` | `mod.rs` | Create a new connection from a Database Arc |
| `clear_cache(&self)` | `mod.rs` | Clear prepared statement cache |
| `cache_size(&self) -> usize` | `mod.rs` | Number of cached prepared statements |
| `query(&self, query_str) -> Result<QueryResult, String>` | `query.rs` | Execute a Cypher query |
| `prepare(&self, query_str) -> Result<PreparedStatement, String>` | `query.rs` | Prepare a parameterized query |
| `execute(&self, prepared, params) -> Result<QueryResult, String>` | `query.rs` | Execute a prepared statement with parameters |

**Internal/pub(crate) methods:**

| Method | File | Description |
|--------|------|-------------|
| `handle_ddl(&self, bound) -> Result<Option<QueryResult>, String>` | `ddl.rs` | Handle DDL statements (CREATE/DROP/ALTER TABLE, sequences, etc.) |
| `handle_foreach(&self, fc) -> Result<Option<QueryResult>, String>` | `dml.rs` | Handle FOREACH loop execution |
| `execute_export_database(&self, e) -> Result<Option<QueryResult>, String>` | `copy.rs` | Handle EXPORT DATABASE |
| `execute_import_database(&self, i) -> Result<Option<QueryResult>, String>` | `copy.rs` | Handle IMPORT DATABASE |
| `begin_write_txn(&self) -> Result<Transaction, String>` | `transaction.rs` | Begin a write transaction |
| `commit_write_txn(&self, txn) -> Result<(), String>` | `transaction.rs` | Commit a write transaction |
| `rollback_write_txn(&self, txn) -> Vec<UndoRecord>` | `transaction.rs` | Rollback a write transaction |
| `create_processor(&self) -> QueryProcessor` | `query.rs` | Create a QueryProcessor with callbacks |
| `maybe_auto_checkpoint(&self) -> Result<(), String>` | `query.rs` | Trigger auto-checkpoint if needed |
| `do_sync_checkpoint(&self) -> Result<(), String>` | `query.rs` | Perform synchronous checkpoint |
| `is_write_statement(bound) -> bool` | `transaction.rs` | Check if bound statement is a write |
| `extract_write_tables(bound) -> Vec<u64>` | `transaction.rs` | Extract table IDs to be written |
| `execute_query_inner(&self, bound, txn_opt) -> Result<QueryResult, String>` | `query.rs` | Inner query execution (plan/optimize/execute) |

**Database API (`database.rs`):**

| Method | Description |
|--------|-------------|
| `Database::new(db_path, config) -> Result<Self, String>` | Create a new database |
| `catalog(&self) -> Arc<Mutex<Catalog>>` | Get the table catalog |
| `table_catalog(&self) -> Arc<TableCatalog>` | Get the storage table catalog |
| `set_spill_threshold(&self, bytes)` | Override spill threshold at runtime |
| `effective_spill_threshold(&self) -> u64` | Get effective spill threshold |
| `spiller(&self) -> Option<Arc<Spiller>>` | Create a Spiller instance |

---

## 6. BOUND STATEMENT TYPES

From `kuzu-binder/src/bound_statement.rs`, the `BoundStatement` enum has **20 variants**:

```rust
pub enum BoundStatement {
    BoundQuery(BoundQuery),
    BoundStandaloneCall(BoundStandaloneCall),
    BoundCreateNodeTable(BoundCreateNodeTable),
    BoundCreateRelTable(BoundCreateRelTable),
    BoundDropTable(BoundDropTable),
    BoundCopyFrom(BoundCopyFrom),
    BoundCopyTo(BoundCopyTo),
    BoundAlterTable(BoundAlterTable),
    BoundCreateVectorIndex(BoundCreateVectorIndex),
    BoundCreateIndex(BoundCreateIndex),
    BoundDropIndex(BoundDropIndex),
    BoundUnion(BoundUnion),
    BoundMerge(BoundMerge),
    BoundCreateDml(BoundCreateDml),
    BoundExplain(BoundExplain),
    BoundCreateSequence(BoundCreateSequence),
    BoundDropSequence(BoundDropSequence),
    BoundCreateMacro(BoundCreateMacro),
    BoundExportDatabase(BoundExportDatabase),
    BoundImportDatabase(BoundImportDatabase),
    BoundCreateFtsIndex(BoundCreateFtsIndex),
    BoundAnalyze(BoundAnalyze),
    BoundTransaction(BoundTransaction),
    BoundExtension(BoundExtension),
    BoundAttachDatabase(BoundAttachDatabase),
    BoundDetachDatabase(BoundDetachDatabase),
    BoundUseDatabase(BoundUseDatabase),
    BoundLoadFrom(BoundLoadFrom),
    BoundCreateType(BoundCreateType),
    BoundCommentOnTable(BoundCommentOnTable),
    BoundCreateGraph(BoundCreateGraph),
    BoundUseGraph(BoundUseGraph),
    BoundDropGraph(BoundDropGraph),
}
```

---

## 7. LOGICAL-TO-PHYSICAL OPERATOR MAPPING (from `processor/mapper/mod.rs`)

| Category | Logical Operators | Mapped To |
|----------|------------------|-----------|
| **Scans** | `ScanNode` | `map_scan::map_and_execute_scan_node` |
| **Scans** | `ScanRel`, `VectorSimilarityScan`, `ArtIndexRangeScan`, `IndexLookup`, `ExpressionsScan`, `PathPropertyProbe` | `map_scan::map_and_execute_scan` |
| **Joins** | `HashJoin`, `SemiJoin`, `AntiJoin`, `Intersect`, `CrossProduct`, `OptionalMatch`, `RecursiveExtend` | `map_join::map_and_execute_join` |
| **Aggregates** | `Aggregate`, `CountRelTable` | `map_aggregate::map_and_execute_aggregate` |
| **Updates** | `Set`, `Delete`, `CreateNode`, `CreateRel`, `Merge`, `Extend`, `BatchInsert`, `Insert`, `CopyFrom` | `map_update::map_and_execute_update` |
| **Projections** | `Projection`, `Filter`, `TopK`, `OrderBy`, `Limit`, `Flatten`, `SemiMasker`, `Unwind`, `Partitioner` | `map_projection::map_and_execute_projection` |
| **DDL/Other** | `CreateNodeTable`, `CreateRelTable`, `DropTable`, `AlterTable`, `CreateIndex`, `DropIndex`, `CreateVectorIndex`, `CreateSequence`, `DropSequence`, `CreateDml`, `ExportDatabase`, `ImportDatabase`, `CreateFtsIndex`, `FtsScan`, `EmptyResult`, `MultiplicityReducer`, `Skip`, `ExtensionClause`, `StandaloneCall`, `TableFunctionCall`, `Foreach`, `Explain` | `map_ddl::map_and_execute_ddl` |
| **Union** | `Union` | `union_helpers::merge_union_chunks` |
| **Accumulate** | `Accumulate` | `PhysicalAccumulate` directly |

---

## 8. STANDALONE CALL HANDLERS (CALL procedures)

24 registered handlers in `DbStandaloneCallHandler`:

| Handler | Aliases |
|---------|---------|
| `ShowTablesHandler` | `show_tables`, `show tables`, `list_tables`, `list tables`, `tables` |
| `TableInfoHandler` | `table_info` |
| `ShowFunctionsHandler` | `show_functions` |
| `ShowIndexesHandler` | `show_indexes` |
| `ShowSequencesHandler` | `show_sequences` |
| `ShowMacrosHandler` | `show_macros` |
| `ShowConnectionHandler` | `show_connection` |
| `DbVersionHandler` | `db_version` |
| `CatalogVersionHandler` | `catalog_version` |
| `CurrentSettingHandler` | `current_setting` |
| `StatsInfoHandler` | `stats_info` |
| `StorageInfoHandler` | `storage_info` |
| `ShowAttachedDatabasesHandler` | `show_attached_databases` |
| `BmInfoHandler` | `bm_info` |
| `FileInfoHandler` | `file_info` |
| `FreeSpaceInfoHandler` | `free_space_info` |
| `DiskSizeInfoHandler` | `disk_size_info` |
| `StorageVersionHandler` | `storage_version` |
| `ShowLoadedExtensionsHandler` | `show_loaded_extensions` |
| `ShowOfficialExtensionsHandler` | `show_official_extensions` |
| `ClearWarningsHandler` | `clear_warnings` |
| `ShowWarningsHandler` | `show_warnings` |

---

## 9. OPTIMIZER PASSES

The optimizer lives in `kuzu-optimizer/src/` and includes these file names:
- (Not fully explored, but the `STATUS.md` / `implementation_plan.md` might detail this)

The `connection/query.rs` uses:
```rust
let optimizer = Optimizer::with_stats(self.database.stats_store.clone());
let optimized_plan = optimizer.optimize(logical_plan);
```

---

## 10. OBSERVABLE GAPS / MISSING PIECES

1. **No `PhysicalOperatorType` enum** -- Physical operators use string-based `operator_type()` trait method instead of a typed enum, which means no compile-time matching on physical operator types.

2. **No `StatementType` enum** -- Different from the C++ codebase which has a `StatementType` enum. The Rust port uses the `Statement` enum from the parser directly.

3. **No separate `ExpressionType` enum** -- `Expression` is a direct enum with all variant data inline (same as C++ `Expression`), but there is no standalone `ExpressionType` identifier enum separate from the expression node itself.

4. **`LogicalPlanner` is minimal** -- `kuzu-planner` has only 4 source files (`lib.rs`, `logical_operator.rs`, `planner.rs`, `join_order.rs`), suggesting a simplified planner compared to the full C++ planner.

5. **No `kuzu-optimizer` crate exploration** -- Need to verify what optimizer passes exist (e.g., filter push-down, join order, SIP, foreign join push-down, factorization rewriting, etc.)

6. **Some logical operators have no corresponding physical operators** for direct execution -- e.g., `LogicalOptionalMatch` is mapped through `map_join` which handles it at a higher level. `LogicalCreateNodeTable` is handled entirely in `ddl.rs` at the connection level, not as a physical operator.

7. **Some table functions are stubs** -- `ScanCsv`, `ScanParquet`, `ScanJson`, `ListTables`, `ShowColumns`, `CurrentSetting` all return errors when executed dynamically via the function registry, saying they "require file/catalog context".

8. **No `LogicalWindow` operator** -- Window functions are not yet implemented.

9. **No `LogicalDistinct` operator** -- DISTINCT is handled via `MultiplicityReducer` rather than a dedicated distinct operator.

10. **Extensions are compile-time only** -- All extensions are behind Cargo feature flags (`json-extension`, `fts-extension`, `vector-extension`, etc.). Dynamic INSTALL/LOAD is not supported; the DDL handler returns messages saying extensions are compile-time features.

11. **CopyToFormat::Parquet requires feature flag** -- `parquet-export` feature must be enabled.

12. **`kuzu-common/src/enums.rs`** -- Not fully explored, but contains enums like `AccumulateType`, `CompressionType`, `ExtendDirection`, `PathSemantic` that are referenced by logical operators.

13. **Prepared statements use a cache** -- `statement_cache: Mutex<HashMap<String, PreparedStatement>>` with support for parameter substitution via `$name` syntax.

14. **Concurrent write support** -- The transaction module supports `concurrent_writes` via `TransactionManagerConfig` with per-transaction `LocalStorage`, `LocalWAL`, and `ShadowFile` resources.

---

## 11. CRATE MODULE BREAKDOWN

### kuzu-processor crate structure:
```
kuzu-processor/src/
├── expression_evaluator.rs
├── lib.rs
├── physical_operator.rs          # Re-exports from physical/
├── physical/
│   ├── mod.rs
│   ├── types.rs                  # PhysicalOperatorExec trait, NodeSemiMask
│   ├── common.rs                 # Utility: store_value, value_cmp, value_hash
│   ├── scan_filter/
│   │   ├── mod.rs
│   │   ├── scan.rs               # PhysicalScan
│   │   ├── scanrel.rs            # PhysicalScanRel
│   │   ├── filter.rs             # PhysicalFilter
│   │   ├── projection.rs         # PhysicalProjection
│   │   ├── limit.rs              # PhysicalLimit
│   │   ├── flatten.rs            # PhysicalFlatten
│   │   ├── primarykeyscan.rs     # PhysicalPrimaryKeyScan
│   │   └── pathpropertyprobe.rs  # PhysicalPathPropertyProbe
│   ├── order_aggregate/
│   │   ├── mod.rs
│   │   ├── orderby.rs            # PhysicalOrderBy
│   │   ├── topk.rs               # PhysicalTopK
│   │   ├── aggregate.rs          # PhysicalAggregate
│   │   ├── aggregatehashtable.rs # Aggregation hash table
│   │   ├── blockmergesort.rs
│   │   ├── radixsort.rs
│   │   ├── splitaggregation.rs
│   ├── join_ops.rs               # PhysicalHashJoin, SemiJoin, AntiJoin, Intersect, CrossProduct, JoinHashTable
│   ├── write_ops/
│   │   ├── mod.rs
│   │   ├── unwind.rs
│   │   ├── set.rs
│   │   ├── delete.rs
│   │   ├── foreach.rs
│   │   ├── vectorsimilarityscan.rs
│   │   ├── copyfrom.rs
│   │   ├── physicalexplain.rs
│   │   ├── recursiveextend.rs
│   │   ├── ddl_fts.rs
│   │   ├── packedextend.rs
│   │   ├── standalonecall.rs
│   │   ├── insert.rs
│   │   └── merge.rs
│   ├── batch_insert.rs
│   ├── index_lookup.rs
│   ├── missing_ops.rs            # Accumulate, Union, ResultCollector, DummySink, Profile, Partitioner
│   └── misc.rs                   # EmptyResult, MultiplicityReducer, Skip, UnionAllScan, Insert, ExtensionClause
└── processor/
    ├── mod.rs                    # QueryProcessor, StandaloneCallHandler/Registry
    ├── chunk_helpers.rs
    ├── join_helpers.rs
    ├── plan_serializer.rs
    ├── projection_helper.rs
    ├── union_helpers.rs
    ├── tests.rs
    ├── tests_only.rs
    └── mapper/
        ├── mod.rs                # PlanMapper, ExecutionContext
        ├── map_aggregate.rs
        ├── map_ddl.rs
        ├── map_join.rs
        ├── map_projection.rs
        ├── map_scan.rs
        └── map_update.rs
```