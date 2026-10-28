#!/usr/bin/env python3
"""doc-check.py — Akar governance document validator.

Checks:
  1. Version consistency across SPEC.md, README.md, AGENTS.md, CHANGELOG.md
  2. Test count consistency across the same files
  3. Finding-reference resolution (every P### in FINDINGS.md resolves to a task)

Usage:
  python tools/doc-check.py                  # Run all checks
  python tools/doc-check.py --check-version  # Version check only
  python tools/doc-check.py --check-test-count
  python tools/doc-check.py --check-findings

Exit 0 = pass, 1 = fail (with named mismatch).
"""

import argparse
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

DOCS = ["SPEC.md", "README.md", "AGENTS.md", "CHANGELOG.md"]
FINDINGS = "FINDINGS.md"
PLAN = "implementation plan.md"


def read(path):
    full = os.path.join(ROOT, path)
    if not os.path.exists(full):
        return None
    with open(full, "r", encoding="utf-8") as f:
        return f.read()


def check_version():
    versions = {}
    for doc in DOCS:
        content = read(doc)
        if content is None:
            continue
        patterns = [
            r'Current version:\*\*\s*`(\d+\.\d+\.\d+)`',
            r'\*\*Current version:\*\*\s*`(\d+\.\d+\.\d+)`',
            r'version\s*[:=]\s*`(\d+\.\d+\.\d+)`',
            r'## \[(\d+\.\d+\.\d+)\]',
        ]
        for pat in patterns:
            m = re.search(pat, content)
            if m:
                versions[doc] = m.group(1)
                break
    if not versions:
        print("VERSION CHECK: SKIP (no version found in any doc)")
        return 0
    unique = set(versions.values())
    if len(unique) == 1:
        print(f"VERSION CHECK: PASS (all = {unique.pop()})")
        return 0
    print("VERSION CHECK: FAIL")
    for doc, ver in versions.items():
        print(f"  {doc}: {ver}")
    return 1


TEST_COUNT_DOCS = ["SPEC.md", "README.md", "AGENTS.md"]


def check_test_count():
    counts = {}
    for doc in TEST_COUNT_DOCS:
        content = read(doc)
        if content is None:
            continue
        patterns = [
            r'\*\*([\d,]+)\*\* total,\s*0\s*ignored',
            r'\*\*([\d,]+)\s*tests,\s*0\s*failed',
            r'([\d,]+)\s*total\s*/\s*0\s*ignored',
            r'tests\s*passing\s*\|\s*\*\*([\d,]+)\*\*',
        ]
        for pat in patterns:
            matches = re.findall(pat, content)
            if matches:
                counts[doc] = matches[0].replace(",", "")
                break
    if not counts:
        print("TEST COUNT CHECK: SKIP (no test count found)")
        return 0
    unique = set(counts.values())
    if len(unique) == 1:
        print(f"TEST COUNT CHECK: PASS (all = {unique.pop()})")
        return 0
    print("TEST COUNT CHECK: FAIL")
    for doc, cnt in counts.items():
        print(f"  {doc}: {cnt}")
    return 1


def check_findings():
    findings = read(FINDINGS)
    if findings is None:
        print("FINDINGS CHECK: SKIP (FINDINGS.md not found)")
        return 0
    plan = read(PLAN)
    changelog = read("CHANGELOG.md")
    plan_text = (plan or "") + "\n" + (changelog or "")
    unresolved = []
    for m in re.finditer(r'P\d{3,4}', findings):
        pid = m.group(0)
        if pid not in plan_text:
            unresolved.append(pid)
    if not unresolved:
        print("FINDINGS CHECK: PASS (all P### references resolve)")
        return 0
    print("FINDINGS CHECK: FAIL")
    for pid in set(unresolved):
        print(f"  {pid} not found in plan or changelog")
    return 1


def check_safety_compliance():
    manifest = os.path.join(ROOT, "akar-core", "Cargo.toml")
    if not os.path.exists(manifest):
        print("SAFETY COMPLIANCE: SKIP (akar-core/Cargo.toml not found)")
        return 0
    import subprocess
    cmd = [
        "cargo", "clippy",
        "--manifest-path", manifest,
        "--lib",
        "--",
        "-D", "clippy::unwrap_used",
        "-D", "clippy::expect_used",
        "-D", "clippy::undocumented_unsafe_blocks",
    ]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if proc.returncode != 0:
            print("SAFETY COMPLIANCE: FAIL (violations found by clippy)")
            return 1
        print("SAFETY COMPLIANCE: PASS (0 unwraps, 0 expects, 100% documented unsafe)")
        return 0
    except FileNotFoundError:
        print("SAFETY COMPLIANCE: WARN (cargo not found)")
        return 0


def main():
    parser = argparse.ArgumentParser(description="Akar governance document validator")
    parser.add_argument("--check-version", action="store_true")
    parser.add_argument("--check-test-count", action="store_true")
    parser.add_argument("--check-findings", action="store_true")
    parser.add_argument("--check-safety", action="store_true")
    args = parser.parse_args()
    run_all = not (args.check_version or args.check_test_count or args.check_findings or args.check_safety)
    rc = 0
    if run_all or args.check_version:
        rc = rc or check_version()
    if run_all or args.check_test_count:
        rc = rc or check_test_count()
    if run_all or args.check_findings:
        rc = rc or check_findings()
    if run_all or args.check_safety:
        rc = rc or check_safety_compliance()
    sys.exit(rc)


if __name__ == "__main__":
    main()
