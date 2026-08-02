> **SUPERSEDED** - Arsip per 2026-08-02. Dokumen snapshot audit/eksplorasi kondisi 17-19 Juli 2026; codebase sudah berubah signifikan (per 2026-08-02: 32 crate, ~86K LOC, 1,311 test, 25 optimizer passes). Untuk state saat ini lihat `akar-core\SPEC.md`, `akar-core\implementation_plan.md`, dan README per crate.

---

# KUZU C++ Codebase - Comprehensive Architecture Audit 19/07/2026

## Project Overview

- **Project**: Kuzu (v0.12.0)
- **Language**: C++20
- **Build System**: CMake
- **License**: MIT-style (CLA.md)
- **Domain**: Embedded property graph database management system (query language: Cypher subset)

---

## 1. MODULE STRUCTURE BY DIRECTORY

```
src/
├── antlr4/                  # ANTLR-generated Cypher parser
├── main/                    # Database entry point, Connection, ClientContext
├── common/                  # Shared types, utilities, serialization, VFS
├── parser/                  # Parsed AST (Statement, Expression, DDL)
├── binder/                  # Semantic analysis, type checking, catalog lookups
├── planner/                 # Logical plan generation, join ordering
├── optimizer/               # Logical plan optimizations
├── processor/               # Physical plan mapping + execution
│   ├── operator/            # Physical operators (scan, join, aggregate, etc.)
│   │   ├── persistent/      # COPY TO/FROM, CSV/Parquet/JSON readers/writers
│   │   ├── aggregate/       # Hash aggregate operators
│   │   ├── hash_join/       # Hash join operators
│   │   ├── order_by/        # Sort operators
│   │   ├── intersect/       # Intersect operator
│   │   ├── scan/            # Node/rel table scan operators
│   │   ├── table_scan/      # Generic table scan
│   │   ├── ddl/             # CREATE/DROP/ALTER table operators
│   │   ├── simple/          # Simple statement operators
│   │   └── macro/           # Macro operators
│   ├── result/              # FactorizedTable, ResultSet, FlatTuple
│   └── map/                 # PlanMapper (LogicalOperator -> PhysicalOperator)
├── storage/                 # Storage engine
│   ├── table/               # NodeTable, RelTable, Column, NodeGroup, CSR structures
│   ├── index/               # HashIndex (on-disk), InMemHashIndex
│   ├── wal/                 # Write-Ahead Log
│   ├── buffer_manager/      # BufferManager, MemoryManager, VMRegion, Spiller
│   ├── compression/         # Compression algorithms (bitpacking, ALP, constant)
│   ├── local_storage/       # LocalTable, LocalNodeTable, LocalRelTable
│   ├── stats/               # ColumnStats, TableStats, HyperLogLog
│   ├── predicate/           # ColumnPredicate, ConstantPredicate, NullPredicate
│   └── ...                  # StorageManager, Checkpointer, ShadowFile, DiskArray
├── catalog/                 # Catalog, CatalogSet, CatalogEntry hierarchy
├── transaction/             # Transaction, TransactionManager, TransactionContext
├── function/                # Built-in functions (arithmetic, string, date, etc.)
├── graph/                   # Graph abstraction for GDS algorithms
├── expression_evaluator/    # Expression evaluation engine
├── extension/               # Extension loading/management system
├── c_api/                   # C API for language bindings
└── binder/                  # (already covered above)
```

---

## 2. MAJOR CLASSES, HIERARCHIES & KEY METHODS

### 2.1 Entry Point / Main Module (`src/include/main/`)

| Class | Description | Key Methods |
|-------|-------------|-------------|
| `Database` | Central database object | `ctor(databasePath, SystemConfig)`, `getCatalog()`, `getStorageManager()`, `getTransactionManager()`, `getQueryProcessor()`, `getMemoryManager()`, `registerFileSystem()`, `registerStorageExtension()`, `addTransformerExtension/BinderExtension/PlannerExtension/MapperExtension()` |
| `Connection` | Per-client connection | `query(string_view)`, `prepare(string_view)`, `execute(PreparedStatement*)`, `interrupt()`, `setQueryTimeOut()`, `createScalarFunction()`, `createVectorizedFunction()` |
| `ClientContext` | Per-connection state | `query()`, `prepareWithParams()`, `executeNoLock()`, `getDatabasePath()`, `setMaxNumThreadForExec()`, `addScanReplace()`, transaction mgmt helpers |
| `SystemConfig` | Config struct | `bufferPoolSize`, `maxNumThreads`, `enableCompression`, `readOnly`, `maxDBSize`, `autoCheckpoint`, `checkpointThreshold`, `forceCheckpointOnClose`, `enableChecksums` |
| `PreparedStatement` | Prepared query | Holds parsed + bound + planned state |
| `QueryResult` | Query result | Iterator over factorized table or Arrow |
| `DatabaseManager` | Multi-database support | Manages attached databases |
| `StorageDriver` | Direct storage access API | |

### 2.2 Parser Module (`src/include/parser/`)

| Class | Description |
|-------|-------------|
| `Parser` | Static `parseQuery()` using ANTLR-generated lexer/parser; returns `vector<shared_ptr<Statement>>` |
| `Statement` (base) | All parsed statements; subtypes: `RegularQuery`, `CreateTable`, `CreateType`, `CreateSequence`, `CreateMacro`, `CopyTo`, `CopyFrom`, `Drop`, `Alter`, `Explain`, `StandaloneCall`, `TransactionStatement`, `ExtensionStatement`, `AttachDatabase`, `DetachDatabase`, `UseDatabase` |
| `ParsedExpression` (base) | Expression tree: `ParsedVariableExpression`, `ParsedPropertyExpression`, `ParsedFunctionExpression`, `ParsedLiteralExpression`, `ParsedParameterExpression`, `ParsedSubqueryExpression`, `ParsedCaseExpression`, etc. |
| `ReadingClause` | `MatchClause`, `UnwindClause`, `InQueryCall`, `LoadFrom` |
| `UpdatingClause` | `InsertClause`, `MergeClause`, `SetClause`, `DeleteClause` |
| `PatternElement` | Graph pattern: `NodePattern`, `RelPattern` |
| Visitor | `ParsedStatementVisitor` for tree walking |

### 2.3 Binder Module (`src/include/binder/`)

| Class | Description | Key Methods |
|-------|-------------|-------------|
| `Binder` | Semantic analysis | `bind(const Statement&) -> BoundStatement`, `bindCreateTable()`, `bindCopyFrom()`, `bindQuery()`, `bindReadingClause/Match/Unwind/InQueryCall/LoadFrom()`, `bindUpdatingClause/Insert/Merge/Set/Delete()`, `bindGraphPattern()`, `bindProjectionBody()`, `bindWhereExpression()` |
| `ExpressionBinder` | Expression binding | Binds `ParsedExpression` -> `Expression` with type resolution |
| `BoundStatement` (base) | Bound statements | `BoundRegularQuery`, `BoundCreateTable`, `BoundCopyFrom`, `BoundCopyTo`, `BoundDrop`, `BoundAlter`, `BoundExplain`, `BoundStandaloneCall`, `BoundTransaction`, `BoundExtension`, `BoundCreateMacro`, `BoundCreateType`, `BoundCreateSequence`, `BoundAttachDatabase`, `BoundDetachDatabase`, `BoundUseDatabase` |
| `Expression` (base) | Bound expressions | `PropertyExpression`, `NodeExpression`, `RelExpression`, `FunctionExpression`, `LiteralExpression`, `ParameterExpression`, `SubqueryExpression`, `CaseExpression`, `VariableExpression` |
| `BoundReadingClause` | `BoundMatchClause`, `BoundUnwindClause`, `BoundInQueryCall`, `BoundLoadFrom` |
| `BoundUpdatingClause` | `BoundInsertClause`, `BoundMergeClause`, `BoundSetClause`, `BoundDeleteClause` |
| `PropertyDefinition` | Column metadata | name, type, default expression |

### 2.4 Planner Module (`src/include/planner/`)

| Class | Description | Key Methods |
|-------|-------------|-------------|
| `Planner` | Logical plan generation | `planStatement(const BoundStatement&)`, `planQuery()`, `planSingleQuery()`, `planReadingClause()`, `planUpdatingClause()`, `planProjectionBody()`, `planAggregate()`, `planOrderBy()`, `planSubquery()`, `planRegularMatch()`, `planOptionalMatch()`, join ordering: `planLevel()`, `planWCOJoin()`, `tryPlanINLJoin()`, `planInnerHashJoin()` |
| `LogicalOperator` (base) | Logical plan node | Subtypes: `LogicalScanNodeTable`, `LogicalExtend`, `LogicalFilter`, `LogicalProjection`, `LogicalHashJoin`, `LogicalAccHashJoin`, `LogicalAggregate`, `LogicalOrderBy`, `LogicalLimit`, `LogicalDistinct`, `LogicalCrossProduct`, `LogicalIntersect`, `LogicalAccumulate`, `LogicalFlatten`, `LogicalMultiplicityReducer`, `LogicalUnwind`, `LogicalTableFunctionCall`, `LogicalCopyFrom`, `LogicalCopyTo`, `LogicalInsert`, `LogicalMerge`, `LogicalSetProperty`, `LogicalDelete`, `LogicalPartitioner`, `LogicalSemiMasker`, `LogicalNodeLabelFilter`, `LogicalPathPropertyProbe`, `LogicalRecursiveExtend`, `LogicalDummyScan`, `LogicalEmptyResult`, `LogicalExplain`, etc. |
| `LogicalPlan` | Container plan | Root operator, schema, cardinality |
| `Schema` | Flat/cardinality info | Groups of expressions, factorized schema |
| `JoinOrderEnumeratorContext` | DP join ordering | DP table with `subPlansTable` |
| `CardinalityEstimator` | Stats-based cardinality estimation | |
| `PropertyExprCollection` | Tracks required properties | |

### 2.5 Optimizer Module (`src/include/optimizer/`)

| Class | Description |
|-------|-------------|
| `Optimizer` | Static `optimize(LogicalPlan*)` - runs all below passes in sequence |
| `FilterPushDownOptimizer` | Pushes filters closer to scan nodes |
| `ProjectionPushDownOptimizer` | Removes unnecessary projections |
| `CorrelatedSubqueryUnnestSolver` | Unnests correlated subqueries |
| `FactorizationRewriter` | Optimizes factorized execution |
| `RemoveFactorizationRewriter` | Reverts factorization when not beneficial |
| `AggregateKeyDependencyOptimizer` | Detects functional dependencies in GROUP BY |
| `LimitPushDownOptimizer` | Pushes limits down |
| `TopKOptimizer` | Converts ORDER BY + LIMIT to TopK |
| `RemoveUnnecessaryJoinOptimizer` | Eliminates unnecessary joins |
| `AccHashJoinOptimizer` | Converts hash joins to accumulative hash joins |
| `CardinalityUpdater` | Recomputes cardinality after optimization |
| `SchemaPopulator` | Populates schema info |
| `LogicalOperatorCollector` | Collects operators by type |
| `LogicalOperatorVisitor` | Visitor pattern for plan traversal |

### 2.6 Processor Module (`src/include/processor/`)

| Class | Description | Key Methods |
|-------|-------------|-------------|
| `QueryProcessor` | Executes physical plan | `execute(PhysicalPlan*, ExecutionContext*)`, `decomposePlanIntoTask()` |
| `PlanMapper` | Logical -> Physical mapping | `getPhysicalPlan(LogicalPlan*)`, `mapOperator()`, 50+ `mapXxx()` methods (one per operator type) |
| `PhysicalOperator` (base) | Executable node | `initLocalState()`, `getNextTuple()`, `getData()`, `getSelVector()` |
| `PhysicalPlan` | Root physical operator | |
| `ExecutionContext` | Per-query context | `queryID`, `profiler`, `clientContext` |
| `WarningContext` | Accumulates warnings | |

Key Physical Operators (50+):
- `ScanNodeTable`, `ScanRelTable`, `IndexLookup`
- `Extend`, `RecursiveExtend`, `PathPropertyProbe`
- `Filter`, `Projection`, `Flatten`, `MultiplicityReducer`
- `HashJoin`, `Intersect`, `CrossProduct`
- `Aggregate` (hash-based), `OrderBy`, `Limit`, `Distinct`
- `Insert`, `Merge`, `SetProperty`, `Delete`, `DetachDelete`
- `CopyFrom`, `CopyTo`, `CopyNodeFrom`, `CopyRelFrom`
- `CreateTable`, `CreateType`, `CreateSequence`, `CreateMacro`
- `Drop`, `Alter`, `StandaloneCall`, `Transaction`, `Explain`
- `Partitioner`, `SemiMasker`, `NodeLabelFilter`
- `Unwind`, `DummyScan`, `EmptyResult`, `UnionAll`, `TableFunctionCall`

### 2.7 Result Module (`src/include/processor/result/`)

| Class | Description |
|-------|-------------|
| `FactorizedTable` | Columnar data store with optional flat/unflat columns |
| `FactorizedTableSchema` | Schema info for `FactorizedTable` |
| `FactorizedTablePool` | Pool for reusing factorized tables |
| `ResultSet` | Set of `ValueVector`s, chunked for processing |
| `ResultSetDescriptor` | Describes schema of result set |
| `BaseHashTable` | Hash table for aggregates/joins |
| `FlatTuple` | Row-oriented result iterator |

### 2.8 Storage Module (`src/include/storage/`)

**Storage Engine Core:**
| Class | Description | Key Methods |
|-------|-------------|-------------|
| `StorageManager` | Manages all tables, shadow file, WAL | `createTable()`, `checkpoint()`, `recover()`, `serialize()`, `deserialize()`, `getTable()`, `captureChangeEpochs()` |
| `Table` (base) | Abstract table | `initScanState()`, `scan()`, `insert()`, `update()`, `delete_()`, `addColumn()`, `commit()`, `checkpoint()`, `serialize()` |
| `NodeTable` : `Table` | Node table | `lookupPK()`, `validateUniquenessConstraint()`, `addIndex()`, `dropIndex()`, `getPKIndex()` |
| `RelTable` : `Table` | Relationship table | `detachDelete()`, `checkIfNodeHasRels()`, `reserveRelOffsets()`, multiple directions (FWD/BWD) |
| `RelTableData` | Per-direction rel storage | CSR header columns (offset + length), property columns, `getCSROffsetColumn()`, `getCSRLengthColumn()` |

**Column Storage Hierarchy:**
| Class | Description |
|-------|-------------|
| `Column` (base) | Persistent column storage on disk |
| `InternalIDColumn` | Special column for node/rel IDs |
| `StringColumn` | String storage with dictionary + overflow |
| `StructColumn` | Struct with nested sub-columns |
| `ListColumn` | List with child column + offset |
| `NullColumn` | Null bitmap storage |
| `DictionaryColumn` | Dictionary for string compression |
| `ColumnFactory` | Creates appropriate column type from `LogicalType` |

**Column Chunk Hierarchy:**
| Class | Description |
|-------|-------------|
| `ColumnChunkData` (base) | In-memory column data segment |
| `ColumnChunk` | Wrapper with update info, segment management |
| `StringChunkData` | String chunk with overflow |
| `ListChunkData` | List chunk |
| `StructChunkData` | Struct chunk |
| `DictionaryChunk` | Dictionary-compressed string chunk |
| `InternalIDChunkData` | Internal ID chunk |
| `ColumnChunkMetadata` | Page-level metadata for each chunk |
| `ColumnChunkStats` | Min/max statistics |

**Node Group System:**
| Class | Description |
|-------|-------------|
| `NodeGroup` | Collection of rows divided into chunked groups |
| `NodeGroupCollection` | Ordered collection of `NodeGroup`s |
| `ChunkedNodeGroup` | Sub-group of rows within a `NodeGroup` |
| `InMemChunkedNodeGroupCollection` | In-memory collection for bulk loading |
| `CSRNodeGroup` | CSR-format node group for rel tables |
| `CSRChunkedNodeGroup` | Chunked node group in CSR format |
| `GroupCollection<T>` | Template for thread-safe group management |

**Index System:**
| Class | Description |
|-------|-------------|
| `Index` (base) | Abstract index: `insert()`, `delete_()`, `lookup()`, `update()` |
| `PrimaryKeyIndex` : `Index` | Composite hash index for PK |
| `HashIndex<T>` | Templated on-disk hash index with linear hashing |
| `InMemHashIndex<T>` | In-memory hash index for building |
| `OnDiskHashIndex` | Virtual interface for checkpointable indexes |
| `HashIndexLocalStorage<T>` | Transaction-local index modifications |
| `HashIndexHeader` | Level/split state for linear hashing |
| `Slot<T>` | Slot with entries + fingerprints |
| `IndexHolder` | Lazy-loaded index wrapper |
| `IndexBuffer<T>` | Batch insert buffer (1024 elements) |
| `IndexInfo` | Metadata about an index |
| `IndexType` | Typed index descriptor (HASH, extensible) |

**Buffer Manager:**
| Class | Description |
|-------|-------------|
| `BufferManager` | Page-based buffer pool with virtual memory |
| `MemoryManager` | Manages memory allocations via BM |
| `VMRegion` | Virtual memory region (regular page or temp page) |
| `Spiller` | Spills to disk when out of memory |
| `EvictionQueue` | Clock-sweep eviction queue |
| `EvictionCandidate` | Candidate page for eviction |
| `PageState` | State machine: EVICTED -> LOCKED -> UNLOCKED -> MARKED |
| `MMAllocator` | Memory allocator backed by BM |

**WAL (Write-Ahead Log):**
| Class | Description |
|-------|-------------|
| `WAL` | Shared WAL file |
| `LocalWAL` | Transaction-local WAL buffer |
| `WALRecord` (base) | Base class for all WAL record types |
| Record subtypes: | `BeginTransactionRecord`, `CommitRecord`, `CheckpointRecord`, `CreateCatalogEntryRecord`, `DropCatalogEntryRecord`, `AlterTableEntryRecord`, `CopyTableRecord`, `TableInsertionRecord`, `NodeDeletionRecord`, `NodeUpdateRecord`, `RelDeletionRecord`, `RelDetachDeleteRecord`, `RelUpdateRecord`, `UpdateSequenceRecord`, `LoadExtensionRecord` |
| `WALReplayer` | Replays WAL during recovery |
| `ChecksumWriter` / `ChecksumReader` | CRC32 checksums for WAL pages |

**Checkpoint System:**
| Class | Description |
|-------|-------------|
| `Checkpointer` | Background + foreground checkpoint management |
| `ShadowFile` | Shadow page mechanism for failure atomicity |
| `PageAllocator` | Allocates pages during checkpoint |

**Persistence Data Structures:**
| Class | Description |
|-------|-------------|
| `DiskArray<T>` | Templated disk-based array with page indirection (PIPs) |
| `DiskArrayInternal` | Internal implementation of `DiskArray` |
| `DiskArrayCollection` | Multiple `DiskArray`s sharing a file |
| `BlockVector<T>` | In-memory block vector |
| `OverflowFile` | Overflow storage for large string values |
| `FreeSpaceManager` | Tracks free pages in data files |
| `FileHandle` | Per-file handle with page state tracking |
| `VersionRecordHandler` | Manages version info during commit/rollback |

**Compression System:**
| Class | Description |
|-------|-------------|
| `CompressionAlg` (base) | Abstract compression algorithm |
| `Uncompressed` | Passthrough (no compression) |
| `ConstantCompression` | When all values are identical |
| `IntegerBitpacking<T>` | Frame-of-reference bitpacking via fastpfor |
| `BooleanBitpacking` | Bit-level packing for booleans |
| `ALP` compression | Adaptive Lossless floating-Point compression (via `alp/state.hpp`) |
| `CompressionMetadata` | Per-chunk metadata: min, max, compression type |
| `ALPMetadata` | Exception count/capacity for ALP |

**Storage Statistics:**
| Class | Description |
|-------|-------------|
| `ColumnStats` | Per-column min/max statistics |
| `TableStats` | Collection of column stats for a table |
| `HyperLogLog` | Cardinality estimation via HyperLogLog |

### 2.9 Catalog Module (`src/include/catalog/`)

| Class | Description | Key Methods |
|-------|-------------|-------------|
| `Catalog` | Central catalog | `getTableCatalogEntry()`, `createTableEntry()`, `dropTableEntry()`, `alterTableEntry()`, `getSequenceEntry()`, `createSequence()`, `containsFunction()`, `addFunction()`, `createIndex()`, `dropIndex()`, `serialize()`, `deserialize()` |
| `CatalogSet` | Versioned map of entries | `createEntry()`, `dropEntry()`, `getEntry()`, `alterTableEntry()`, version chain traversal |
| `CatalogEntry` (base) | Entry with versioning | `prev`/`next` pointers for MVCC version chain |
| `TableCatalogEntry` | Table metadata | Properties, column IDs, scan function |
| `NodeTableCatalogEntry` : `TableCatalogEntry` | Node table metadata |
| `RelGroupCatalogEntry` : `TableCatalogEntry` | Rel group metadata (multi-directional) |
| `FunctionCatalogEntry` | Function metadata |
| `SequenceCatalogEntry` | Sequence auto-increment metadata |
| `IndexCatalogEntry` | Index metadata |
| `ScalarMacroCatalogEntry` | Macro definition |
| `TypeCatalogEntry` | User-defined type metadata |
| `PropertyDefinitionCollection` | Ordered property definitions |
| `CatalogEntryType` enum | `NODE_TABLE_ENTRY`, `REL_GROUP_ENTRY`, `SEQUENCE_ENTRY`, `FUNCTION_ENTRY`, `TYPE_ENTRY`, `INDEX_ENTRY`, `MACRO_ENTRY`, `DUMMY_ENTRY` |

### 2.10 Transaction Module (`src/include/transaction/`)

| Class | Description | Key Methods |
|-------|-------------|-------------|
| `Transaction` | Per-transaction state | `commit()`, `rollback()`, `getLocalStorage()`, `pushInsertInfo()`, `pushDeleteInfo()`, `pushVectorUpdateInfo()`, `pushCreateDropCatalogEntry()` |
| `TransactionManager` | Global TX manager | `beginTransaction()`, `commit()`, `rollback()`, `checkpoint()`, `shutdownAutoCheckpointWorker()` |
| `TransactionContext` | Per-connection TX mode | AUTO/MANUAL, `beginReadTransaction()`, `beginWriteTransaction()`, `commit()`, `rollback()` |
| `TransactionMode` | `AUTO` or `MANUAL` |
| `TransactionType` | `READ_ONLY`, `WRITE`, `CHECKPOINT`, `DUMMY`, `RECOVERY` |
| `LocalCacheManager` | Per-transaction cache |

### 2.11 Function Module (`src/include/function/`)

| Component | Details |
|-----------|---------|
| `function_set` | Collection of function overloads |
| `scalar_func_exec_t` | Scalar function execution signature |
| `ScalarFunction` | Scalar function with exec + select |
| `AggregateFunction` | Aggregate with init/update/combine/finalize |
| `BuiltInFunctionsUtils` | Registration of built-in functions |
| Expression type categories | `arithmetic/`, `string/`, `date/`, `timestamp/`, `interval/`, `list/`, `array/`, `struct/`, `map/`, `union/`, `cast/`, `boolean/`, `null/`, `blob/`, `uuid/`, `internal_id/`, `hash/`, `path/`, `pattern/`, `sequence/`, `schema/`, `utility/`, `export/`, `gds/`, `table/`, `aggregate/`, `comparison/`, `pointer/`, `udf/`, `rewrite/` |

### 2.12 Expression Evaluator (`src/include/expression_evaluator/`)

| Class | Description |
|-------|-------------|
| `ExpressionEvaluator` (base) | Abstract expression evaluation |
| `FunctionEvaluator` | Evaluates function expressions |
| `LiteralEvaluator` | Returns constant values |
| `ReferenceEvaluator` | Reads from input vectors |
| `CaseEvaluator` | CASE/WHEN evaluation |
| `LambdaEvaluator` | Lambda function evaluation |
| `PatternEvaluator` | Graph pattern evaluation |
| `PathEvaluator` | Path expression evaluation |

### 2.13 Graph Module (`src/include/graph/`)

| Class | Description |
|-------|-------------|
| `Graph` (base) | Abstract graph for GDS: `scanFwd()`, `scanBwd()`, `scanVertices()`, `prepareVertexScan()`, `prepareRelScan()` |
| `OnDiskGraph` | Concrete graph over storage engine |
| `GraphEntry` / `GraphEntrySet` | Graph selection metadata |
| `NbrScanState` / `VertexScanState` | Scan state with chunk iteration |
| `GraphRelInfo` | Relationship info for graph traversal |

### 2.14 Extension Module (`src/include/extension/`)

| Class | Description |
|-------|-------------|
| `ExtensionManager` | Manages loading/unloading extensions |
| `ExtensionLoader` | Dynamic library loading |
| `ExtensionInstaller` | Extension download/install |
| `Extension` (base) | Extension interface |
| `TransformerExtension` | Pre-parsing transform |
| `BinderExtension` | Extends binder |
| `PlannerExtension` | Extends planner |
| `MapperExtension` | Extends plan mapper |
| `CatalogExtension` | Extends catalog |

### 2.15 File System (`src/include/common/file_system/`)

| Class | Description |
|-------|-------------|
| `VirtualFileSystem` (VFS) | Abstract VFS with `openFile()`, `readFile()`, etc. |
| `LocalFileSystem` | OS file system implementation |
| `S3FileSystem` | S3-compatible object storage |
| `HTTPFileSystem` | HTTP/HTTPS file access |

---

## 3. QUERY PROCESSING PIPELINE

```
SQL String
    |
    v
[Parser] (ANTLR4 generated)
    |  lexer -> parser -> AST (vector<shared_ptr<Statement>>)
    |  Transformer extensions can modify before main parse
    v
[Binder]
    |  Catalog lookups, type checking, graph pattern resolution
    |  Creates BoundStatement tree with resolved types
    |  Binder extensions can extend
    v
[Planner]
    |  Converts BoundStatement -> LogicalPlan
    |  DP-based join order enumeration
    |  Worst-case optimal join planning
    |  SIP (Sideways Information Passing) planning
    |  Plugs in planner extensions
    v
[Optimizer]
    |  Runs passes sequentially:
    |  1. Filter Push Down
    |  2. Projection Push Down
    |  3. Correlated Subquery Unnesting
    |  4. Factorization Rewriting
    |  5. Aggregate Key Dependency
    |  6. Limit Push Down / Top-K
    |  7. Remove Unnecessary Joins
    |  8. Accumulative Hash Join Optimization
    v
[PlanMapper]
    |  Maps LogicalOperator -> PhysicalOperator
    |  Creates PhysicalPlan as pull-based iterator tree
    v
[QueryProcessor]
    |  Decomposes plan into parallel Tasks
    |  TaskScheduler distributes across threads
    |  Each operator implements getNextTuple() (pull model)
    v
[Result Collection]
    |  FactorizedTable (columnar) or Arrow result format
    v
[QueryResult]
    |  Iterates result sets to client
```

---

## 4. STORAGE ENGINE DETAILS

### 4.1 Data Organization

```
Database Directory/
├── data.kz              # Main data file
├── wal                  # Write-Ahead Log
├── wal.checkpoint       # Frozen WAL for checkpointing
└── shadow               # Shadow file for atomic checkpoint

Data File Layout:
┌─────────────────────────┐
│ DB Header (Page 0)      │  <- databaseID, snapshot timestamps
├─────────────────────────┤
│ DiskArray Headers       │  <- pre-allocated stable pages for each DiskArray
├─────────────────────────┤
│ PIPs (Page Index Pages) │  <- track which pages belong to what
├─────────────────────────┤
│ Array Pages             │  <- actual Slot/Column data
│   ┌─────────────────┐   │
│   │ Page Header      │   │  <- metadata, compression info
│   │ Compressed Data  │   │  <- ALP/bitpacking/uncompressed
│   └─────────────────┘   │
├─────────────────────────┤
│ Overflow Pages          │  <- strings, large values
└─────────────────────────┘
```

### 4.2 Table Storage Structures

**Node Tables:**
- `NodeTable` owns:
  - `vector<Column>` - one per property
  - `NodeGroupCollection` - rows divided into node groups (~128K rows each)
  - `PrimaryKeyIndex` - hash index on PK column
  - `vector<IndexHolder>` - secondary indexes
- Each `NodeGroup` contains:
  - `GroupCollection<ChunkedNodeGroup>` - for MVCC, new versions
  - `VersionInfo` - tracks inserted/deleted/updated rows

**Rel Tables:**
- `RelTable` owns per-direction `RelTableData` objects
- `RelTableData` owns:
  - `CSRHeaderColumns` (offset + length columns) - CSR adjacency list
  - `vector<Column>` - property columns
  - `NodeGroupCollection` - CSR node groups
- CSR format: For each source node, stores offset into adjacency array + degree

**CSR Format for Relationships:**
```
For each source node group:
  CSROffsetColumn: [start_offset_for_node0, start_offset_for_node1, ...]
  CSRLengthColumn: [degree_of_node0, degree_of_node1, ...]
  NbrIDColumn:     [nbr0_of_node0, nbr1_of_node0, ..., nbr0_of_node1, ...]
  PropertyCols:    [prop0_of_rel0, prop0_of_rel1, ...]
```

### 4.3 Index Structures

**Primary Key Index (Hash Index):**
- On-disk linear hash table with split policy
- `NUM_HASH_INDEXES` = 256 sub-indices for parallelism
- Slot size: 256 bytes, fixed capacity per type
- Uses `DiskArray<Slot<T>>` for persistent storage
- Fingerprint (1 byte) per entry for fast negative checks
- Chained overflow slots for collisions
- Local storage per transaction for uncommitted changes
- In-memory builder (`InMemHashIndex`) for initial bulk load
- Split policy: As slots fill, new slots are added at level+1

### 4.4 Compression

- **Integer Bitpacking**: Frame-of-reference with offset, 32-value chunks via fastpfor
- **Boolean Bitpacking**: 1 bit per boolean
- **ALP (Adaptive Lossless Float Compression)**: For float/double; encodes with exponent + factor, stores exceptions
- **Constant**: When all values in a page are identical
- **Uncompressed**: Raw copy
- Compression is page-level, configured per column as part of `CompressionMetadata`

### 4.5 Page & Segment Architecture

- `KUZU_PAGE_SIZE` = 4096 bytes (configurable via `KUZU_PAGE_SIZE_LOG2` = 12)
- Segments divide pages; a segment is a contiguous set of pages
- `SegmentState` + `ChunkState` track position within column data
- `ColumnChunk` manages in-memory segments (vector of `ColumnChunkData`)
- `ResidencyState`: `IN_MEMORY` or `ON_DISK`

---

## 5. CATALOG/SCHEMA SYSTEM

- **`Catalog`** owns six `CatalogSet`s: `tables`, `sequences`, `functions`, `types`, `indexes`, `macros` + their `internal` counterparts
- **`CatalogSet`** implements MVCC via version chains: each entry has `prev`/`next` pointers forming a singly-linked list (prev = older version, next = newer version)
- **`CatalogEntry`** has `timestamp` (transaction commit timestamp), `deleted` flag, OID
- Entries are looked up per transaction: the version visible at the transaction's start timestamp is found by traversing the version chain
- DD operations (CREATE/DROP/ALTER) push undo/redo info to the transaction's undo buffer and WAL
- Catalog serializes/deserializes as part of checkpoint

---

## 6. TRANSACTION SYSTEM

- **MVCC** with optimistic concurrency control
- Transaction types: READ_ONLY, WRITE, CHECKPOINT, DUMMY, RECOVERY
- **TransactionManager**: serializes begin/commit/rollback, handles checkpoint coordination, runs auto-checkpoint worker
- **TransactionContext**: per-connection AUTO/MANUAL transaction mode
- **LocalStorage**: Per-transaction buffer for uncommitted inserts/updates/deletes
- **UndoBuffer**: Rollback information for abort
- **LocalWAL**: Transaction-local WAL entries that are flushed to shared WAL on commit
- **VersionInfo**: Tracks inserted/deleted row ranges per node group
- **VersionRecordHandler**: Applies version info on commit (per NodeTable/RelTable)
- **Commit protocol**: flush LocalWAL -> append COMMIT record to shared WAL -> checkpoint if threshold exceeded
- **Checkpoint**: drains active transactions, takes snapshot, writes shadow pages, atomically updates DB header
- **Recovery**: on startup, replays WAL from last checkpoint, applies shadow pages

---

## 7. THIRD-PARTY DEPENDENCIES

| Dependency | Usage |
|-----------|-------|
| **ANTLR4** (Cypher grammar) | Parser/lexer generation for Cypher language |
| **ANTLR4 Runtime** | ANTLR runtime library |
| **fast_float** | Fast float parsing |
| **fastpfor** | Integer bitpacking compression |
| **alp** | Adaptive Lossless Float compression |
| **roaring_bitmap** | Roaring bitmaps for semi masks |
| **zstd** | Zstandard compression |
| **lz4** | LZ4 compression |
| **snappy** | Snappy compression |
| **miniz** | Minimal gzip/zlib |
| **brotli** | Brotli compression |
| **mbedtls** | TLS/crypto for HTTPS/checksums |
| **re2** | Regular expression matching |
| **yyjson** | Fast JSON parsing |
| **nlohmann_json** | JSON library (export/import) |
| **utf8proc** | Unicode processing |
| **spdlog** | Logging (test/benchmark only) |
| **pcg** | Random number generation |
| **simsimd** | SIMD distance functions (for GDS?) |
| **pcg** | Random number generator |
| **taywee_args** | Command-line argument parsing |
| **pybind11** | Python bindings |
| **httplib** | HTTP client library |
| **thrift** | Apache Thrift (Parquet metadata) |
| **parquet** | Parquet file format support |
| **cppjieba** | Chinese text segmentation |
| **pyparse** | Python parsing |
| **glob** | File glob pattern matching |
| **cpptrace** (optional) | Backtrace on crash |

---

## 8. SERIALIZATION/DESERIALIZATION FORMAT

- Homegrown binary serialization via `Serializer`/`Deserializer` classes
- `Writer`/`Reader` abstraction: `BufferedFileWriter`, `BufferWriter`, `InMemFileWriter`
- Simple protocol: size-prefixed vectors, raw struct serialization for trivially-copyable types, custom `serialize()` methods for complex types
- Used for: catalog persistence, WAL records, column data (checkpoint), `COPY TO`, `EXPORT DATABASE`
- `BufferReader`/`BufferWriter` for in-memory serialization
- Debugging info strings for development validation (guarded by `KUZU_DESER_DEBUG`)
- Checkpoint format: shadow pages replace originals atomically via DB header update

---

## 9. CSV/JSON/PARQUET IMPORT PIPELINE

### CSV Import (`COPY FROM`):
```
1. Parser: COPY table FROM 'file.csv' (OPTIONS)
2. Binder: bindCopyNodeFrom / bindCopyRelFrom
   - Auto-detect: dialect, delimiters, headers, types
   - Various CSV parsing options (delim, quote, escape, skip, etc.)
3. Planner: LogicalCopyFrom
4. Mapper: maps to PhysicalCopyFrom -> CopyNodeFrom / CopyRelFrom
5. Execution:
   NodeTable:
     - Parallel CSV reader (ParallelCSVReader) chunks file into blocks
     - SerialCSVReader for header + dialect detection
     - Each block parsed independently by Driver
     - Error handling via BatchInsertErrorHandler + FileErrorHandler
     - Bulk insert into InMemChunkedNodeGroupCollection
     - Build primary key hash index (InMemHashIndex)
     - Flush to NodeTable
   RelTable:
     - Two-pass: first partition by source node, then build CSR
     - Partitioner assigns rels to partitions
     - RelBatchInsert builds CSR node groups
     - Supports both FWD and BWD directions
```

### Parquet Import/Export:
- `ParquetReader` with column readers for each Parquet type
- `BooleanColumnReader`, `StringColumnReader`, `ListColumnReader`, `StructColumnReader`, `IntervalColumnReader`, `UUIDColumnReader`, `TemplatedColumnReader`
- RLE/bitpack decoding for repetition/definition levels
- Thrift-based metadata parsing
- `ParquetWriter` for `COPY TO` export

### JSON Import:
- Uses `yyjson` for fast JSON parsing via `TableFunction` interface
- `JSONScan` table function (in extension)

### NYP Import:
- `NpyReader` for NumPy .npy file format

### Error Handling:
- `WarningContext` accumulates row-level errors
- `CopyFromError` captures line number, byte offset, skipped data
- `IGNORE_ERRORS` option to continue on parse failures

---

## 10. KEY ARCHITECTURAL FEATURES

### Factorized Execution Model
- Query processing uses a **factorized** (columnar) model
- `ValueVector` holds flat/unfiltered or selected data
- `DataChunk` = group of `ValueVector`s with shared selection state
- `FactorizedTable` = columnar table with optional flat columns (for joins/aggregates)
- Reduces tuple reconstruction cost in multi-way joins

### Pull-Based Execution
- Physical operators implement `getNextTuple()` (pull from child)
- Parallelism via operator decomposition into `Task`s
- `TaskScheduler` dispatches tasks to worker threads
- Supports intra-operator parallelism (e.g., parallel scan)

### Virtual File System
- `VirtualFileSystem` abstracts local/S3/HTTP file access
- Extensible for new file systems

### Extension System
- Five extension points: `TransformerExtension`, `BinderExtension`, `PlannerExtension`, `MapperExtension`, `CatalogExtension`
- Dynamic library loading via `ExtensionLoader`
- Extension manager handles install/load lifecycle

### Multi-Database (Attach)
- `AttachedKuzuDatabase` for accessing multiple Kuzu instances
- `DatabaseManager` manages attached databases
- `USE database` statement for switching context