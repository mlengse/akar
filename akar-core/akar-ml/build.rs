//! akar-ml `build.rs` — air-gapped bundled model assets (P98.1).
//!
//! Minimal, deterministic and idempotent. When the additive feature
//! `bundle-default-models` is enabled, the git-ignored staging tree
//! `models/.staging/<name>/` is copied into `$OUT_DIR/assets/<name>/` so model
//! bytes travel with the local build with no network round-trip. The library
//! consumes the extracted tree via the `AKAR_ML_ASSET_DIR` env var this script
//! emits and `akar_ml::assets::bundled_model_dir`.
//!
//! Contract (see `akar-ml/models/README.md`):
//! - No network, no system toolchain, no `[build-dependencies]`.
//! - Idempotent: a bundle dir whose `model.onnx` already exists is left alone;
//!   re-runs are no-ops (an interrupted copy is completed on the next run).
//! - Feature OFF: the script returns immediately and never touches build
//!   output, keeping the default `test [akar-core]` gate byte-identical.
//! - Missing or empty `models/.staging/` degrades to a `cargo:warning`
//!   (build still succeeds; the runtime helper then yields `None`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Sentinel proving a bundle directory is fully extracted.
fn extracted(bundle_dir: &Path) -> bool {
    bundle_dir.join("model.onnx").is_file()
}

/// Copy the regular files of `src` into `dst` (flat layout; subdirs skipped).
fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if env::var("CARGO_FEATURE_BUNDLE_DEFAULT_MODELS").is_err() {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let staging = manifest_dir.join("models").join(".staging");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("assets");

    println!("cargo:rustc-env=AKAR_ML_ASSET_DIR={}", out_dir.display());
    println!("cargo:rerun-if-changed={}", staging.display());
    let _ = fs::create_dir_all(&out_dir);

    let Ok(read_dir) = fs::read_dir(&staging) else {
        println!(
            "cargo:warning=akar-ml: `bundle-default-models` enabled but {} is missing — no bundled model assets (see akar-ml/models/README.md)",
            staging.display()
        );
        return;
    };

    let mut names: Vec<String> = read_dir
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();

    for name in &names {
        let dst = out_dir.join(name);
        if extracted(&dst) {
            continue;
        }
        if let Err(err) = copy_tree(&staging.join(name), &dst) {
            println!("cargo:warning=akar-ml: failed to stage bundled model `{name}`: {err}");
        }
    }
}
