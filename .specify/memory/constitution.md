<!--
SYNC IMPACT REPORT (temporary — remove before committing this file)
================================================================
Version change: (none — unfilled scaffold) → 1.0.0
Bump rationale: initial ratification. No prior governance text existed in
.specify/memory/constitution.md (file held the verbatim template with every
[ALL_CAPS_IDENTIFIER] placeholder unset), so there is no prior version to be
backward incompatible with. 1.0.0 = first stable governance baseline, not a MAJOR
redefinition of an existing constitution.

Modified principles (all five are initial content, none renamed):
- [PRINCIPLE_1_NAME] → I. Pure Rust, Zero FFI (NON-NEGOTIABLE)
- [PRINCIPLE_2_NAME] → II. Spec-Driven Development (NON-NEGOTIABLE)
- [PRINCIPLE_3_NAME] → III. Test-Gated Delivery (NON-NEGOTIABLE)
- [PRINCIPLE_4_NAME] → IV. Evidence Over Assertion (NON-NEGOTIABLE)
- [PRINCIPLE_5_NAME] → V. Embedded-Only Domain Boundary

Added sections:
- ## Engineering Constraints   (was [SECTION_2_NAME])
- ## Development Workflow & Quality Gates   (was [SECTION_3_NAME])
- ## Governance                 (filled; was [GOVERNANCE_RULES] placeholder)

Removed sections: none (template headings preserved; only placeholder bodies
replaced).

Follow-up TODOs / deferred items: none. No placeholder was intentionally
deferred; RATIFICATION_DATE is known (2026-09-28, this amendment) because the
constitution was never previously ratified.
-->

# Akar Constitution

Akar is a pure-Rust embedded graph database for AI agent memory. This constitution
governs every change to the `akar` repository, its Cargo workspace, and its
published crates.

## Core Principles

### I. Pure Rust, Zero FFI (NON-NEGOTIABLE)

Akar is a from-scratch pure-Rust reimplementation. It carries zero C++ dependencies
and zero foreign-function-interface shims in any production code path. This is not
a style preference: it is the project's memory-safety and auditability guarantee,
and it is the reason Akar can ship inside a host process without inheriting that
host's native toolchain. Any change that introduces a C/C++ dependency, a `build.rs`
that compiles native source, or an `extern "C"` block outside the dedicated
`akar-c` binding crate MUST be rejected as a backward-incompatible governance
change.

The single admitted exception is `akar-c`, the published-on-request C binding
crate, which exists solely so other languages can embed Akar. It is a leaf: it
depends on core crates and nothing in core may depend on it.

A second, narrower exception exists for extension crates that link optional system
libraries (`libduckdb-sys`, `SQLite`, and peers). These libraries MUST be gated
behind opt-in cargo features, MUST NOT be reachable from `akar-main`'s default
feature set, and MUST NOT be compiled by the release gate. Where a feature-gated
dependency is unavoidable, the `check [akar-core]` pre-commit run (release
profile, all features) exists to keep that surface compiling.

Rationale: the pure-Rust-no-FFI decision is recorded in `akar-core/docs/adr/`;
reverting it would invalidate the WASM target, the cross-platform claim, and the
memory-safety premise that consumers rely on.

### II. Spec-Driven Development (NON-NEGOTIABLE)

Work is specified before it is written, and the specification is a living document
rather than a historical artifact. `SPEC.md` at the repository root is the single
source of truth for what Akar is and why: architecture, crate topology, registered
function counts, operator inventory, test totals, performance parity, and release
state all live there.

Therefore:

- `SPEC.md` MUST be read before code is touched, and MUST be updated in the same
  batch as any change that moves a metric, an architecture boundary, or a documented
  behavior.
- Changing behavior without a corresponding `SPEC.md` update is drift, and drift
  fails review. A code change that contradicts `SPEC.md` is by definition wrong
  until one of the two documents is amended.
- `implementation plan.md` MUST contain only work that is not yet done. Finished
  work is removed from the plan entirely — never struck through, never tagged
  `FIXED`/`DONE` — and its history is recorded in `CHANGELOG.md`.
- Every task MUST carry a stable `P###` identifier, a severity, and a
  `file:line` pointer to the code it addresses. Findings that have no task yet are
  recorded in `FINDINGS.md`; findings that do have one MUST be removed from
  `FINDINGS.md` so the two files never disagree.
- Raw audit notes belong in `docs/audits/`, which is local-only and
  git-excluded. They are working material, not project truth.

Rationale: the project has accumulated hundreds of `P###` tasks across many
sessions. The identifier and the reconcile step are the only mechanism that keeps
a long-running spec-driven effort from losing track of what is open.

### III. Test-Gated Delivery (NON-NEGOTIABLE)

No change is complete until the gate proves it. The gate is the `test [akar-core]`
run configuration, and its report header MUST read **0 failed, 0 ignored**.

- The total test count MUST never decrease. Deleting or `#[ignore]`ing a test to
  make a batch pass is forbidden; a test that is obsolete is removed only with an
  explicit replacement or an explicit decision recorded in `CHANGELOG.md`.
- Every bug fix MUST ship with a regression test that fails before the fix and
  passes after it. If a defect cannot be expressed as a test, that fact is stated
  explicitly in the batch's `CHANGELOG.md` entry.
- Committed code MUST be warning-free under CI's contract:
  `RUSTFLAGS="-Dwarnings"`, `cargo clippy --workspace --all-targets -- -D warnings`,
  and `cargo fmt --all -- --check` with `max_width = 120` (edition 2024).
- New or changed Rust source files MUST be reachable from the PSI index
  (`mod name;` declared in the owning `lib.rs`/`main.rs`) so tooling and agents
  can resolve symbols without a full compile.
- Timing-sensitive tests MUST NOT depend on a single machine's speed. Known flaky
  tests (e.g. plan-cache timing) are fixed at the root cause, not re-run until
  green.
- Fuzz targets in `akar-core/fuzz/fuzz_targets/` (5 targets) cover untrusted-input
  surfaces: Cypher parsing, expression evaluation, `COPY FROM` CSV, compression
  round-trips, and WAL bytes.

Rationale: a test total that can shrink is a gate that can be satisfied by
deleting work. Pinning both "zero failures" and "never fewer tests" makes the
number meaningful across sessions.

### IV. Evidence Over Assertion (NON-NEGOTIABLE)

Reported status MUST be observed, never assumed. Tool output that asserts a build
or test outcome does not by itself constitute evidence.

- Test and gate status is authoritative only from the run configuration's exported
  report or an equivalent `cargo test` summary, and the batch's `CHANGELOG.md`
  entry MUST cite the measured numbers plus their date.
- A file's actual content is authoritative only from a direct read or
  `git diff`, not from a search-tool snippet or a summary.
- A symbol or file reported as absent MUST be confirmed absent with a repository
  search before the fact is stated in writing. Fabricated test names, crate
  contents, and pass counts are treated as defects in their own right.
- Metric values (test totals, function counts, crate counts) are copied from
  `SPEC.md` §1.2 and §11.1, and updated there in the same batch that changes
  them. They are never retyped from memory.

Rationale: earlier sessions in this project produced confident reports of passing
gates and of test files that did not exist. The correction is procedural, not
informational: evidence is defined by source, and no amount of confident
prose substitutes for it.

### V. Embedded-Only Domain Boundary

Akar is an embedded library. It starts in a host process, runs in that process's
memory, and shuts down with it. Akar MUST NOT grow a daemon lifecycle, a
long-running server role, a service-discovery surface, or a deployment topology.

Consequently:

- `akar-main` is the public entry point. `Database`, `Connection`, and
  `SystemConfig` are the supported API surface.
- `akar-server` (TCP JSON broker) is deprecated for production use. It is retained
  as a test harness and a wire-format reference only. The production daemon belongs
  to the consumer (`sulur-server`), which embeds `akar-main` in-process; this
  division is recorded as ADR-02 and enforced by
  `sulur/tools/boundary-check.py`.
- Process lifecycle, scheduling, and supervision are the consumer's
  responsibility. Akar MUST NOT infer or impose them.
- Cross-process concerns are limited to documented, crash-safe file primitives:
  atomic WAL writes, file locking, and checkpoint-and-recovery. Durability MUST NOT
  depend on any configurable checkpoint threshold — the default is an
  optimization, never a correctness guarantee.

Rationale: the boundary keeps Akar embeddable in arbitrary hosts, including
language runtimes and sandboxes, and keeps the surface small enough to reason
about across 36+ crates.

## Engineering Constraints

- **Language and edition:** Rust 2024 edition; MSRV 1.80+.
- **Workspace layout:** the repository root holds documentation and governance;
  the Cargo workspace root is `akar-core/`. All `cargo` commands run from
  `akar-core/`. `cargo fmt` and `cargo clippy` are always invoked with
  `--workspace`.
- **Formatting:** `rustfmt.toml` at the workspace root sets `max_width = 120`.
  Formatting is not negotiable per-file.
- **Versioning:** Semantic Versioning 2.0.0. All crates are versioned in
  lockstep, bottom-up along the dependency graph. Breaking a public API during
  `0.x` is a MINOR bump and every publishable crate moves together — never one
  crate alone. A `STORAGE_VERSION` increase in `akar-storage` without automatic
  migration is a breaking change by definition.
- **Publishability:** `akar-c`, `akar-python`, and `akar-fuzz` are
  `publish = false` and MUST NOT be added to a crates.io release wave.
- **Error handling:** all fallible paths return `Result<T, E>` with `?`
  propagation. `panic!()` and `.unwrap()` MUST NOT appear on production code
  paths; use `ok_or_else(...)`, and map lock poisoning with
  `.lock().map_err(...)`. Epsilon comparisons apply to float equality.
- **Concurrency:** multi-writer correctness rests on MVCC with
  optimistic-concurrency row-level conflict detection. A change that introduces
  blocking locks on the write path, or a write lock held across a `Ref` borrow of
  the same map, MUST be rejected — re-entrant `DashMap` access has already caused
  a production flake (P53.14).
- **Licensing:** GPLv3. New files inherit the repository license; no
  copyleft-incompatible dependency may enter the workspace.
- **Cross-repo ordering:** `akar` is released before `sulur`, and the published
  version is verified on PyPI/crates.io before the downstream release proceeds.

## Development Workflow & Quality Gates

The workflow is a fixed cycle, and the order is not negotiable:

1. **Specify** — read `SPEC.md` first; state the problem and the measurable
   success criteria.
2. **Plan** — add the task to `implementation plan.md` as `PLANNED`, with
   `P###`, severity, and `file:line`.
3. **Tasks** — record raw findings in `docs/audits/` or `FINDINGS.md` as
   appropriate.
4. **Implement** — work in small batches, one coherent concern per batch. Each
   batch ends with a gate run.
5. **Reconcile** — after the commit: remove the finished task from the plan,
   record the batch with its task ids, commit hash, and gate numbers in
   `CHANGELOG.md` ([Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
   format, categorized under `### Added` / `### Fixed` / `### Changed` /
   `### Removed`), and update `SPEC.md` metrics. Reconcile is part of the batch,
   not a follow-up.

Quality gates, in increasing cost:

- **Per-file:** `get_file_problems` must report zero errors before commit.
- **Fast pre-commit:** `check [akar-core]` — release-profile `cargo check` with
  all features, which also compiles the native C++ extension surfaces.
- **Release gate:** `test [akar-core]` — 0 failed, 0 ignored, total never
  decreasing.
- **CI:** twelve jobs across Linux, macOS (arm64), Windows, and
  `wasm32-unknown-unknown`, plus `cargo audit`, feature-gated builds, benchmark
  compilation, coverage, and `wasm-pack test`. A green local gate does not
  substitute for CI, and CI does not substitute for the local gate when a
  change touches native-extension or WASM surfaces.

Release is scripted, not manual: `python tools/doc-check.py` validates changelog
format, dependency alignment, and `SPEC.md` metrics, then
`python tools/release.py <version>` runs the gate, bumps versions, finalizes the
changelog, tags, and publishes bottom-up.

Review expectations:

- Pull requests stay small; a batch that touches many crates at once is split
  before review.
- Every pull request states which `P###` tasks it closes and which gate numbers
  were measured.
- Contributors agree to the CLA and the Code of Conduct, and do not push directly
  to the default branch.
- Reviewers verify constitution compliance first and code style second.

## Governance

This constitution supersedes all other project practices, including conventions
recorded in `AGENTS.md` and defaults inherited from templates. Where a document,
comment, or habit conflicts with this file, this file wins and the conflicting
document is corrected in the same batch.

**Amendment procedure.** Amendments are made by editing
`.specify/memory/constitution.md` — never by editing a versioned template layer in
`.specify/templates/`. An amendment MUST: (a) state which principles are added,
modified, or removed; (b) increment the version per the policy below; (c) carry a
`Last Amended` date in `YYYY-MM-DD` form; and (d) be confirmed by the project owner
before commit. Amendments that remove or redefine a principle MUST include a
migration plan for work already in flight. `Ratified` records the original
adoption date and MUST NOT change on amendment.

**Constitution versioning.** The version follows Semantic Versioning 2.0.0:

- **MAJOR** — a principle is removed, or an existing principle is redefined in a
  way that invalidates previously accepted work.
- **MINOR** — a principle or section is added, or existing guidance is materially
  expanded.
- **PATCH** — clarification, wording, typo, or formatting change with no
  semantic effect.

Where the bump type is ambiguous, the amendment MUST justify the chosen type in
its sync impact report before it is finalized.

**Compliance review.** Every pull request and every batch review MUST verify
compliance with the five core principles explicitly, not implicitly. Complexity
that cannot be justified against a named principle MUST be cut. The `Next
Actions` section of `implementation plan.md` is the queue of work whose
acceptance criteria are derived from these principles, and reviewing that queue
against this file is how drift in the *process* is caught.

**Deviations.** A deviation from any MUST is permitted only when the pull request
records: the principle deviated from, the reason, the scope, and the plan to
restore compliance. Undocumented deviations are defects.

**Version**: 1.0.0 | **Ratified**: 2026-09-28 | **Last Amended**: 2026-09-28
