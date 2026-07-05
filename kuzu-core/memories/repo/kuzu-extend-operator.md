# Extend Operator Implementation (March 2025)

## What was implemented
The `Extend` logical and physical operator for relationship pattern matching in queries like `MATCH (u:User)-[:Likes]->(p:Post)`.

## Problem
`ScanRel` was doing a full table scan of the relationship table, then combined with other scans via CrossProduct. For `MATCH (u:User)-[:Likes]->(p:Post) WHERE u.id = 1 RETURN p.content` this produced 4 rows instead of 1 (the expected "Hello").

## Solution
Added `LogicalExtend` and `PhysicalExtend` operators that properly extend from a source node through a relationship table:
1. Takes source node scan output as input
2. For each source row, looks up adjacency list in the rel table via `RelTable::scan_adj_list()`
3. Produces expanded output with source fields + rel properties + dest node properties

## Files changed
- `kuzu-planner/src/logical_operator.rs` — Added `LogicalExtend` struct and `Extend` variant
- `kuzu-planner/src/planner.rs` — Planner now creates Extend instead of standalone ScanRel for connected patterns
- `kuzu-processor/src/physical_operator.rs` — Added `PhysicalExtend` with adjacency-based execution; also fixed `PhysicalArtIndexRangeScan` to include field_names
- `kuzu-processor/src/processor.rs` — Added Extend mapping, ArtIndexRangeScan alias prefixing
- `kuzu-optimizer/src/passes.rs` — Extend added to match arms
- `kuzu-optimizer/src/join_order.rs` — Extend added to match arms

## Key design decisions
- Extend is a pipeline operator (no children), placed between source scan and filter/projection
- Extend replaces both ScanRel and destination ScanNode in the pipeline
- Column layout: [input_fields | rel_properties | dest_node_fields]
- Field names prefixed with variable names for correct filtering/projection
