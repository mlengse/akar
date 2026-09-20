# Akar Dream Engine Extension

Dream phase **compute primitives** for memory consolidation in the Akar database
engine, plus the storage port they run over.

**Phases (primitives):** NREM → SUPERSEDES → REM → Insight → AFE → Synthesis → DAE

**Components:**
- `DreamBackend` — backend-agnostic storage port (`MockBackend` is an in-memory double)
- `DreamConfig` — primitive tuning only (sampling, decay, thresholds)
- `PhaseStats` — the result of a *single* phase call
- `phases::<name>::run_*` — one function per phase

**The cycle is host-owned.** Akar does not decide which phases run, in what
order, when to trigger them, or how pause/resume behaves (SPEC §13): those are
sequencing decisions, so they belong to the host that owns the loop —
`akar-server` among these crates as the wire reference, `sulur-server` in
production.

**Usage pattern:**
```rust
use akar_dream::config::DreamConfig;
use akar_dream::phases;

let config = DreamConfig::default();
let stats = phases::nrem::run_nrem(&backend, &config);
```

**Tests:** 8
