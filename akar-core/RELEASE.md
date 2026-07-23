# Release Process — Akar

This document describes how to cut a release of Akar (`akar-core/`).

> **crates.io publishing is deferred** (Design Decision #11). Only GitHub Releases with prebuilt CLI binaries are produced at this stage. See [implementation_plan.md §DD11](implementation_plan.md) for rationale.

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
curl -LO https://github.com/anjangkusumanetra/akar/releases/download/v0.1.0/akar-cli-linux-amd64
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

## Future: crates.io Publishing

When the API stabilises and we are ready to publish to crates.io, the following steps are needed:

1. Remove `publish = false` from all internal crates
2. Publish internal crates in dependency order (see graph below)
3. Update `akar-main`'s path dependencies to `{ version = "...", path = "..." }`
4. Re-enable `cargo publish -p akar-main` and `cargo publish -p akar-cli` in the release workflow

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
