6 todos

Let me read the current stubs for ResultCollector and Profile.

Now let me implement ResultCollector properly (consolidates multiple chunks into a single result).

Now let me implement Profile with real timing exposure.

Let me run tests.

Clean. Let me update the documentation and present the full status.

Now let me update the physical operators table in §1.5 and the phase table.

## Objective
- Continue executing the remaining implementation plan items after an audit that corrected all plan documentation inaccuracies against the actual codebase.

## Important Details
- Actual codebase metrics (June 8 audit): 189 .rs files, 61,432 LOC, 29 crates, 960 tests passing, 51 logical operators, 33 physical operators, 234 functions, 43 BoundStatement variants, 58 AST Statement variants, 21 optimizer passes (14 flat + 7 tree).
- Key corrections across all plan docs: test count (952→960), .rs files (~224→189), crates (28→29), LOC (~40k→61,432), logical ops (50→51), physical ops (31→33), functions (155+→234), BoundStatements (28→43), AST variants (38+→58), optimizer passes (21→21 confirmed, C++ has 17 not 19).
- §8 Gap Analysis updated: Types (JSON/UINT128/DTime ✅), Storage (Parquet writer/NPY/HLL ✅ P14), Critical Gaps (CREATE TYPE, COMMENT ON, CREATE/USE/DROP GRAPH, GDS_CALL ✅ P13), overall parity ~88%.
- Remaining items from plan: P-MOD2B (processor.rs 2,755 → 6 modules), P10.3 (STANDALONE_CALL deferred - no functional gap), P14.4/P14.5/P14.6 (Roaring bitmap, lazy scanner, float compression — P3 deferred).

## Work State
- Completed:
  - **Audit & doc fixes:** All 6 plan documents corrected with accurate metrics; §8 Gap Analysis updated; P14 section in implementation_plan.md fixed (was ❌ incorrectly).
  - **P-MOD2B — processor.rs split:** 2,755→6 modules (mod, join_helpers, union_helpers, chunk_helpers, projection_helper, plan_serializer). 960 tests pass, clippy clean.
  - **P16.1 — PhysicalAccumulate:** Real materialize (concatenates all input chunks into one). Previously pass-through.
  - **P16.1 — PhysicalUnion:** Kept (not used — Union handled inline in execute_internal).
  - **P16.2a — ResultCollector:** Real consolidate (merges multiple chunks into one). Previously pass-through.
  - **P10.3/P14.4-6:** Confirmed as intentionally deferred (no functional gap / P3 low priority).
- Active: Continuing P16 physical operator implementations.
- Blocked: None.

## Next Move
1. Implement remaining P16.2 operators: Profile (timing wrapper), DummySink/DummySimpleSink (already correct as no-ops), PackedExtend/Partitioner/PathPropertyProbe/PrimaryKeyScan/AggregateFinalize (deeper architecture work).
2. Run `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` after each change.
3. Keep STATUS.md markdown synced with each implementation step.

Remaining P16.2 stubs all require architectural integration:

| Operator | What's needed |
|----------|--------------|
| `PackedExtend` | Adjacency list resolution for multi-rel extends |
| `Partitioner` | Morsel-driven parallelism framework |
| `PathPropertyProbe` | Storage engine property resolution |
| `PrimaryKeyScan` | ART index integration |
| `AggregateFinalize/Scan` | Split aggregate architecture (partial→finalize) |

These aren't simple stubs — each requires deeper integration with the execution/storage engine. Want to tackle any of these, pivot to another area (e.g., P10.3 STANDALONE_CALL test, optimizer passes, more aggregate/table functions), or stop here?