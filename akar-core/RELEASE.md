# Release Process — Akar

This document describes how to cut a release of Akar (`akar-core/`).

> **crates.io publishing is active (since 2026-08-08, Sprint 18/P50).** All 33 publishable
> crates are live on crates.io at `0.3.x` (bottom-up, dependency-ordered). Latest release
> **v0.3.4** (2026-11-05); latest GitHub Releases carry prebuilt CLI binaries for
> Linux/macOS/Windows.

---

## Version Numbering

We follow [Semantic Versioning 2.0.0](https://semver.org/) with the `MAJOR.MINOR.PATCH` format:

- **MAJOR**: Breaking changes to the public API.
- **MINOR**: New features, non-breaking additions.
- **PATCH**: Bug fixes, performance improvements, documentation.

Pre-release versions use suffixes like `-alpha.1`, `-beta.2`, `-rc.1`.

The current version is tracked in `akar-core/Cargo.toml` under `[workspace.package] version`.

> **Note (2026-11-14):** beberapa crate telah dikonversi dari `version.workspace = true` ke
> versi eksplisit sendiri (akar-function, akar-json, akar-llm, akar-ml, akar-neo4j,
> akar-search, dst.) agar bisa di-bump independen. Untuk crate tersebut,
> bump versi langsung di `Cargo.toml` masing-masing, bukan lewat `[workspace.package]`.

---

## What Gets Released

| Asset | Platform | Source |
|-------|----------|--------|
| `akar-cli-linux-amd64` | Linux (x86_64) | `cargo build --release -p akar-cli` |
| `akar-cli-macos-arm64` | macOS (Apple Silicon) | `cross` build via GitHub Actions |
| `akar-cli-windows-amd64.exe` | Windows (x86_64) | `cargo build --release -p akar-cli` |

All assets are built and attached to the GitHub Release automatically by the `rust-release.yml` workflow.

---

## Steps to Cut a Release

### 1. Prepare the release branch

```bash
# From the main branch, ensure everything is up to date
git checkout main
git pull origin main

# Create a release branch
git checkout -b release/vX.Y.Z
```

### 2. Update version

Update the version in `akar-core/Cargo.toml`:

```toml
[workspace.package]
version = "X.Y.Z"  # Change to target version
```

Update any references to the old version in documentation (README.md, etc.).

### 3. Run checks

```bash
cd akar-core

# Format check
cargo fmt --all -- --check

# Clippy
cargo clippy --workspace --all-targets -- -D warnings

# Full test suite
cargo test --workspace --no-fail-fast
```

### 4. Commit and tag

```bash
git add -A
git commit -m "chore: bump version to vX.Y.Z"
git tag vX.Y.Z
git push origin release/vX.Y.Z
git push origin vX.Y.Z
```

### 5. Create PR

Open a pull request from `release/vX.Y.Z` to `main`. The CI will run automatically.

### 6. Release

Once the PR is merged and the tag is pushed to `main`, the `rust-release.yml` workflow will:

1. Run `cargo test --workspace` on Ubuntu
2. Build CLI binaries for Linux, macOS, and Windows
3. Create a GitHub Release with auto-generated changelog from git log
4. Attach CLI binaries as release assets

### 7. Verify

```bash
# Download and test the binary
curl -LO https://github.com/mlengse/akar/releases/download/vX.Y.Z/akar-cli-linux-amd64
chmod +x akar-cli-linux-amd64
./akar-cli-linux-amd64 --version
```

---

## Manual Dispatch (Dry Run)

You can trigger the release workflow manually from the GitHub Actions UI without pushing a tag:

1. Go to **Actions** → **Rust Release** → **Run workflow**
2. Keep **"Dry run"** set to `true` (default)
3. Click **"Run"**

This runs tests and a release build to verify everything compiles, without creating an actual release.

---

## crates.io Publishing

Publishing to crates.io is **active** (Sprint 18/P50, 2026-08-08). The full process:

1. All 33 publishable crates have `publish = true` (only `akar-c` remains `publish = false` —
   FFI cdylib, local builds only; legacy daemon/dream crates are no longer part of the workspace).
2. Publish internal crates bottom-up in dependency order (see graph below), paced by the
   crates.io rate limit.
3. `akar-main`'s path dependencies ship with `version = "0.3.0"` so they resolve to the registry copies after publish.
4. Publish `akar-main`, then `akar-cli`, `akar-wasm`, `akar-migrate`.

### Dependency Publication Order (33 crates)

```
akar-common        (no deps)
akar-parser        (no deps)
akar-function      (common)
akar-catalog       (common)
akar-transaction   (common)
akar-extension     (common, function, catalog)
akar-binder        (common, parser, catalog)
akar-vector        (common, function, extension)
akar-storage       (common, catalog, transaction, vector)
akar-fts           (common, function, extension, storage)
akar-planner       (common, binder, parser, catalog)
akar-graph         (common, storage)
akar-optimizer     (common, planner, storage, binder, parser)
akar-processor     (common, catalog, planner, function, storage, transaction, parser, fts, vector, binder)
akar-ml            (common, function, extension, catalog)
akar-json          (common, function, extension)
akar-httpfs        (common, function, extension)
akar-markdown      (common, function, extension)
akar-duckdb        (common, function, extension) — must precede delta/iceberg/azure/unity-catalog
akar-algo          (common, function, extension, graph, catalog)
akar-neo4j         (function, extension)
akar-llm           (common, function, extension)
akar-sqlite        (common, function, extension)
akar-postgres      (common, function, extension, catalog)
akar-delta         (common, function, extension, duckdb)
akar-iceberg       (common, function, extension, duckdb)
akar-azure         (common, function, extension, duckdb)
akar-unity-catalog (common, function, extension, duckdb)
akar-search        (no deps)
akar-main          (depends on ALL core + json/httpfs/duckdb/algo/llm/sqlite/delta/iceberg/azure/postgres)
akar-migrate       (main, storage, common)
akar-wasm          (main, common)
akar-cli           (main, common, binder, catalog)
```

Extension crates (akar-json, akar-markdown, akar-fts, akar-vector, akar-httpfs, akar-duckdb, akar-algo, akar-neo4j, akar-llm, akar-sqlite, akar-delta, akar-iceberg, akar-azure, akar-unity-catalog, akar-search, akar-postgres) can be published in any order after their core dependencies are available.

**SKIP_CRATES (non-publishable):** `akar-c` (FFI cdylib, local builds only), `akar-python` (PyPI, frozen at 0.2.5), `akar-fuzz` (internal)

### Status

As of 2026-11-05: **33/33 published** (bottom-up, dependency-ordered). Latest release tag **v0.3.4**.
Detail per-crate patch per rilis tercatat di [`CHANGELOG.md`](../CHANGELOG.md) — jangan
duplikasi tabel versi di sini agar tidak drift.
Rate limit: 1 crate/min after the initial burst (30).

---

## Release Script Usage

```bash
# Full release (gate → bump → dep align → commit → tag → crates.io → GitHub)
python tools/release.py 0.3.4

# Dry run (gate + version bump only, no publish)
python tools/release.py 0.3.4 --dry

# Skip tests (use with caution)
python tools/release.py 0.3.4 --skip-gate

# Only crates.io publish (assumes version already bumped & tagged)
python tools/release.py --publish-only 0.3.4
```

Prerequisites:
- `cargo login <token>` (or `CARGO_REGISTRY_TOKEN` env var)
- `gh auth login` (for GitHub release creation)
- Clean working tree (or `--allow-dirty`)