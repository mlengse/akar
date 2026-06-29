### What's Still Missing 🔴 (from MISSING-2.md + codebase check)

I verified these against the actual source:

#### 1. 🔴 `PhysicalScan` still generates **synthetic data**

```rust
// kuzu-processor/src/physical_operator.rs, line ~45
let mut v = ValueVector::new(PhysicalTypeID::Int64, 1000);
for i in 0..100.min(self.estimated_cardinality as usize) {
    v.set_i64(i, (col_id as i64) * 1000 + i as i64);
}
```

Not connected to `NodeTable`/`RelTable`. The table structs themselves are **schema-only** with `num_rows: 0` and no in-memory data storage.

#### 2. 🔴 No real columnar on-disk storage

`NodeTable` and `RelTable` in `kuzu-storage/src/table.rs` have no methods to read/write rows. The Buffer Manager, WAL, and compression exist but aren't integrated with table storage.

#### 3. 🔴 Limited Cypher coverage

Parser grammar (cypher.pest) only supports:
- ✅ `CREATE NODE TABLE`, `CREATE REL TABLE`, `DROP TABLE`
- ✅ `MATCH` .. `RETURN` .. `WHERE`
- ✅ `ORDER BY`, `LIMIT`, `SKIP`
- ❌ `COPY FROM/TO` — **no data loading**
- ❌ `ALTER TABLE`
- ❌ `MERGE`, `DELETE`, `SET`
- ❌ `UNION`, `UNION ALL`
- ❌ `OPTIONAL MATCH`
- ❌ `WITH` clause
- ❌ `UNWIND`
- ❌ Subqueries / `CALL`
- ❌ Variable-length paths `(a)-[*1..5]->(b)`

#### 4. Missing from plans entirely

| Item | Status |
|------|--------|
| **rust_api integration** | ❌ Not started — C++ FFI still in parent |
| **CI/CD** (Fase 21) | ❌ Not configured |
| **C++ code removal** (Fase 14) | ❌ Out of scope / not done |
| **FactorizedTable** | ❌ Not ported (C++ has 10 files) |
| **CSV/Parquet IO** | ❌ Not ported |

---

### Plans vs Reality Map


Plan file                            | Status    | What's left
-------------------------------------|-----------|-------------------------------------------
plan-kuzuRefactor.prompt.md (F0-14)  | ✅ Mostly | C++ cleanup, tools/rust_api
plan-kuzuRustGap.prompt.md (F1-4)   | ✅ Mostly | Real storage read in PhysicalScan
plan-planLanjutan.prompt.md (F15-21) | ✅ Mostly | CI/CD, tools/rust_api
plan-optimizerAndExtensions.prompt.md| ✅ Done   | Nothing — optimizer + extensions complete
plan-kuzuDuckdbBindingPlan.prompt.md | ✅ Done   | Nothing — DuckDB + 6 extensions complete
plan-kuzuAuditBenchmarkPlan.prompt.md| ❌ Pending| Phases 1-6 (build C++ benchmark, add Rust benches, gap analysis)


---

### The REAL Remaining Work (Priority Order)

Based on reconciling all documents with the codebase:

| Priority | What | Where | Effort |
|----------|------|-------|--------|
| **P0** | **Connect `PhysicalScan` to real table data** | `kuzu-processor/src/physical_operator.rs` + `kuzu-storage/src/table.rs` | Medium — need to add actual data storage to NodeTable/RelTable |
| **P0** | **Add `COPY FROM`** (CSV/Parquet loading) | Parser + Binder + Processor | Large — grammar, binding, file parsing |
| **P0** | **Implement columnar storage** on `NodeTable`/`RelTable` | `kuzu-storage/src/table.rs` | Large — connect BufferManager, WAL, compression |
| **P1** | **Add Cypher features**: DELETE, SET, ALTER, MERGE, OPTIONAL MATCH, WITH, UNION, UNWIND | Parser + Binder + Planner + Processor | Large — multi-layered |
| **P2** | **Benchmark infrastructure** (Phase 1-6 of audit plan) | `kuzu-core/kuzu-main/benches/`, C++ build | Medium |
| **P3** | **CI/CD** (Fase 21) | workflows | Low |
| **P3** | **tools/rust_api integration** (Fase 20) | rust_api | Medium |

The **biggest architectural gap** remains: the project has built a *beautiful, complete query compilation pipeline* (parse → bind → plan → optimize → execute) but the **last mile** — connecting the physical operators to real data — isn't done. Queries compile and plan correctly, but `PhysicalScan` returns synthetic data instead of reading from storage.