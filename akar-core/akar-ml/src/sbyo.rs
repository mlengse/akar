//! Shared offline "bring-your-own" ONNX model loading.
//!
//! fastembed's `*Embedding::try_new_from_user_defined` constructors all consume
//! the same shape of local files: a single ONNX model plus four tokenizer JSON
//! files. This module centralises locating and reading those files from a
//! directory so every provider (`FastEmbedProvider`, `SparseEmbedProvider`,
//! `Bgem3Provider`, `RerankProvider`) can build a user-defined model the same
//! way, without network downloads (P90.1).

use std::path::Path;

use fastembed::TokenizerFiles;

use crate::embed::EmbeddingError;

/// An ONNX model plus the tokenizer files required to run it via fastembed.
///
/// Produced by [`SbyoLoad`]; consumed by the per-provider
/// `*Embedding::try_new_from_user_defined` constructors.
#[derive(Debug, Clone)]
pub(crate) struct SbyoModel {
    /// Raw ONNX model bytes.
    pub(crate) onnx: Vec<u8>,
    /// Tokenizer JSON files (`tokenizer.json`, `config.json`,
    /// `special_tokens_map.json`, `tokenizer_config.json`).
    pub(crate) tokenizer: TokenizerFiles,
}

/// Builder that loads a user-defined model from a local directory.
#[derive(Debug, Default)]
pub(crate) struct SbyoLoad;

impl SbyoLoad {
    /// Load the ONNX model and tokenizer files from `dir`.
    ///
    /// The directory must contain a `.onnx` file plus `tokenizer.json`,
    /// `config.json`, `special_tokens_map.json`, and `tokenizer_config.json`.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the directory is unreadable or
    /// the ONNX model or a required tokenizer file is missing.
    pub(crate) fn from_dir(dir: &Path) -> Result<SbyoModel, EmbeddingError> {
        let onnx_path = find_onnx_file(dir)?;
        let onnx = std::fs::read(&onnx_path).map_err(|e| {
            EmbeddingError::InitFailed(format!("failed to read ONNX model {}: {e}", onnx_path.display()))
        })?;

        let tokenizer = TokenizerFiles {
            tokenizer_file: read_required_token_file(dir, "tokenizer.json")?,
            config_file: read_required_token_file(dir, "config.json")?,
            special_tokens_map_file: read_required_token_file(dir, "special_tokens_map.json")?,
            tokenizer_config_file: read_required_token_file(dir, "tokenizer_config.json")?,
        };

        Ok(SbyoModel { onnx, tokenizer })
    }
}

/// Locate the ONNX model file in a local model directory.
///
/// Prefers a non-quantized `*.onnx` file over a quantized `*_int8.onnx` when both
/// are present. Returns [`EmbeddingError::InitFailed`] when no `.onnx` file exists.
pub(crate) fn find_onnx_file(dir: &Path) -> Result<std::path::PathBuf, EmbeddingError> {
    let mut fallback = None;
    for entry in std::fs::read_dir(dir)
        .map_err(|e| EmbeddingError::InitFailed(format!("cannot read model directory {}: {e}", dir.display())))?
    {
        let path = entry
            .map_err(|e| EmbeddingError::InitFailed(format!("cannot read model directory entry: {e}")))?
            .path();
        if path.extension().is_some_and(|ext| ext == "onnx") {
            if path.to_string_lossy().ends_with("_int8.onnx") {
                fallback.get_or_insert(path);
            } else {
                return Ok(path);
            }
        }
    }
    fallback.ok_or_else(|| {
        EmbeddingError::InitFailed(format!(
            "no .onnx model found in {} (expected a plain .onnx or *_int8.onnx file)",
            dir.display()
        ))
    })
}

/// Read a required tokenizer file from a local model directory.
fn read_required_token_file(dir: &Path, name: &str) -> Result<Vec<u8>, EmbeddingError> {
    let path = dir.join(name);
    std::fs::read(&path).map_err(|e| {
        EmbeddingError::InitFailed(format!("missing or unreadable tokenizer file {}: {e}", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sbyo_from_dir_empty_dir() {
        let dir = tempfile::tempdir().unwrap();

        match SbyoLoad::from_dir(dir.path()) {
            Err(EmbeddingError::InitFailed(msg)) => {
                assert!(msg.contains("onnx"), "expected missing-onnx message, got: {msg}");
            }
            other => panic!("expected InitFailed for empty dir, got {other:?}"),
        }
    }

    #[test]
    fn test_sbyo_from_dir_missing_tokenizer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("model.onnx"), b"not-a-real-onnx").unwrap();

        match SbyoLoad::from_dir(dir.path()) {
            Err(EmbeddingError::InitFailed(msg)) => {
                assert!(
                    msg.contains("tokenizer"),
                    "expected missing-tokenizer message, got: {msg}"
                );
            }
            other => panic!("expected InitFailed for missing tokenizer, got {other:?}"),
        }
    }

    #[test]
    fn test_sbyo_from_dir_loads_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("model.onnx"), b"onnx-bytes").unwrap();
        for name in [
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ] {
            std::fs::write(dir.path().join(name), b"{}").unwrap();
        }

        let model = SbyoLoad::from_dir(dir.path()).unwrap();
        assert_eq!(model.onnx.as_slice(), b"onnx-bytes");
        assert_eq!(model.tokenizer.tokenizer_file.as_slice(), b"{}");
        assert_eq!(model.tokenizer.config_file.as_slice(), b"{}");
        assert_eq!(model.tokenizer.special_tokens_map_file.as_slice(), b"{}");
        assert_eq!(model.tokenizer.tokenizer_config_file.as_slice(), b"{}");
    }

    #[test]
    fn test_sbyo_from_dir_prefers_plain_onnx() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("model_int8.onnx"), b"int8").unwrap();
        std::fs::write(dir.path().join("model.onnx"), b"plain").unwrap();
        for name in [
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ] {
            std::fs::write(dir.path().join(name), b"{}").unwrap();
        }

        // Locate the plain .onnx via find_onnx_file (pub(crate) helper, same module).
        let found = find_onnx_file(dir.path()).unwrap();
        assert_eq!(found.file_name().unwrap(), "model.onnx");
        assert!(!found.to_string_lossy().ends_with("_int8.onnx"));

        // The loader must pick the plain model, not the _int8 fallback.
        let model = SbyoLoad::from_dir(dir.path()).unwrap();
        assert_eq!(model.onnx.as_slice(), b"plain");
    }
}
