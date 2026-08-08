# Release Process — Akar

This document describes how to cut a release of Akar (`akar-core/`).

> **crates.io publishing is active (since 2026-08-08, Sprint 18/P50).** 31/31 publishable crates are versioned `0.1.0` and published bottom-up to crates.io (all verified live on crates.io; crates.io rate limit was 1 new crate per 10 min, so publish was paced). GitHub Releases with prebuilt CLI binaries are produced as well.

---

## Version Numbering

We follow [Semantic Versioning 2.0.0](https://semver.org/) with the `MAJOR.MINOR.PATCH` format:

- **MAJOR**: Breaking changes to the public API.
- **MINOR**: New features, non-breaking additions.
- **PATCH**: Bug fixes, performance improvements, documentation.

Pre-release versions use suffixes like `-alpha.1`, `-beta.2`, `-rc.1`.

The current version is tracked in `akar-core/Cargo.toml` under `[workspace.package] version`.

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
git checkout -b release/v0.1.0
```

### 2. Update version

Update the version in `akar-core/Cargo.toml`:

```toml
[workspace.package]
version = "0.1.0"  # Change to target version
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
git commit -m "chore: bump version to 0.1.0"
git tag v0.1.0
git push origin release/v0.1.0
git push origin v0.1.0
```

### 5. Create PR

Open a pull request from `release/v0.1.0` to `main`. The CI will run automatically.

### 6. Release

Once the PR is merged and the tag is pushed to `main`, the `rust-release.yml` workflow will:

1. Run `cargo test --workspace` on Ubuntu
2. Build CLI binaries for Linux, macOS, and Windows
3. Create a GitHub Release with auto-generated changelog from git log
4. Attach CLI binaries as release assets

### 7. Verify

```bash
# Download and test the binary
curl -LO https://github.com/mlengse/akar/releases/download/v0.1.0/akar-cli-linux-amd64
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

1. All internal crates have `publish = true` (only `akar-c` remains `publish = false` — FFI cdylib, local builds only).
2. Publish internal crates bottom-up in dependency order (see graph below), 1 crate per 10 min due to the crates.io rate limit.
3. `akar-main`'s path dependencies ship with `version = "0.1.0"` so they resolve to the registry copies after publish.
4. Publish `akar-main`, then `akar-server`, `akar-wasm`, `akar-cli`, `akar-migrate`.

### Dependency Publication Order

```
akar-common
  → akar-storage, akar-transaction, akar-function, akar-parser
    → akar-catalog
      → akar-binder
        → akar-planner
          → akar-optimizer
            → akar-processor
              → akar-graph
                → akar-main
                  → akar-cli
```

Extension crates (akar-json, akar-fts, etc.) can be published in any order after their core dependencies are available.

### Status

As of 2026-08-08: **31/31 published** (all `0.1.0`, P50 complete).
