//! Air-gapped bundled model assets (feature `bundle-default-models`).
//!
//! The crate's `build.rs` copies the git-ignored staging tree
//! `models/.staging/<name>/` into the build's `$OUT_DIR/assets/<name>/`; the
//! extracted root is injected at compile time through the `AKAR_ML_ASSET_DIR`
//! env var that the build script emits, so this module never performs file I/O
//! at runtime beyond path checks and never touches the network.
//!
//! A bundle is the canonical `models/<name>/` file set — `model.onnx`,
//! `tokenizer.json`, `config.json`, `special_tokens_map.json`,
//! `tokenizer_config.json` — exactly the set `crate::sbyo::SbyoLoad::from_dir`
//! reads. See `akar-ml/models/README.md` for the full schema contract.

use std::path::{Path, PathBuf};

/// Root of the extracted bundled assets (`$OUT_DIR/assets`, set by build.rs
/// only under the `bundle-default-models` feature).
pub(crate) static ASSET_DIR: &str = env!("AKAR_ML_ASSET_DIR");

/// The canonical file names that make up a `models/<name>/` bundle, mirroring
/// the schema documented in `akar-ml/models/README.md`.
pub const BUNDLE_FILES: &[&str] = &[
    "model.onnx",
    "tokenizer.json",
    "config.json",
    "special_tokens_map.json",
    "tokenizer_config.json",
];

/// Directory of a bundled model by canonical name.
///
/// Returns `Some(dir)` when `dir/model.onnx` exists and `None` otherwise
/// (unknown name, or `models/.staging/` carried no blob for this name at
/// build time — e.g. features enabled without local staging).
pub fn bundled_model_dir(name: &str) -> Option<PathBuf> {
    let dir = Path::new(ASSET_DIR).join(name);
    dir.join("model.onnx").is_file().then_some(dir)
}

/// Path of a bundled model by canonical name, guaranteed complete.
///
/// Like [`bundled_model_dir`] but additionally requires every
/// [`BUNDLE_FILES`] entry to be present, so the returned directory is ready to
/// hand to an offline loader (e.g. [`crate::embed::FastEmbedProvider::new_from_dir`])
/// without a partial-extraction surprise.
pub fn bundled_model(name: &str) -> Option<PathBuf> {
    let dir = bundled_model_dir(name)?;
    is_complete_bundle(&dir).then_some(dir)
}

/// `true` when every [`BUNDLE_FILES`] entry is present under `dir`.
///
/// Use before handing a directory to a loader ([`crate::embed`] offline
/// constructors) so a stale/partial extraction fails fast with a clear cause.
pub fn is_complete_bundle(dir: &Path) -> bool {
    BUNDLE_FILES.iter().all(|file| dir.join(file).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// The asset dir injected by build.rs must resolve under the feature.
    #[test]
    fn asset_dir_resolves() {
        let root = Path::new(ASSET_DIR);
        assert!(root.is_absolute(), "asset dir must be absolute");
        assert!(root.is_dir(), "build.rs must have created `{ASSET_DIR}`");
    }

    /// Unknown or unstaged model names yield `None` — never a panic, never a
    /// wrong directory.
    #[test]
    fn missing_model_is_none() {
        assert!(bundled_model_dir("definitely-not-a-bundled-model").is_none());
    }

    /// A staged bundle is copied by build.rs into the asset dir with every
    /// schema file, byte-identical to staging. Self-skips when
    /// `models/.staging/**` was absent at build time (e.g. CI checkout) so the
    /// feature never needs the network.
    #[test]
    fn staged_bundle_copied_and_complete() {
        let Some(dir) = bundled_model_dir("synthetic") else {
            return; // nothing staged at build time — graceful skip
        };
        assert!(
            is_complete_bundle(&dir),
            "bundled `synthetic` must contain every schema file in `{}`",
            dir.display()
        );
        let staging = Path::new(env!("CARGO_MANIFEST_DIR")).join("models/.staging/synthetic");
        if staging.is_dir() {
            for file in BUNDLE_FILES {
                let src = staging.join(file);
                if src.is_file() {
                    assert_eq!(
                        std::fs::read(&src).unwrap(),
                        std::fs::read(dir.join(file)).unwrap(),
                        "copy of `{file}` must be byte-identical",
                    );
                }
            }
        }
    }
}
