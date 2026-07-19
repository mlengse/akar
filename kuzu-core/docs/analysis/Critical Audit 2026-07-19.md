# Critical Audit 2026-07-19: Kuzu C++ → Rust Porting Fidelity

> **Date:** 2026-07-19
> **Scope:** Full codebase audit — Parser, Binder, Planner, Optimizer, Processor, Storage, Extensions
> **Methodology:** Compare C++ Kuzu (v0.12.0), LadybugDB C++ (v0.18.0 fork), against Rust kuzu-core
> **Overall completion:** ~70%

---

## 1. Executive Summary

### Critical Findings (Blockers)

| # | Finding | Severity | Location |
|---|---------|----------|----------|
| 1 | **CSR adjacency stub** — `get_neighbors()` returns `Ok(vec![])` | 🔴 CRITICAL | `kuzu-storage/src/csr.rs:90` |
| 2 | **12 DDL operators no-op** — CREATE/DROP/ALTER TABLE return empty chunks | 🔴 CRITICAL | `kuzu-processor/src/physical/write_ops/map_ddl.rs` |
| 3 | **ORDER BY/LIMIT/SKIP discarded** — parsed but not stored in AST | 🔴 CRITICAL | `kuzu-parser/src/ast.rs:227` |
| 4 | **Binder type resolution hardcoded** — `name`→String, `age`→Int64 heuristic | 🟡 HIGH | `kuzu-binder/src/binder/mod.rs:200-250` |
| 5 | **Checkpoint no-op** — `flush_table()` is empty function | 🟡 HIGH | `kuzu-storage/src/checkpoint.rs` |

### Pipeline Completeness

```
Parser → Binder → Planner → Optimizer → Processor → Storage
  70%      70%      70%        85%         75%          80%
```

**Hot path (SELECT/FILTER/JOIN/AGG):** At parity — Rust 397 µs ≈ Vela 400 µs ≈ LadybugDB 374 µs
**Full pipeline:** ~70% — ORDER BY/LIMIT/SKIP, DDL, CSR adjacency are critical gaps

---

## 2. Architecture Overview

```
C++ Kuzu (v0.12.0)                     Rust kuzu-core (v0.1.0)
├── src/                                ├── kuzu-core/
│   ├── antlr4/ (ANTLR grammar)         │   ├── kuzu-parser/ (pest.rs PEG)
│   ├── main/                           │   ├── kuzu-main/
│   ├── common/                         │   ├── kuzu-common/
│   ├── parser/                         │   ├── kuzu-parser/
│   ├── binder/                         │   ├── kuzu-binder/
│   ├── planner/                        │   ├── kuzu-planner/
│   ├── optimizer/                      │   ├── kuzu-optimizer/
│   ├── processor/                      │   ├── kuzu-processor/
│   ├── storage/                        │   ├── kuzu-storage/
│   ├── catalog/                        │   ├── kuzu-catalog/
│   ├── transaction/                    │   ├── kuzu-transaction/
│   ├── function/                       │   ├── kuzu-function/
│   ├── graph/                          │   ├── kuzu-graph/
│   ├── expression_evaluator/           │   │   (ada di processor)
│   └── c_api/                          │   ├── kuzu-c/
├── extension/ (14 modules)             │   ├── kuzu-{json,fts,vector,...}*
└── tools/                              └── kuzu-cli/, kuzu-wasm/, kuzu-migrate/
```

**Ladybug C++** (`ladybug/`) = fork Kuzu v0.18.0 dengan tambahan fitur (IceDisk, ART Index, morsel scan, concurrent writers, 4 optimizer passes, PackedExtend).

---

## 3. Status Per Layer

| Layer | C++ Kuzu | Rust Port | Parity | Critical Gap |
|-------|----------|-----------|--------|--------------|
| **Parser** | ANTLR4 (917 lines) | pest.rs PEG (477 lines) | **~70%** | ORDER BY/LIMIT/SKIP parsed but discarded |
| **Binder** | Catalog lookup | Hardcoded heuristic | **~70%** | Property type resolution not catalog-based |
| **Planner** | Full logical plan | 15+ empty plans | **~70%** | Never produces OrderBy/Limit/Skip |
| **Optimizer** | 16 passes | 22 passes | **~85%** | TopK/LimitPushDown are dead code |
| **Processor** | 60 operators | 45 implemented | **~75%** | 12 DDL no-op stubs |
| **Storage** | CSR + Checkpoint | Stub CSR, no-op Checkpoint | **~80%** | CSR adjacency broken |
| **Functions** | 234 unique | 234 registered | **~90%** | Alias/overload gap |
| **GDS** | 15 algorithms | 15 algorithms | **100%** | — |
| **Types** | 35+ types | 36 types | **100%** | — |
| **Extensions** | 15 modules | 15 crates | **~15%** | 3/15 implemented, rest placeholders |

---

## 4. Processor Operator Audit

### 4.1 Complete Implementation Matrix (60 LogicalOperator variants)

| # | LogicalOperator | Physical Rust Struct | Status |
|---|---|---|---|
| **SCANS** | | | |
| 1 | `ScanNode` | `PhysicalScan` | ✅ FULLY IMPLEMENTED |
| 2 | `ScanRel` | `PhysicalScanRel` | ✅ FULLY IMPLEMENTED |
| 3 | `VectorSimilarityScan` | `PhysicalVectorSimilarityScan` | ✅ FULLY IMPLEMENTED |
| 4 | `ArtIndexRangeScan` | `PhysicalArtIndexRangeScan` | ✅ FULLY IMPLEMENTED |
| 5 | `IndexLookup` | `PhysicalIndexLookup` | ✅ FULLY IMPLEMENTED |
| 6 | `ExpressionsScan` | (inline in mapper) | ⚠️ STUB |
| 7 | `PathPropertyProbe` | `PhysicalPathPropertyProbe` | ✅ FULLY IMPLEMENTED |
| **FILTER / PROJECTION** | | | |
| 8 | `Filter` | `PhysicalFilter` | ✅ FULLY IMPLEMENTED |
| 9 | `Projection` | `PhysicalProjection` | ✅ FULLY IMPLEMENTED |
| 10 | `Flatten` | `PhysicalFlatten` | ✅ FULLY IMPLEMENTED |
| 11 | `Unwind` | `PhysicalUnwind` | ✅ FULLY IMPLEMENTED |
| 12 | `SemiMasker` | `PhysicalSemiMasker` | ✅ FULLY IMPLEMENTED |
| **SORT / LIMIT / AGGREGATE** | | | |
| 13 | `OrderBy` | `PhysicalOrderBy` | ✅ FULLY IMPLEMENTED |
| 14 | `TopK` | `PhysicalTopK` | ✅ FULLY IMPLEMENTED |
| 15 | `Limit` | `PhysicalLimit` | ✅ FULLY IMPLEMENTED |
| 16 | `Skip` | `PhysicalSkip` | ✅ FULLY IMPLEMENTED |
| 17 | `Aggregate` | `PhysicalAggregate` (split) | ✅ FULLY IMPLEMENTED |
| 18 | `CountRelTable` | `PhysicalCountRelTable` | ✅ FULLY IMPLEMENTED |
| **JOINS** | | | |
| 19 | `HashJoin` | `PhysicalHashJoin` | ✅ FULLY IMPLEMENTED |
| 20 | `SemiJoin` | `PhysicalSemiJoin` | ✅ FULLY IMPLEMENTED |
| 21 | `AntiJoin` | `PhysicalAntiJoin` | ✅ FULLY IMPLEMENTED |
| 22 | `Intersect` | `PhysicalIntersect` | ✅ FULLY IMPLEMENTED |
| 23 | `CrossProduct` | `PhysicalCrossProduct` | ✅ FULLY IMPLEMENTED |
| 24 | `OptionalMatch` | (inline in mapper) | ✅ FULLY IMPLEMENTED |
| **DML / UPDATES** | | | |
| 25 | `Set` | `PhysicalSet` | ✅ FULLY IMPLEMENTED |
| 26 | `Delete` | `PhysicalDelete` | ✅ FULLY IMPLEMENTED |
| 27 | `CreateNode` | `PhysicalInsertNode` | ✅ FULLY IMPLEMENTED |
| 28 | `CreateRel` | `PhysicalInsertRel` | ✅ FULLY IMPLEMENTED |
| 29 | `Extend` | `PhysicalExtend` | ✅ FULLY IMPLEMENTED |
| 30 | `Merge` | `PhysicalMerge` | ✅ FULLY IMPLEMENTED |
| 31 | `Insert` | `PhysicalInsert` | ✅ FULLY IMPLEMENTED |
| 32 | `CopyFrom` | `PhysicalCopyFrom` | ✅ FULLY IMPLEMENTED |
| 33 | `BatchInsert` | `PhysicalBatchInsert` | ✅ FULLY IMPLEMENTED |
| **RECURSIVE / PATH** | | | |
| 34 | `RecursiveExtend` | `PhysicalRecursiveExtend` | ✅ FULLY IMPLEMENTED |
| 35 | `PackedExtend` | `PhysicalPackedExtend` | ✅ FULLY IMPLEMENTED |
| **DDL** | | | |
| 36 | `CreateNodeTable` | (empty chunk) | 🔴 STUB — No-Op |
| 37 | `CreateRelTable` | (empty chunk) | 🔴 STUB — No-Op |
| 38 | `DropTable` | (empty chunk) | 🔴 STUB — No-Op |
| 39 | `AlterTable` | (empty chunk) | 🔴 STUB — No-Op |
| 40 | `CreateIndex` | (empty chunk) | 🔴 STUB — No-Op |
| 41 | `DropIndex` | (empty chunk) | 🔴 STUB — No-Op |
| 42 | `CreateVectorIndex` | (empty chunk) | 🔴 STUB — No-Op |
| 43 | `CreateSequence` | (empty chunk) | 🔴 STUB — No-Op |
| 44 | `DropSequence` | (empty chunk) | 🔴 STUB — No-Op |
| 45 | `CreateDml` | (empty chunk) | 🔴 STUB — No-Op |
| 46 | `ExportDatabase` | (empty chunk) | 🔴 STUB — No-Op |
| 47 | `ImportDatabase` | (empty chunk) | 🔴 STUB — No-Op |
| **FTS** | | | |
| 48 | `CreateFtsIndex` | `PhysicalCreateFtsIndex` | ✅ FULLY IMPLEMENTED |
| 49 | `FtsScan` | `PhysicalFtsScan` | ✅ FULLY IMPLEMENTED |
| **MISC** | | | |
| 50 | `EmptyResult` | `PhysicalEmptyResult` | ✅ FULLY IMPLEMENTED |
| 51 | `MultiplicityReducer` | `PhysicalMultiplicityReducer` | ✅ FULLY IMPLEMENTED |
| 52 | `Union` | (inline in mapper) | ✅ FULLY IMPLEMENTED |
| 53 | `Accumulate` | `PhysicalAccumulate` | ✅ FULLY IMPLEMENTED |
| 54 | `Partitioner` | `Partitioner` | ✅ FULLY IMPLEMENTED |
| 55 | `Explain` | `PhysicalExplain` | ✅ FULLY IMPLEMENTED |
| 56 | `Foreach` | `PhysicalForeach` | ✅ FULLY IMPLEMENTED |
| 57 | `TableFunctionCall` | (delegated) | ⚠️ PARTIAL |
| 58 | `StandaloneCall` | `PhysicalStandaloneCall` | ✅ FULLY IMPLEMENTED |
| 59 | `ExtensionClause` | `PhysicalExtensionClause` | ✅ FULLY IMPLEMENTED |
| 60 | `PrimaryKeyScan` | `PhysicalPrimaryKeyScan` | ✅ FULLY IMPLEMENTED |

### 4.2 Summary Counts

| Category | Count |
|---|---|
| **LogicalOperator variants total** | **60** |
| **Fully implemented** | **45** (75%) |
| **Partially implemented / stubs** | **3** (5%) |
| **No-op stubs (DDL)** | **12** (20%) |

### 4.3 DDL No-Op Details

All 12 DDL operators in `map_ddl.rs` return empty chunks without side effects:

```
CreateNodeTable, CreateRelTable, DropTable, AlterTable,
CreateIndex, DropIndex, CreateVectorIndex, CreateSequence,
DropSequence, CreateDml, ExportDatabase, ImportDatabase
```

**Root cause:** These operators have dispatch paths but `execute()` methods are empty/return empty result.

---

## 5. Binder Audit

### 5.1 Statement Coverage

| Category | Statements | Status |
|---|---|---|
| **Queries** | MATCH, RETURN, WHERE, WITH, CREATE, DELETE, SET, UNWIND, FOREACH, OptionalMatch | ✅ Implemented |
| **DDL** | CREATE NODE/REL TABLE, DROP TABLE, ALTER TABLE | ✅ Implemented |
| **Indexes** | CREATE/DROP INDEX, CREATE VECTOR INDEX | ✅ Implemented |
| **Sequences** | CREATE/DROP SEQUENCE | ✅ Implemented |
| **FTS** | CREATE FTS INDEX, USING FTS INDEX | ✅ Implemented |
| **Graph** | CREATE/USE/DROP GRAPH | ✅ Implemented |
| **Database** | EXPORT/IMPORT DATABASE, ATTACH/DETACH/USE DATABASE | ✅ Implemented |
| **DML** | CREATE DML, MERGE, COPY FROM/TO | ✅ Implemented |
| **Misc** | EXPLAIN, ANALYZE, TRANSACTION, EXTENSION, STANDALONE_CALL, LOAD FROM, CREATE TYPE, CREATE MACRO, COMMENT ON | ✅ Implemented |
| **NOT Implemented** | ORDER BY, SKIP, LIMIT | 🔴 Parser only, binder passes through |

### 5.2 Critical Binder Issues

1. **Property type resolution is heuristic-based:** Uses hardcoded name → type mappings (`name` → String, `age` → Int64) instead of catalog lookup.

2. **Code duplication:** `binder/mod.rs` and `binder/dml.rs` contain nearly identical implementations of 14+ methods (`bind_query`, `bind_match`, `bind_pattern`, etc.). The `mod.rs` version is canonical; `dml.rs` appears to be a refactoring artifact.

3. **`ddl.rs` is actually test code:** File starts with `impl Binder {` but immediately has test assertions — structurally broken.

---

## 6. Planner Audit

### 6.1 Statement Planning Coverage

| Statement | Planner Output |
|-----------|---------------|
| `BoundQuery` | ✅ Full logical plan |
| `BoundCopyFrom` | ✅ LogicalCopyFrom |
| `BoundUnion` | ✅ LogicalUnion |
| `BoundMerge` | ✅ LogicalMerge + LogicalSet |
| `BoundExplain` | ✅ LogicalExplain |
| `BoundCreateNodeTable` | ✅ LogicalCreateNodeTable |
| `BoundCreateRelTable` | ✅ LogicalCreateRelTable |
| `BoundDropTable` | ✅ LogicalDropTable |
| `BoundAlterTable` | ✅ LogicalAlterTable |
| `BoundCreateIndex` | ✅ LogicalCreateIndex |
| `BoundCreateSequence` | ✅ LogicalCreateSequence |
| `BoundCreateFtsIndex` | ✅ LogicalCreateFtsIndex |
| `BoundStandaloneCall` | ✅ LogicalStandaloneCall |
| **15+ other statements** | 🔴 **Empty plan** (`Ok(Vec::new())`) |

### 6.2 Missing Planner Features

1. **ORDER BY/LIMIT/SKIP never produced:** Planner never creates `LogicalOrderBy`, `LogicalLimit`, or `LogicalSkip` operators.

2. **AggregateDetection has no GROUP BY extraction:** Only creates aggregates with empty `group_by` Vec.

3. **TopK/LimitPushDown are dead code:** These optimizer passes look for OrderBy + Limit patterns that the planner never produces.

4. **15+ statements produce empty plans:** Transaction, Extension, Attach/Detach/Use DB, LoadFrom, CreateType, CommentOn, Graph, Analyze, CopyTo, CreateMacro.

---

## 7. Optimizer Audit

### 7.1 Pass Inventory (22 passes: 15 flat + 7 tree)

| # | Pass | Type | Status |
|---|------|------|--------|
| 1 | `RemoveUnnecessaryOperators` | Flat | ✅ Working |
| 2 | `FilterPushDown` | Flat | ✅ Working |
| 3 | `PredicatePushDown` | Flat | ✅ Working |
| 4 | `ProjectionPushDown` | Flat | ✅ Working |
| 5 | `ConstantFolding` | Flat | ✅ Working |
| 6 | `AggregateDetection` | Flat | ⚠️ No GROUP BY extraction |
| 7 | `JoinOptimization` | Flat | ✅ Working |
| 8 | `TopKOptimization` | Flat | 🔴 Dead code (no OrderBy/Limit from planner) |
| 9 | `VectorSimilarityDetection` | Flat | ✅ Working |
| 10 | `ArtRangeScanDetection` | Flat | ✅ Working |
| 11 | `LimitPushDown` | Flat | 🔴 Dead code (no Limit from planner) |
| 12 | `CommonSubexpressionElimination` | Flat | ✅ Working |
| 13 | `OrderByPushDown` | Flat | ✅ Working (Ladybug) |
| 14 | `UnwindDedup` | Flat | ✅ Working (Ladybug) |
| 15 | `CountRelTable` | Flat | ✅ Working (Ladybug) |
| T1 | `FactorizationRewriting` | Tree | ✅ Working |
| T2 | `ForeignJoinPushDown` | Tree | ✅ Working |
| T3 | `AccHashJoinOptimization` | Tree | ✅ Working |
| T4 | `SIPOptimization` | Tree | ✅ Working (has `println!()` debug) |
| T5 | `CorrelatedSubqueryUnnesting` | Tree | ✅ Working |
| T6 | `AggKeyDependency` | Tree | ✅ Working |
| T7 | `CardinalityEstimation` | Tree | ✅ Working |

### 7.2 Dead Code

- **TopKOptimization** — looks for OrderBy + Limit patterns, but planner never produces them
- **LimitPushDown** — looks for Limit operators, but planner never produces them

### 7.3 Code Quality Issues

- `println!()` in SIP pass — should use `tracing::debug!()`
- `#[allow(dead_code)]` annotations present

---

## 8. Storage Engine Audit

### 8.1 Component Status

| Component | C++ Status | Rust Status | Gap |
|-----------|-----------|-------------|-----|
| **BufferManager** | MMAP, NUMA, readahead | Clock eviction, Vec-backed | No memory mapping |
| **RelTable** | CSR adjacency per direction | `Vec<RelData>` flat | 🔴 **CSR stub** |
| **Checkpoint** | Shadow file COW, WAL truncation | `flush_table()` no-op | 🔴 **No persistence** |
| **ART Index** | Node4/16/48/256, persistent | Ported ✅ | — |
| **Hash Index** | On-disk linear hashing | Ported ✅ | — |
| **WAL** | 16 record types, CRC32 | Ported ✅ | — |
| **Compression** | ALP, bitpacking, Dictionary | Per-type dispatch | StringDictionary pass-through |
| **LocalStorage** | Arrow columnar | Serialized byte blobs | Not vectorized |
| **ShadowFile** | COW at BM level | HashMap in-memory | Not persisted |
| **Spiller** | Arrow IPC zero-copy | JSON lines | Slow format |
| **RoaringBitmap** | Array + Bitmap + Run | Array + Bitmap only | No Run, no serde |

### 8.2 CSR Stub Details

```rust
// kuzu-storage/src/csr.rs:90
pub fn get_neighbors(&self, _node_id: u64) -> Result<Vec<u64>, String> {
    Ok(vec![])  // Returns empty — graph traversal broken
}
```

RelTable uses flat `Vec<RelData>` as fallback. Graph traversal queries (MATCH (a)-[r]->(b)) cannot use CSR-accelerated neighbor lookup.

### 8.3 Checkpoint Stub Details

```rust
// kuzu-storage/src/checkpoint.rs
pub fn flush_table(&self, _table: &Table) -> Result<(), String> {
    Ok(())  // No-op — data never persists to disk
}
```

---

## 9. Extensions Status

| Extension | C++ | Rust | Status |
|-----------|-----|------|--------|
| json | Full | `kuzu-json` | Placeholder |
| fts | Full | `kuzu-fts` | ✅ Implemented (BM25) |
| vector (HNSW) | Full | `kuzu-vector` | ✅ Implemented |
| httpfs | Full | `kuzu-httpfs` | Placeholder |
| duckdb | Full | `kuzu-duckdb` | Placeholder |
| algo (GDS) | Full | `kuzu-algo` | ✅ Implemented |
| neo4j | Full | `kuzu-neo4j` | Placeholder |
| llm | Full | `kuzu-llm` | Placeholder |
| sqlite | Full | `kuzu-sqlite` | Placeholder |
| delta | Full | `kuzu-delta` | Placeholder |
| iceberg | Full | `kuzu-iceberg` | Placeholder |
| azure | Full | `kuzu-azure` | Placeholder |
| postgres | Full | `kuzu-postgres` | Placeholder |
| unity-catalog | Full | `kuzu-unity-catalog` | Placeholder |
| adbc | Full (Ladybug) | `kuzu-main/adbc.rs` | Basic wrapper |

**Implemented:** 3/15 (fts, vector, algo) + ADBC wrapper
**Placeholders:** 12/15

---

## 10. Cross-Cutting Findings

### 10.1 Code Quality

- **No `todo!()` or `unimplemented!()`** in any physical operator code
- **No `placeholder`, `stub`, `FIXME`, `HACK`, `XXX`** comments
- **No TODO comments**
- **5 `unreachable!()` occurrences** — all in binder (duplicate code)

### 10.2 Structural Issues

1. **Binder code duplication:** `binder/mod.rs` and `binder/dml.rs` contain 14+ identical methods
2. **`ddl.rs` is test code masquerading as implementation**
3. **No `PhysicalOperatorType` enum** — string-based `operator_type()` instead
4. **No `LogicalWindow` operator** — window functions not implemented
5. **Extensions are compile-time only** — dynamic INSTALL/LOAD not supported

---

## 11. Completion Metrics

| Area | Perkiraan Completion | Baris Kode (Rust) |
|------|---------------------|-------------------|
| Common/Type System | ~95% | ~5K |
| Storage Engine | ~80% (CSR broken) | ~15K |
| Transaction | ~95% | ~1K |
| Catalog | ~100% | ~1.5K |
| Parser | ~70% (ORDER BY discarded) | ~2.5K |
| Binder | ~70% (heuristic types) | ~3K |
| Planner | ~70% (15+ empty plans) | ~2.5K |
| Optimizer | ~85% (dead code) | ~3.5K |
| Processor | ~75% (12 DDL stubs) | ~8K |
| Function | ~90% | ~3K |
| Graph/GDS | ~40% | ~2K |
| Extensions | ~15% (3/15 implemented) | ~5K |
| CLI/WASM/C API | ~90% | ~1.5K |
| **Total** | **~70%** | **~55K+** |

---

## 12. Priority Recommendations

### 🔴 Blockers (cannot run meaningful queries)

1. **CSR adjacency** — graph traversal broken
2. **12 DDL no-ops** — database cannot be schema'd
3. **ORDER BY/LIMIT/SKIP** — silently discarded
4. **DDL physical operators** — must modify catalog/storage

### 🟡 High Priority

5. **Binder property type resolution** — catalog lookup instead of heuristic
6. **Eliminate code duplication** in binder
7. **Planner must produce OrderBy/Limit/Skip** from AST
8. **Aggregate GROUP BY extraction** — currently only COUNT(*)
9. **TableFunctionCall processor** — only vector_similarity_scan works
10. **Checkpoint persistence** — flush_table() must actually persist

### 🟠 Medium Priority

11. **Column batch scan** — Arrow-format instead of Vec<Value>
12. **StringDictionary compression** — actual encoding
13. **ShadowFile disk persistence**
14. **WAL truncation/rotation**
15. **Stats persistence**
16. **RoaringBitmap Run container + serialization**
17. **Binder ORDER BY/LIMIT/SKIP binding**
18. **Planner handling of 15+ empty-plan statements**

### 🟢 Low Priority (Ladybug-specific)

19. **IceDisk full implementation**
20. **Morsel-driven scan parallelism**
21. **PackedExtend + PackedChildSlices**
22. **ColumnarNodeTableBase / ColumnarRelTableBase**
23. **ForeignRelTable**
24. **StorageFormat enum**
