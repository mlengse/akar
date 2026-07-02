---
name: run-kuzu-cli
description: Build, run, and drive kuzu-cli — the Cypher REPL for the kuzu-core Rust workspace. Use when asked to start kuzu-cli, run a Cypher query against the engine, smoke-test the query pipeline (parse/bind/plan/optimize/execute), or verify a change to kuzu-binder/kuzu-parser/kuzu-planner/kuzu-processor/kuzu-optimizer actually works end to end.
---

`kuzu-cli` is the interactive Cypher shell (rustyline REPL) for the
`kuzu-core` Rust reimplementation of KuzuDB — it's the one runnable
binary in this ~27-crate workspace and the fastest way to exercise the
full query pipeline across any of the engine crates (parser, binder,
planner, optimizer, processor). Drive it via
`.claude/skills/run-kuzu-cli/driver.mjs`, which pipes a Cypher/dot-command
script to the binary and returns its formatted output — **no tmux or
pty emulation needed** (see Gotchas for why piped stdin is enough).

All paths below are relative to `kuzu-core/` (this workspace root).

## Prerequisites

Rust 1.87+ (edition 2024) and Node.js (any recent version, only used to
run the driver script). Nothing else — no system packages were needed
to build this on Windows; it's pure Rust with no native/C dependencies
in the `kuzu-cli` binary's dependency tree.

## Build

```bash
cargo build --bin kuzu-cli
```

Works from `kuzu-core/` or from inside any crate directory (e.g.
`kuzu-core/kuzu-cli/`) — Cargo walks up to find the workspace root.
Produces `target/debug/kuzu-cli(.exe)`. First build compiles the whole
dependency graph (~1–2 min); incremental builds are fast.

## Run (agent path)

```bash
node .claude/skills/run-kuzu-cli/driver.mjs --smoke
```

This builds the binary if missing, feeds it a canned script (create
table, insert rows, `.tables`, `.schema`, a property query, `.mode
json`, a literal `RETURN`), and asserts on the output. Expected: four
`PASS` lines on stderr, exit code 0.

To run your own script:

```bash
node .claude/skills/run-kuzu-cli/driver.mjs path/to/script.cql        # in-memory DB
node .claude/skills/run-kuzu-cli/driver.mjs path/to/script.cql /tmp/db  # persistent path (see Gotchas)
echo 'MATCH (n) RETURN n;' | node .claude/skills/run-kuzu-cli/driver.mjs -
```

Script format: one statement per line, each ending in `;`, plus dot-commands
(`.tables`, `.schema`, `.mode <table|csv|json|line|column>`, `.import
<file> <table>`, `.export <file> <query>`, `.help`, `.exit`). End the
script with `.exit` (not required — EOF also exits cleanly — but it
makes intent explicit and flushes history).

Verified output for the smoke script (this is real output from this
session, not the expected/hoped-for shape):

```
Kuzu CLI v0.1.0
Enter queries (end with ;). Type .help

(empty)
Node table 'Person' created
...
  Person
TABLE Person (NODE)
  name: String PK
  age: Int64
+-----+
| col_0 |
+-----+
| <val> |
| <val> |
+-----+
(2 rows)
Mode set to Json
[
  {"col_0": "1", "col_1": "2", "col_2": "3"}
]
Bye!
```

## Run (human path)

```bash
cargo run --bin kuzu-cli               # in-memory
cargo run --bin kuzu-cli -- /path/to/db  # persistent (see Gotchas: doesn't actually persist yet)
```

Opens the same rustyline REPL with real line history (↑↓) and tab
completion for keywords/table names. `Ctrl-D` or `.exit` to quit.

## Test

```bash
cargo test -p kuzu-parser      # fast (~5s), verified: 53 passed
cargo test --workspace         # full suite — did not finish in 90s in this session; scope to the crate(s) you touched instead
```

Prefer `cargo test -p <crate-you-changed>` over `--workspace` unless
you specifically need cross-crate integration coverage — the full
workspace suite is slow enough that it timed out in this session's
90-second budget.

## Gotchas

- **No tmux/pty needed — the REPL's tty-detection is env-var based, not
  a real `isatty()` check.** `main.rs`'s `atty_check()` looks at `TERM`
  and `CI`, not whether stdin is an actual terminal. Piping a script in
  with `TERM=xterm` (and `CI` unset) gets you the fully-formatted
  interactive REPL — banner, tables, `.dot-command` output, error
  text — over a plain pipe. The driver script does this for you; you
  don't need `tmux send-keys`/`capture-pane` at all for this binary.
- **`TERM=dumb` (or `CI` set) silently switches to script mode, which
  prints almost nothing.** In script mode (`run_script` in `main.rs`),
  only error lines are printed — successful `CREATE`/`MATCH` output is
  completely silent. If you pipe a script and see no output at all,
  you're in script mode, not a hung process. Always force
  `TERM=xterm` (the driver does this) if you want to see results.
- **⚠️ `MATCH (p:T) RETURN p.<property>` currently returns the node's
  primary-key value for *any* requested property, not the actual bound
  property.** Verified: for `T3(id INT64, score INT64, PRIMARY
  KEY(id))` with rows `(id:55, score:999)` and `(id:56, score:111)`,
  `MATCH (t:T3) RETURN t.score` returns `55` and `56` (the PK values),
  not `999`/`111`. This reproduces regardless of which non-PK property
  is named. This is a known gap in property projection somewhere in
  the binder/planner/processor pipeline (this is a WIP Rust port —
  see `KONSOLIDASI_DOKUMEN.md` / `CONSOLIDATED_PLAN.md` for the
  feature-gap tracking), not something the driver or your local build
  broke. Don't lose time assuming a regression here unless you're
  specifically working on property projection.
- **`fmt_val` in `kuzu-cli/src/main.rs` only formats `Int64/Int32/Int16`,
  `Double/Float`, and `Bool` physical types — everything else (String
  columns included, when they ARE correctly bound) renders as the
  literal placeholder `<val>`.** Multi-column `RETURN` of *literals*
  (e.g. `RETURN 1, 2, 3;`) works fine and shows real values per column —
  it's specifically node/property-bound results that are affected,
  compounding with the PK-substitution bug above.
- **Persistent mode (`kuzu-cli /some/dir`) does not currently persist
  data across process restarts.** Verified: creating a table + row
  against a directory path writes only a near-empty `wal.log`
  (1 byte); reopening the same directory in a fresh process shows
  `.tables` → "No tables." and the catalog is empty. Use `:memory:`
  (the default, no path arg) unless you're specifically testing/fixing
  storage recovery.
- **On Windows/Git Bash, don't embed `/tmp/...`-style paths inside a
  piped script for `.import`/`.export`.** MSYS path translation only
  rewrites path-looking arguments on the command line, not strings
  embedded in piped stdin data — a `.import /tmp/x.csv T` line reaches
  the Rust binary as the literal string `/tmp/x.csv`, which doesn't
  resolve to a real file from a native Windows process, and you get
  `Bind error: File '/tmp/x.csv' not found`. Use a path relative to the
  CLI's working directory instead (e.g. `.import x.csv T` after `cd`-ing
  next to the file) — verified working.

## Troubleshooting

No build or launch errors were hit in this session — `cargo build
--bin kuzu-cli` and `cargo run --bin kuzu-cli` both worked cleanly on
first try (only pre-existing `dead_code`/`unused` warnings in
`kuzu-graph`/`kuzu-optimizer`/`kuzu-processor`, not failures).
