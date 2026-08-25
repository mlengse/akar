#!/usr/bin/env python3
"""
akar release pipeline — gate → version bump → dep align → commit → tag →
crates.io publish (bottom-up) → GitHub release.

Usage:
    python tools/release.py 0.1.13          # full release
    python tools/release.py 0.1.13 --dry    # gate + version bump only
    python tools/release.py 0.1.13 --skip-gate   # skip tests (use with caution)
    python tools/release.py --publish-only 0.1.13  # only crates.io publish

Prerequisites:
    - cargo login <token>  (or CARGO_REGISTRY_TOKEN env var)
    - gh auth login        (for GitHub release creation)
    - Clean working tree (or --allow-dirty)
"""

from __future__ import annotations

import argparse
import datetime
import os
import re
import subprocess
import sys
import tomllib  # Python 3.11+ built-in
from pathlib import Path

AKAR_ROOT = Path(__file__).resolve().parent.parent
CARGO_ROOT = AKAR_ROOT / "akar-core"
WORKSPACE_TOML = CARGO_ROOT / "Cargo.toml"
CHANGELOG = AKAR_ROOT / "CHANGELOG.md"
SPEC = AKAR_ROOT / "SPEC.md"

# ── crates.io publish order (bottom-up per dep graph) ──────────────
# Core crates: must be published in this exact order.
# Extension crates: can be published in any order AFTER their core deps.
PUBLISH_ORDER_CORE = [
    "akar-common",
    "akar-storage",
    "akar-transaction",
    "akar-function",
    "akar-parser",
    "akar-catalog",
    "akar-binder",
    "akar-planner",
    "akar-optimizer",
    "akar-processor",
    "akar-graph",
    "akar-extension",
    "akar-main",
    "akar-cli",
]

PUBLISH_ORDER_EXTENSIONS = [
    "akar-json",
    "akar-fts",
    "akar-vector",
    "akar-httpfs",
    "akar-duckdb",
    "akar-algo",
    "akar-neo4j",
    "akar-llm",
    "akar-sqlite",
    "akar-delta",
    "akar-iceberg",
    "akar-azure",
    "akar-postgres",
    "akar-unity-catalog",
    "akar-ml",
    "akar-dream",
    "akar-search",
    "akar-migrate",
    "akar-wasm",
    "akar-server",
]

# Skip these crates entirely (not on crates.io)
SKIP_CRATES = {"akar-c", "akar-python", "akar-fuzz"}

PUBLISH_ORDER = PUBLISH_ORDER_CORE + PUBLISH_ORDER_EXTENSIONS

RATE_LIMIT_SECONDS = 8  # crates.io rate limit buffer


def run(
    cmd: list[str],
    *,
    cwd: Path | None = None,
    check: bool = True,
    capture: bool = False,
    env: dict | None = None,
) -> subprocess.CompletedProcess:
    merged_env = {**os.environ, **(env or {})}
    result = subprocess.run(
        cmd,
        cwd=cwd or AKAR_ROOT,
        check=False,
        capture_output=capture,
        text=True,
        env=merged_env,
    )
    if check and result.returncode != 0:
        print(f"FAIL: {' '.join(cmd)}")
        if capture:
            print(result.stdout[-2000:] if result.stdout else "")
            print(result.stderr[-2000:] if result.stderr else "")
        sys.exit(1)
    return result


def git_clean() -> bool:
    result = run(["git", "status", "--porcelain"], capture=True)
    return result.stdout.strip() == ""


def git_tag_exists(tag: str) -> bool:
    result = run(["git", "tag", "-l", tag], capture=True)
    return tag in result.stdout


def parse_workspace_version() -> str:
    with open(WORKSPACE_TOML, "rb") as f:
        data = tomllib.load(f)
    return data["workspace"]["package"]["version"]


def parse_crate_version(crate_name: str) -> str | None:
    """Get version of a specific crate (workspace or explicit)."""
    crate_dir = CARGO_ROOT / crate_name
    if not crate_dir.exists():
        return None
    cargo_toml = crate_dir / "Cargo.toml"
    with open(cargo_toml, "rb") as f:
        data = tomllib.load(f)
    pkg = data.get("package", {})
    ver = pkg.get("version")
    if isinstance(ver, str):
        return ver
    # version.workspace = true → use workspace version
    return parse_workspace_version()


def set_workspace_version(version: str) -> None:
    text = WORKSPACE_TOML.read_text(encoding="utf-8")
    # Replace [workspace.package] version = "x.y.z"
    text = re.sub(
        r'(\[workspace\.package\]\s*version\s*=\s*")([^"]*)(")',
        rf"\g<1>{version}\3",
        text,
    )
    WORKSPACE_TOML.write_text(text, encoding="utf-8")
    print(f"  workspace version → {version}")


def set_crate_version(crate_name: str, version: str) -> None:
    """Set version for a crate that has version.workspace = false."""
    cargo_toml = CARGO_ROOT / crate_name / "Cargo.toml"
    if not cargo_toml.exists():
        return
    text = cargo_toml.read_text(encoding="utf-8")
    # Match version = "x.y.z" inside [package]
    text = re.sub(
        r'(^version\s*=\s*")[^"]*(")',
        rf"\g<1>{version}\2",
        text,
        count=1,
        flags=re.MULTILINE,
    )
    cargo_toml.write_text(text, encoding="utf-8")


def align_dep_versions(version: str) -> None:
    """Ensure all akar-* deps in workspace crates match the target version."""
    for cargo_toml in CARGO_ROOT.rglob("Cargo.toml"):
        if cargo_toml == WORKSPACE_TOML:
            continue
        text = cargo_toml.read_text(encoding="utf-8")
        original = text
        # Update akar-* version specs: version = "0.1.x" → version = "0.1.y"
        text = re.sub(
            r'(akar-[a-z0-9_-]+)\s*=\s*\{\s*version\s*=\s*"[^"]*"',
            rf'\1 = {{version = "{version}"',
            text,
        )
        # Also handle simple: akar-xxx = "0.1.x"
        text = re.sub(
            r'^(akar-[a-z0-9_-]+)\s*=\s*"[^"]*"',
            rf'\1 = "{version}"',
            text,
            flags=re.MULTILINE,
        )
        if text != original:
            cargo_toml.write_text(text, encoding="utf-8")
            print(f"  deps aligned in {cargo_toml.parent.name}")


def check_dep_alignment() -> list[str]:
    """Verify all akar-* deps match workspace version. Returns list of issues."""
    ws_ver = parse_workspace_version()
    issues = []
    for cargo_toml in CARGO_ROOT.rglob("Cargo.toml"):
        if cargo_toml == WORKSPACE_TOML:
            continue
        text = cargo_toml.read_text(encoding="utf-8")
        for m in re.finditer(
            r'(akar-[a-z0-9_-]+)\s*=\s*\{\s*version\s*=\s*"([^"]*)"',
            text,
        ):
            dep_name, dep_ver = m.group(1), m.group(2)
            if dep_ver != ws_ver:
                issues.append(
                    f"  {cargo_toml.parent.name}: {dep_name} has version "
                    f'"{dep_ver}" (expected "{ws_ver}")'
                )
    return issues


# ── gates ───────────────────────────────────────────────────────────
def gate_fmt() -> bool:
    print("[gate] cargo fmt --check ...")
    result = run(
        ["cargo", "fmt", "--all", "--", "--check"],
        cwd=CARGO_ROOT,
        check=False,
        capture=True,
    )
    if result.returncode != 0:
        print("FAIL: formatting issues. Run `cargo fmt` first.")
        print(result.stdout[-1000:] if result.stdout else "")
        return False
    print("  PASS")
    return True


def gate_clippy() -> bool:
    print("[gate] cargo clippy ...")
    result = run(
        [
            "cargo", "clippy", "--workspace", "--all-targets",
            "--", "-D", "warnings",
        ],
        cwd=CARGO_ROOT,
        check=False,
        capture=True,
        env={"RUSTFLAGS": "-Dwarnings"},
    )
    if result.returncode != 0:
        print("FAIL: clippy warnings/errors.")
        print(result.stdout[-2000:] if result.stdout else "")
        print(result.stderr[-2000:] if result.stderr else "")
        return False
    print("  PASS")
    return True


def gate_test() -> bool:
    print("[gate] cargo test --workspace ...")
    result = run(
        ["cargo", "test", "--workspace", "--no-fail-fast"],
        cwd=CARGO_ROOT,
        check=False,
        capture=True,
    )
    if result.returncode != 0:
        print("FAIL: test failures.")
        print(result.stdout[-3000:] if result.stdout else "")
        return False
    # Parse summary line
    output = result.stdout
    for line in output.splitlines():
        if "test result:" in line:
            print(f"  {line.strip()}")
            if "0 failed" not in line:
                print("FAIL: non-zero test failures.")
                return False
            break
    print("  PASS")
    return True


# ── changelog ───────────────────────────────────────────────────────
def finalize_changelog(version: str, tag: str) -> None:
    """Move [Unreleased] content to [version] section."""
    text = CHANGELOG.read_text(encoding="utf-8")
    today = datetime.date.today().isoformat()

    # Find [Unreleased] section
    unreleased_match = re.search(
        r"## \[Unreleased\]\s*\n(.*?)(?=\n## \[|\Z)",
        text,
        re.DOTALL,
    )
    if not unreleased_match:
        print("WARN: no [Unreleased] section found in CHANGELOG.md")
        return

    unreleased_content = unreleased_match.group(1).strip()
    if not unreleased_content:
        print("WARN: [Unreleased] section is empty")
        return

    # Find the previous version tag for comparison link
    prev_version = None
    for m in re.finditer(r"## \[(\d+\.\d+\.\d+)\]", text):
        prev_version = m.group(1)

    # Build new section
    new_section = f"## [{version}] - {today}\n\n{unreleased_content}\n"

    # Replace [Unreleased] with [Unreleased] (empty) + new version section
    old_unreleased = unreleased_match.group(0)
    text = text.replace(
        old_unreleased,
        f"## [Unreleased]\n\n{new_section}",
    )

    # Add comparison link at bottom
    repo_url = "https://github.com/mlengse/akar"
    link = f"[{version}]: {repo_url}/compare/v{prev_version}...{tag}\n"
    if link not in text:
        # Find footer section or append
        if prev_version and f"[{prev_version}]:" in text:
            # Insert after last comparison link
            last_link_pos = text.rfind(f"[{prev_version}]:")
            end_of_line = text.index("\n", last_link_pos) + 1
            text = text[:end_of_line] + link + text[end_of_line:]
        else:
            text += f"\n{link}"

    CHANGELOG.write_text(text, encoding="utf-8")
    print(f"  CHANGELOG: [Unreleased] → [{version}] - {today}")


def commit_and_tag(version: str, tag: str) -> None:
    """Commit all changes and create annotated tag."""
    # Stage everything
    run(["git", "add", "-A"])

    # Check if there's anything to commit
    result = run(["git", "diff", "--cached", "--stat"], capture=True)
    if not result.stdout.strip():
        print("  nothing to commit")
        return

    # Write commit message to file
    msg_file = Path(os.environ.get("TEMP", "/tmp")) / "akar_release_msg.txt"
    msg_file.write_text(
        f"release: v{version}\n\n"
        f"Bump version to {version} across workspace.\n"
        f"Update CHANGELOG.md with release entries.\n",
        encoding="utf-8",
    )

    run(["git", "commit", "-F", str(msg_file)])
    print(f"  committed: release v{version}")

    run(["git", "tag", "-a", tag, "-m", f"v{version}"])
    print(f"  tagged: {tag}")


def push(tag: str) -> None:
    run(["git", "push", "origin", "main"])
    run(["git", "push", "origin", tag])
    print(f"  pushed: main + {tag}")


def publish_crate(crate_name: str, version: str) -> bool:
    """Publish a single crate to crates.io. Returns True on success."""
    crate_dir = CARGO_ROOT / crate_name
    if not crate_dir.exists():
        print(f"  SKIP {crate_name}: directory not found")
        return True

    cargo_toml = crate_dir / "Cargo.toml"
    with open(cargo_toml, "rb") as f:
        data = tomllib.load(f)
    publish = data.get("package", {}).get("publish", True)
    if publish is False:
        print(f"  SKIP {crate_name}: publish = false")
        return True

    crate_ver = parse_crate_version(crate_name)
    if crate_ver != version:
        print(f"  SKIP {crate_name}: version is {crate_ver}, expected {version}")
        return False

    print(f"  publishing {crate_name}@{version} ...")
    result = run(
        ["cargo", "publish", "--allow-dirty"],
        cwd=crate_dir,
        check=False,
        capture=True,
    )
    if result.returncode != 0:
        output = (result.stdout or "") + (result.stderr or "")
        if "crate version .* is already uploaded" in output or "already uploaded" in output:
            print(f"    already on crates.io, skipping")
            return True
        print(f"    FAIL: {output[-1000:]}")
        return False
    print(f"    OK")
    return True


def publish_all(version: str, dry: bool = False) -> bool:
    """Publish all crates bottom-up. Returns True on success."""
    print(f"\n[publish] crates.io bottom-up (v{version})")
    issues = check_dep_alignment()
    if issues:
        print("FAIL: dep alignment issues:")
        for issue in issues:
            print(issue)
        return False

    published = []
    for crate in PUBLISH_ORDER:
        if crate in SKIP_CRATES:
            print(f"  SKIP {crate}: excluded")
            continue
        if dry:
            print(f"  DRY: would publish {crate}@{version}")
            continue
        if not publish_crate(crate, version):
            print(f"FAIL: {crate} publish failed. Aborting.")
            print("  Already published (safe to re-run):")
            for p in published:
                print(f"    {p}")
            return False
        published.append(crate)
        # Rate limit: crates.io allows burst of ~30 then ~1/min
        if len(published) > 10:
            import time
            print(f"    waiting {RATE_LIMIT_SECONDS}s (rate limit) ...")
            time.sleep(RATE_LIMIT_SECONDS)

    print(f"\n  all {len(published)} crates published successfully")
    return True


def create_github_release(version: str, tag: str) -> None:
    """Create GitHub release via gh CLI (triggers rust-release.yml workflow)."""
    print(f"\n[release] GitHub release {tag}")
    result = run(
        [
            "gh", "release", "create", tag,
            "--title", f"v{version}",
            "--generate-notes",
        ],
        check=False,
        capture=True,
    )
    if result.returncode != 0:
        print(f"  WARN: gh release create failed: {result.stderr[-500:]}")
        print("  You can create it manually at:")
        print(f"    https://github.com/mlengse/akar/releases/new?tag={tag}")
    else:
        print(f"  GitHub release created: {result.stdout.strip()}")


# ── main ────────────────────────────────────────────────────────────
def main() -> None:
    parser = argparse.ArgumentParser(
        description="akar release pipeline",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("version", nargs="?", help="version to release (e.g. 0.1.13)")
    parser.add_argument("--dry", action="store_true", help="gate + version bump only, no publish")
    parser.add_argument("--skip-gate", action="store_true", help="skip fmt/clippy/test gate")
    parser.add_argument("--skip-deps", action="store_true", help="skip dep alignment")
    parser.add_argument("--allow-dirty", action="store_true", help="proceed with dirty tree")
    parser.add_argument("--publish-only", action="store_true", help="only run crates.io publish")
    parser.add_argument("--skip-github", action="store_true", help="skip GitHub release creation")
    args = parser.parse_args()

    if not args.version and not args.publish_only:
        parser.error("version is required (e.g. python tools/release.py 0.1.13)")

    version = args.version or input("Version to publish: ").strip()
    tag = f"v{version}"
    semver_re = re.compile(r"^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?$")
    if not semver_re.match(version):
        print(f"FAIL: invalid semver: {version}")
        sys.exit(1)

    os.chdir(AKAR_ROOT)
    ws_ver = parse_workspace_version()

    print(f"=== akar release v{version} (workspace: {ws_ver}) ===\n")

    # ── publish-only mode ──
    if args.publish_only:
        ok = publish_all(version)
        sys.exit(0 if ok else 1)

    # ── pre-flight ──
    if not args.allow_dirty and not git_clean():
        print("FAIL: working tree not clean. Use --allow-dirty to proceed.")
        sys.exit(1)

    if git_tag_exists(tag):
        print(f"FAIL: tag {tag} already exists.")
        sys.exit(1)

    # ── gate ──
    if not args.skip_gate:
        print("=== GATE ===\n")
        gates = [gate_fmt(), gate_clippy(), gate_test()]
        if not all(gates):
            print("\nFAIL: gate did not pass. Aborting release.")
            sys.exit(1)
        print()

    # ── version bump ──
    print("=== VERSION BUMP ===\n")
    if version != ws_ver:
        set_workspace_version(version)
    else:
        print(f"  workspace version already {version}")

    # ── dep alignment ──
    if not args.skip_deps:
        print("\n=== DEP ALIGNMENT ===\n")
        align_dep_versions(version)
        issues = check_dep_alignment()
        if issues:
            print("WARN: dep alignment issues found (fixing):")
            for issue in issues:
                print(issue)
        else:
            print("  all deps aligned")

    # ── changelog ──
    print("\n=== CHANGELOG ===\n")
    finalize_changelog(version, tag)

    # ── commit + tag ──
    print("\n=== COMMIT & TAG ===\n")
    commit_and_tag(version, tag)

    # ── push ──
    print("\n=== PUSH ===\n")
    push(tag)

    # ── crates.io publish ──
    if not args.dry:
        ok = publish_all(version)
        if not ok:
            print("\nWARN: some crates failed to publish. Fix manually.")
            sys.exit(1)
    else:
        print("\n=== DRY RUN: skipping crates.io publish ===")

    # ── GitHub release ──
    if not args.skip_github and not args.dry:
        create_github_release(version, tag)

    print(f"\n=== DONE: v{version} ===")
    print(f"  Tag: {tag}")
    print(f"  crates.io: https://crates.io/crates/akar-main/{version}")
    print(f"  GitHub: https://github.com/mlengse/akar/releases/tag/{tag}")


if __name__ == "__main__":
    main()
