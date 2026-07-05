# Audit & Porting Plan — Kuzu C++ → Rust (2026-07-01)

## 1. Verifikasi Status — Apa yang Didapatkan dari Audit

### ✅ Terverifikasi BENAR (STATUS.md & implementation_plan.md akurat)

| Klaim | Hasil Verifikasi |
|-------|-----------------|
| **52/52 fitur real implementation** | ✅ Terverifikasi — semua fitur ada |
| **691 test pass** (diklaim 540+) | ✅ **691 test**, 0 failures — melebihi klaim |
| **17 physical operators** | ✅ 18 terverifikasi (termasuk ExpressionEvaluator) |
| **23 logical operator variants** | ✅ Terverifikasi |
| **13 optimizer passes** | ✅ 11 flat + 2 tree = 13 pass |
| **UNION ALL execution** | ✅ Planner + processor + `all` flag + 9 tests |
| **MERGE execution** | ✅ Planner + processor + ON MATCH/ON CREATE SET + 5 tests |
| **OptionalMatch tree** | ✅ Left/right independent execution + merge |
| **ART Index** | ✅ Node4/16/48/256 + range_scan + BufferManager persistence |
| **HNSW Vector Index** | ✅ Full integration: DDL, persistence, 5 metrics, optimizer detection |
| **Disk Spilling** | ✅ Spiller + MultiWayStreamMerge + NodeGroup hooks + 9 tests |
| **Concurrent Multi-Writer** | ✅ dashmap catalog + LocalWAL + MVCC version chains |
| **100+ built-in functions** | ✅ 78 scalar + 9 aggregate + table functions |
| **15 extension crates** | ✅ 16 crates (14 named + kuzu-extension + kuzu-graph) |
| **Value enum** | ✅ 27 variants sesuai C++ |
| **LogicalTypeID** | ✅ 34 variants — semua tipe utama ada |

### ❌ Terverifikasi SALAH atau Berlebihan

| Klaim | Realitas |
|-------|----------|
| **"0 clippy warnings"** | ❌ **128 warnings** — bukan 0 |
| **"WASM support — all crates check clean"** | ❌ **Tidak ada actual WASM cfg gates** — hanya comment-level awareness di DuckDB |
| **"Full TCK — 100%"** | ❌ **Variable-length path hanya PARSER/BINDER** — tidak ada RecursiveExtend operator |
| **"Full Cypher coverage"** | ❌ EXPLAIN, IMPORT/EXPORT DATABASE tidak ada |

---

## 2. Gap Analysis — Fitur C++ yang Belum di-Port ke Rust

### 🔴 P1 — Critical for Cypher Completeness

#### GAP 1: Variable-length Path Physical Execution (RecursiveExtend)
| Aspek | Detail |
|-------|--------|
| **C++ Source** | `src/include/processor/operator/recursive_extend.h`, `src/include/processor/operator/path_property_probe.h` |
| **Rust Status** | ✅ Parser: `[*1..5]` grammar + AST bounds. ❌ **Tidak ada RecursiveExtend physical operator** |
| **Dampak** | Query `MATCH (a)-[*1..3]->(b)` parse OK, bind OK, **execute hasilnya kosong/salah** |
| **Estimasi** | ~3-4 hari. Butuh: RJAlgorithm framework, RecursiveExtend operator, PathPropertyProbe, BFS graph traversal |
| **Complexity** | Tinggi — terkait erat dengan GDS framework |

#### GAP 2: Shortest Path Algorithms
| Aspek | Detail |
|-------|--------|
| **C++ Source** | `src/include/function/gds/gds_function_collection.h` — 8 RJAlgorithm variants |
| **Rust Status** | ✅ PageRank, WCC, SCC, K-Core, Louvain, Spanning Forest. ❌ **Tidak ada shortest path** |
| **Dampak** | `MATCH (a)-[e* SHORTEST 1..5]->(b)` tidak bisa dijalankan |
| **Estimasi** | ~2-3 hari. Port dari C++ `weight_utils.h` + shortest path algorithms |
| **Complexity** | Sedang — butuh weighted graph support |

### 🟠 P2 — Important for Feature Parity

#### GAP 3: Sequence / Auto-increment (SERIAL)
| Aspek | Detail |
|-------|--------|
| **C++ Source** | `src/include/catalog/catalog_entry/sequence_catalog_entry.h`, `logical_create_sequence.h` |
| **Rust Status** | ✅ `LogicalTypeID::Serial` exists. ❌ **No SequenceEntry, no nextval/currval, no CREATE SEQUENCE** |
| **Dampak** | `SERIAL` column type cannot auto-generate values |
| **Estimasi** | ~1 hari |

#### GAP 4: Schema Functions
| Aspek | Detail |
|-------|--------|
| **C++ Source** | `src/include/function/schema/` — `offset_functions.h`, `vector_node_rel_functions.h` |
| **Rust Status** | ❌ **Tidak ada**: `OFFSET()`, `ID()`, `START_NODE()`, `END_NODE()`, `LABEL()` |
| **Dampak** | Query seperti `RETURN ID(n), LABEL(n), START_NODE(r)` tidak dikenal |
| **Estimasi** | ~1 hari |

#### GAP 5: EXPLAIN Statement
| Aspek | Detail |
|-------|--------|
| **C++ Source** | `src/include/parser/explain_statement.h`, `logical_explain.h` |
| **Rust Status** | ❌ **Tidak ada**: Statement::Explain tidak ada di AST, parser, planner, atau processor |
| **Dampak** | `EXPLAIN MATCH (n) RETURN n` tidak dikenal |
| **Estimasi** | ~1 hari |

#### GAP 6: IMPORT DATABASE / EXPORT DATABASE
| Aspek | Detail |
|-------|--------|
| **C++ Source** | `src/include/processor/operator/simple/import_db.h`, `export_db.h` |
| **Rust Status** | ❌ **Tidak ada**: hanya CLI `.import`/`.export` untuk CSV single-table |
| **Dampak** | Tidak bisa export/import full database |
| **Estimasi** | ~2-3 hari |

### 🟡 P3 — Performance & Quality

#### GAP 7: Intersect Operator
| Aspek | Detail |
|-------|--------|
| **C++ Source** | `src/include/processor/operator/intersect/intersect.h`, `intersect_build.h` |
| **Rust Status** | ❌ **Tidak ada** |
| **Dampak** | Multi-pattern matching (`(a)-[:r1]->(b), (a)-[:r2]->(c)`) tidak optimal |
| **Estimasi** | ~2 hari |

#### GAP 8: SIP (Sideways Information Passing)
| Aspek | Detail |
|-------|--------|
| **C++ Source** | `src/include/planner/operator/sip/` — `side_way_info_passing.h`, `logical_semi_masker.h` |
| **Rust Status** | ❌ **Tidak ada** |
| **Dampak** | Hash join tidak bisa push semi-mask ke scan — full scan tiap sisi |
| **Estimasi** | ~2 hari |

#### GAP 9: Array Functions
| Aspek | Detail |
|-------|--------|
| **C++ Source** | `src/include/function/array/` — 13 functions |
| **Rust Status** | ❌ **Tidak ada**: array_cosine_similarity, array_distance, dll. |
| **Dampak** | Tidak bisa gunakan array math untuk vector/embedding ops |
| **Estimasi** | ~1 hari |

#### GAP 10: Clippy Warnings (128)
| Aspek | Detail |
|-------|--------|
| **Detail** | 128 warnings across ~15 crates |
| **Top offenders** | kuzu-storage (27), kuzu-processor (23), kuzu-fts (9), kuzu-graph (3), etc. |
| **Estimasi** | ~2 hari untuk cleanup menyeluruh |

### 🟢 P4 — Low Priority

#### GAP 11: WASM Configuration
| Aspek | Detail |
|-------|--------|
| **Saat ini** | Tidak ada `.cargo/config.toml` untuk wasm target, tidak ada `#[cfg]` gates |
| **Estimasi** | ~1 hari setup |

#### GAP 12: Additional Catalog Entry Types
| Aspek | Detail |
|-------|--------|
| **C++** | 14 catalog entry types. **Rust**: hanya 3 (`NodeTable`, `RelTable`, `VectorIndex`) |
| **Missing** | `SequenceEntry`, `ForeignTableEntry`, `StandaloneTableFunctionEntry`, `TypeEntry` |
| **Estimasi** | ~1 hari per entry type |

---

## 3. Rencana Porting — Prioritas & Timeline

### FASE C: Cypher Completeness (P1) — Estimasi: 5-7 hari

| Step | Task | Files | Complexity |
|------|------|-------|------------|
| **C1** | **RecursiveExtend Operator** | Baru: `kuzu-processor/src/recursive_extend.rs` | 🔴 Tinggi |
| C1.1 | RJAlgorithm trait + BFS frontier | `kuzu-graph/src/bfs.rs` (baru) | |
| C1.2 | RecursiveExtend physical operator (sink, wrapping RJAlgorithm) | `kuzu-processor/src/physical_operator.rs` | |
| C1.3 | PathPropertyProbe — fetch properties for path nodes/edges | `kuzu-processor/src/path_property_probe.rs` (baru) | |
| C1.4 | LogicalRecursiveExtend + planner integration | `kuzu-planner/src/logical_operator.rs`, `planner.rs` | |
| C1.5 | Binder: bind recursive path patterns with bounds | `kuzu-binder/src/binder.rs` | |
| C1.6 | Tests: var-length path execution (`[*], [*1..3], [r:KNOWS*1..5]`) | `kuzu-main/tests/` | |
| **C2** | **GDS Shortest Path** | Baru: `kuzu-algo/src/shortest_path.rs` | 🟠 Sedang |
| C2.1 | WeightUtils + Dijkstra/A* shortest path | Port dari `weight_utils.h` + `rec_joins.h` | |
| C2.2 | AllSP / SingleSP / WeightedSP algorithm variants | 8 algorithm functions | |
| C2.3 | RJAlgorithm trait (gunakan dari C1) | | |
| C2.4 | Tests: shortest path queries | `kuzu-algo/tests/` | |

### FASE D: Feature Parity (P2) — Estimasi: 4-6 hari

| Step | Task | Files | Complexity |
|------|------|-------|------------|
| **D1** | **Sequence Support** | Baru: `kuzu-catalog/src/sequence_entry.rs` | 🟢 Rendah |
| D1.1 | SequenceCatalogEntry struct (currVal, nextKVal, rollbackVal) | | |
| D1.2 | CREATE SEQUENCE grammar + AST + parser + binder + planner | Cross-cutting | |
| D1.3 | nextval/currval built-in functions | `kuzu-function/src/registry.rs` | |
| D1.4 | SERIAL column auto-increment di insert_row() | `kuzu-storage/src/table.rs` | |
| **D2** | **Schema Functions** | `kuzu-function/src/registry.rs` | 🟢 Rendah |
| D2.1 | OFFSET(node) → i64 | | |
| D2.2 | ID(node_or_rel) → InternalID | | |
| D2.3 | START_NODE(rel), END_NODE(rel) | | |
| D2.4 | LABEL(node_or_rel) → String | | |
| **D3** | **EXPLAIN Statement** | Cross-cutting | 🟠 Sedang |
| D3.1 | ExplainType enum + Statement::Explain di AST | | |
| D3.2 | Grammar `EXPLAIN [PROFILE] <statement>` | | |
| D3.3 | LogicalExplain planner node | | |
| D3.4 | PhysicalExplain — serialize plan tree ke output | | |
| **D4** | **IMPORT/EXPORT DATABASE** | Baru | 🟠 Sedang |
| D4.1 | IMPORT DATABASE grammar + sequential COPY FROM | | |
| D4.2 | EXPORT DATABASE grammar + sequential COPY TO | | |

### FASE E: Performance (P3) — Estimasi: 5-6 hari

| Step | Task | Files | Complexity |
|------|------|-------|------------|
| **E1** | **Intersect Operator** | Baru | 🟠 Sedang |
| E1.1 | IntersectBuild + Intersect physical operator | | |
| E1.2 | LogicalIntersect + planner | | |
| E1.3 | Two-way sorted node ID intersection | | |
| **E2** | **SIP (Semi Masks)** | Baru | 🟠 Sedang |
| E2.1 | SemiMasker logical operator | | |
| E2.2 | SemiMaskTargetType (SCAN_NODE, RECURSIVE_EXTEND, etc.) | | |
| E2.3 | SIP in join optimization pass | | |
| **E3** | **Array Math Functions** | `kuzu-function/` | 🟢 Rendah |
| E3.1 | array_cosine_similarity, array_distance, dll. | | |
| **E4** | **Clippy Cleanup (128 warnings)** | All crates | 🟢 Rendah |
| E4.1 | Fix: `io::Error::other()`, `div_ceil`, manual range contains | | |
| E4.2 | Fix: redundant closures, identity map, unneeded return | | |
| E4.3 | Fix: large type complexity, pointer deref warnings | | |

### FASE F: Polish (P4) — Estimasi: 2-3 hari

| Step | Task | Complexity |
|------|------|------------|
| **F1** | **WASM Target Setup** — `.cargo/config.toml` + cfg gates | 🟢 Rendah |
| **F2** | **Catalog Entry Expansion** — SequenceEntry, ForeignTableEntry, dll. | 🟢 Rendah |
| **F3** | **DDL Operator Refactor** — align with C++ DDL operator structure | 🟢 Rendah |

---

## 4. Total Estimasi

| Fase | Tujuan | Hari | Prioritas |
|------|--------|------|-----------|
| **C** | Cypher Completeness (RecursiveExtend + Shortest Path) | 5-7 | 🔴 P1 |
| **D** | Feature Parity (Sequence + Schema Functions + EXPLAIN + Import/Export) | 4-6 | 🟠 P2 |
| **E** | Performance (Intersect + SIP + Array Functions + Clippy) | 5-6 | 🟡 P3 |
| **F** | Polish (WASM + Catalog + DDL) | 2-3 | 🟢 P4 |
| **Total** | **13 gap items** | **16-22 hari** | |

---

## 5. Catatan Penting

1. **STATUS.md overclaiming**: 3 klaim diverifikasi salah atau berlebihan — WASM, 0 clippy warnings, Full TCK. Perlu diperbaiki.
2. **Actual codebase health**: Sangat baik — 691 test pass, 0 kompilasi error, 0 clippy errors. Ini menunjukkan kualitas tinggi.
3. **Strategic gap**: Variable-length path execution adalah gap terbesar — ini fundamental untuk Cypher TCK compliance.
4. **Opportunity**: Shortest path + GDS algorithms adalah flagship feature Kuzu yang belum di-port optimal ke Rust.
