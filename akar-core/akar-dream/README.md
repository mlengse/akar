# Akar Dream Engine Extension

Dream-engine orchestration for memory consolidation in the Akar database engine.

**Cycle:** NREM → SUPERSEDES → REM → Insight → AFE → Synthesis → DAE

**Components:**
- `DreamOrchestrator` — runs the full consolidation cycle
- `DreamBackend` — backend-agnostic persistence interface
- `DreamConfig` — per-phase configuration
- `DreamStats` — per-phase statistics

**Usage pattern:**
```rust
let mut orchestrator = DreamOrchestrator::new(backend, config);
let stats = orchestrator.run_cycle()?;
```

**Tests:** 5
