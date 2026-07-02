#!/usr/bin/env node
// Driver for kuzu-cli — feeds a Cypher/dot-command script to the REPL binary
// and prints its stdout. See SKILL.md for why this works without a pty/tmux.
//
// Usage:
//   node driver.mjs <script.cql> [db_path]      # run a script file
//   node driver.mjs --smoke [db_path]           # run the built-in smoke test
//   echo 'RETURN 1;' | node driver.mjs -        # read script from stdin
//
// db_path defaults to ":memory:". Pass a directory path for persistent mode
// (see Gotchas in SKILL.md — persistence across process restarts is currently
// a no-op in this WIP engine, so ":memory:" is almost always what you want).

import { spawnSync } from "node:child_process";
import { readFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
// .claude/skills/run-kuzu-cli/driver.mjs -> kuzu-core/ is three levels up
const workspaceRoot = path.resolve(here, "..", "..", "..");
const binCandidates = [
  path.join(workspaceRoot, "target", "debug", "kuzu-cli.exe"),
  path.join(workspaceRoot, "target", "debug", "kuzu-cli"),
];

const SMOKE_SCRIPT = `CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name));
CREATE (:Person {name: 'Alice', age: 30});
CREATE (:Person {name: 'Bob', age: 25});
.tables
.schema
MATCH (p:Person) RETURN p.age;
.mode json
RETURN 1, 2, 3;
.exit
`;

function findBinary() {
  for (const p of binCandidates) if (existsSync(p)) return p;
  return null;
}

function build() {
  console.error("[driver] building kuzu-cli (debug)...");
  const r = spawnSync("cargo", ["build", "--bin", "kuzu-cli"], {
    cwd: workspaceRoot,
    stdio: "inherit",
  });
  if (r.status !== 0) {
    console.error("[driver] build failed");
    process.exit(1);
  }
}

function runScript(scriptText, dbPath) {
  const bin = findBinary() ?? (build(), findBinary());
  if (!bin) {
    console.error("[driver] could not find or build kuzu-cli binary");
    process.exit(1);
  }
  // TERM must be set to something other than "dumb", and this must be
  // true regardless of whether stdin is a real tty. See SKILL.md Gotchas:
  // the binary's tty-detection is env-var based, not a real isatty() check,
  // so piping a script in with TERM=xterm gets you the fully-formatted
  // interactive REPL (tables, .dot-command output, error text) with no
  // pty/tmux required.
  const r = spawnSync(bin, [dbPath ?? ":memory:"], {
    input: scriptText,
    encoding: "utf8",
    env: { ...process.env, TERM: "xterm" },
  });
  process.stdout.write(r.stdout ?? "");
  if (r.stderr) process.stderr.write(r.stderr);
  return r.stdout ?? "";
}

const args = process.argv.slice(2);
if (args[0] === "--smoke") {
  const out = runScript(SMOKE_SCRIPT, args[1]);
  const checks = [
    ["table created", /Node table 'Person' created/],
    ["tables listed", /Person/],
    ["schema shown", /TABLE Person \(NODE\)/],
    ["json mode literals", /"col_0": "1"/],
  ];
  let ok = true;
  for (const [label, re] of checks) {
    const pass = re.test(out);
    console.error(`[driver] ${pass ? "PASS" : "FAIL"}: ${label}`);
    if (!pass) ok = false;
  }
  process.exit(ok ? 0 : 1);
} else if (args[0] === "-") {
  const script = readFileSync(0, "utf8");
  runScript(script, args[1]);
} else if (args[0]) {
  const script = readFileSync(args[0], "utf8");
  runScript(script, args[1]);
} else {
  console.error(__filename_usage());
  process.exit(1);
}

function __filename_usage() {
  return `Usage:
  node driver.mjs <script.cql> [db_path]
  node driver.mjs --smoke [db_path]
  echo 'RETURN 1;' | node driver.mjs -`;
}
