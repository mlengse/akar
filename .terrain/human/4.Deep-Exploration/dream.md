# Deep Exploration — akar-dream

The dream orchestrator is Akar's memory-consolidation engine: it runs scheduled "sleep" cycles that reorganize an agent's memory graph — detecting duplicate/sovereign facts, repurposing spans, writing consolidated backdoors, and logging the outcomes — so memory stays bounded and meaningful as it grows. It is driven either from Python (`akar-python::dream()`) or via `akar-server` dream endpoints.

## Key Components

| Phase | Purpose | Located at |
|-------|---------|-----------|
| `run_cycle` | Entry point: runs all phases, returns `DreamReport{successful, latency, details}` | `akar-core/akar-dream/src/orchestrator.rs:37` |
| NREM phase1 / phase2 | Backbone consolidation scan | `akar-core/akar-dream/src/` |
| SUPERSEDES | Mark old/sovereign facts as superseded by newer ones | `akar-core/akar-dream/src/` |
| REM | Revisit/repurpose spans | `akar-core/akar-dream/src/` |
| WRITE | Persist consolidated state (checkpoint + compact) | `akar-core/akar-dream/src/` |
| LOG / GRAPH / DAE | Log outcomes, update graph, run DAE (data aggregation engine) | `akar-core/akar-dream/src/` |

## Design Decisions

- **Phases run in a fixed cycle.** NREM → SUPERSEDES → REM → WRITE → LOG → GRAPH → DAE. Fixed ordering makes the cycle reproducible and testable, at the cost of flexibility (a single `run_cycle` is the unit; finer-grained control uses the phase functions directly).
- **Consolidation runs in-process but through the normal query path.** Dreams manipulate memory via the same Cypher DDL/DML as the agent — no special storage hooks — keeping the memory engine consistent with normal writes.
- **Checkpoint at the end of the cycle.** `WRITE` triggers storage flush + compact so consolidated state is durable before the orchestrator returns; latency reported in `DreamReport` covers the full cycle.

## Why It Matters

Dream is what distinguishes "database with embeddings" from a memory system. Without consolidation, memory graphs degrade with duplicate facts and stale spans. The orchestrator's telemetry (`DreamReport`), exposed to sulur via `akar-server` `/dream/start` + `/dream/stop`, lets the daemon schedule sleep during idle windows.