# Release Process — Kuzu Core Rust

This document describes how to cut a release of the Kuzu Core Rust workspace (`kuzu-core/`) to crates.io.

---

## Version Numbering

We follow [Semantic Versioning 2.0.0](https://semver.org/) with the `MAJOR.MINOR.PATCH` format:

- **MAJOR**: Breaking changes to the public API (kuzu-main).
- **MINOR**: New features, non-breaking additions.
- **PATCH**: Bug fixes, performance improvements, documentation.

Pre-release versions use suffixes like `-alpha.1`, `-beta.2`, `-rc.1`.

The current version is tracked in `kuzu-core/Cargo.toml` under `[workspace.package] version`.

---

## What Gets Published

| Crate | Published | Notes |
|-------|-----------|-------|
| `kuzu-main` | ✅ Yes | Main library crate — the public API |
| `kuzu-cli`  | ✅ Yes | CLI binary (requires kuzu-main) |
| All other crates | ❌ No | Internal implementation details, marked `publish = false` |

> **Note:** `kuzu-main` depends on internal crates via `path = "../kuzu-*"`. Currently these are marked `publish = false`, which means `cargo publish -p kuzu-main` only works if those path dependencies are resolvable. For full crates.io publishing, you would need to either:
> - Publish all internal crates first, then update kuzu-main's deps to use `{ version = "...", path = "..." }`, or
> - Consolidate into a single crate.

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

Update the version in `kuzu-core/Cargo.toml`:

```toml
[workspace.package]
version = "0.1.0"  # Change to target version
```

Update any references to the old version in documentation (README.md, etc.).

### 3. Run checks

```bash
cd kuzu-core

# Format check
cargo fmt --all -- --check

# Clippy
cargo clippy --workspace --all-targets -- -D warnings

# Full test suite
cargo test --workspace --no-fail-fast

# WASM check (optional but recommended)
cargo check --target wasm32-unknown-unknown --workspace
```

### 4. Dry-run publish

```bash
# Dry-run to verify everything is publishable
cargo publish --dry-run -p kuzu-main
cargo publish --dry-run -p kuzu-cli
```

### 5. Commit and tag

```bash
git add -A
git commit -m "chore: bump version to 0.1.0"
git tag v0.1.0
git push origin release/v0.1.0
git push origin v0.1.0
```

### 6. Create PR

Open a pull request from `release/v0.1.0` to `main`. The CI will run automatically.

### 7. Publish

Once the PR is merged and the tag is pushed, the `rust-release.yml` workflow will:

1. Run `cargo test --workspace`
2. Publish `kuzu-main` to crates.io
3. Publish `kuzu-cli` to crates.io
4. Create a GitHub Release with auto-generated changelog

### 8. Verify

```bash
# Check that the published version is available
cargo search kuzu-main
```

---

## Dependency Publication Order

If you ever need to publish individual crates (after removing `publish = false`), the order is determined by the dependency graph:

```
kuzu-common
  → kuzu-storage, kuzu-transaction, kuzu-function, kuzu-parser
    → kuzu-catalog (depends on common, parser)
      → kuzu-binder (depends on common, catalog, parser)
        → kuzu-planner (depends on common, binder)
          → kuzu-optimizer (depends on planner)
            → kuzu-processor (depends on planner, function, storage)
              → kuzu-graph (depends on common, storage)
                → kuzu-main (depends on everything above)
                  → kuzu-cli (depends on main)
```

Extension crates (kuzu-json, kuzu-fts, etc.) can be published in any order after their core dependencies are available.

---

## Manual Dispatch

You can also trigger the release workflow manually from the GitHub Actions UI with `dry_run=true` to verify without publishing:

1. Go to Actions → Rust Release → "Run workflow"
2. Set "Dry run" to `true`
3. Click "Run"

This runs tests and `cargo publish --dry-run` for both publishable crates.
