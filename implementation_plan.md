# Implementation Plan: Free Space Manager & Zone Map Predicate

## Goal Description
We need to integrate two dormant storage optimizations into the main storage/scan flow:
1. **Free Space Manager**: A component that tracks reusable disk pages (using a buddy-system approach) to avoid endless file growth upon row deletions.
2. **Zone Map Predicate**: A pushdown optimization that evaluates min/max/null statistics on chunks to skip scanning entirely when a predicate cannot possibly match any values in the chunk.

## Decisions Made
- **Free Space Manager Persistence**: We will use an in-memory-only implementation for now. Persistence via WAL or a `.fsm` file can be added later.
- **Zone Map Integration**: Zone map evaluation will occur at the `ColumnChunk` level inside `NodeTable::scan` / `PhysicalScan` before reading the chunk into memory.

## Proposed Changes

---

### kuzu-planner/src/join_order.rs
Update `flatten_plan` to properly encapsulate sub-pipelines.
#### [MODIFY] join_order.rs
Instead of pushing the left and right sub-plans into the shared `ops` vector, we will serialize them into `Vec<LogicalOperator>` and wrap them in a synthetic `LogicalOperator::Projection` (with empty expressions). This leverages existing pipeline evaluation logic used by `Union` without changing the `LogicalOperator` enum.
- In `JoinPlan::HashJoin`, map `left` and `right` to their own flattened vectors, wrap them in `LogicalOperator::Projection`, and store them in `build_side` and `probe_side`.
- In `JoinPlan::CrossProduct`, do the same for `left` and `right`.

---

### kuzu-processor/src/processor.rs
Update `Processor::execute` to evaluate left and right sides of joins independently.
#### [MODIFY] processor.rs
- For `LogicalOperator::HashJoin` and `LogicalOperator::CrossProduct`, extract the left and right pipelines using `flatten_union_child`.
- Call `self.execute(&left_ops)` and `self.execute(&right_ops)` to independently evaluate `build_chunks` and `probe_chunks`.
- Call the new inherent `execute_binary` method on `PhysicalHashJoin` and `PhysicalCrossProduct` passing both `build_chunks` and `probe_chunks` explicitly.

---

### kuzu-processor/src/physical_operator.rs
Remove the flawed `input.len() / 2` logic and accept exact inputs.
#### [MODIFY] physical_operator.rs
- Add `pub fn execute_binary(&self, build_chunks: Vec<DataChunk>, probe_chunks: Vec<DataChunk>) -> OperatorResult` as inherent methods to `PhysicalHashJoin` and `PhysicalCrossProduct`.
- Remove their `PhysicalOperatorExec` trait implementations entirely, as the processor uses concrete types and no longer needs the unary trait for these operators.
- Inside `execute_binary`, directly use `build_chunks` and `probe_chunks` without guessing boundaries.

## Verification Plan

### Automated Tests
- Run all `cargo test` across `kuzu-processor`, `kuzu-planner`, and `kuzu-main`.
- Add integration tests for 0-match joins, cross product without WHERE, and asymmetric table sizes (e.g. joining 4096 rows with 1 row) to prove that the left/right boundary is preserved.
