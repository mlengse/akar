//! Shared offline "bring-your-own" ONNX model loading.
//!
//! fastembed's `*Embedding::try_new_from_user_defined` constructors all consume
//! the same shape of local files: a single ONNX model plus four tokenizer JSON
//! files. This module centralises locating and reading those files from a
//! directory so every provider (`FastEmbedProvider`, `SparseEmbedProvider`,
//! `Bgem3Provider`, `RerankProvider`) can build a user-defined model the same
//! way, without network downloads (P90.1).

use std::collections::BTreeSet;
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
    /// External-initializer weights for models that keep them out of line
    /// (e.g. in a `model.onnx_data` sidecar, or referenced by
    /// `model.onnx_data_location`). Populated by [`SbyoLoad::from_dir`] (P93);
    /// empty for self-contained models. Consumed by the dense provider via
    /// `UserDefinedEmbeddingModel::with_external_initializer`.
    pub(crate) external_initializers: Vec<(String, Vec<u8>)>,
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
    /// Any external-initializer sidecar files (P93) are located and read into
    /// [`SbyoModel::external_initializers`].
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the directory is unreadable, the
    /// ONNX model or a required tokenizer file is missing, or an external
    /// initializer file cannot be read.
    pub(crate) fn from_dir(dir: &Path) -> Result<SbyoModel, EmbeddingError> {
        let onnx_path = find_onnx_file(dir)?;
        let onnx = std::fs::read(&onnx_path).map_err(|e| {
            EmbeddingError::InitFailed(format!("failed to read ONNX model {}: {e}", onnx_path.display()))
        })?;
        let external_initializers = read_external_initializers(dir, &onnx)?;

        let tokenizer = TokenizerFiles {
            tokenizer_file: read_required_token_file(dir, "tokenizer.json")?,
            config_file: read_required_token_file(dir, "config.json")?,
            special_tokens_map_file: read_required_token_file(dir, "special_tokens_map.json")?,
            tokenizer_config_file: read_required_token_file(dir, "tokenizer_config.json")?,
        };

        Ok(SbyoModel {
            onnx,
            external_initializers,
            tokenizer,
        })
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

/// Locate the external-initializer companion files of an ONNX model (P93.1).
///
/// Large or quantized ONNX models keep their weights in separate data files
/// instead of inline-tensor bytes. Two signals identify them:
///
/// 1. **Naming convention** — sidecar files ending in `.onnx_data` or
///    `.onnx.data` living next to the model (the de-facto HuggingFace/ONNX
///    export layout, e.g. `model.onnx` + `model.onnx_data`).
/// 2. **`model.onnx_data_location`** — the ONNX protobuf records each tensor's
///    data file as a `location` string (`TensorProto.external_data →
///    ExternalDataInfo.location`). [`scan_onnx_external_data_names`] extracts
///    those filenames from the raw model bytes without a protobuf dependency.
///
/// The union of both signals is intersected with the files actually present on
/// disk (a referenced location that ships detached is simply skipped) and
/// returned in deterministic order. Empty when the model is self-contained.
pub(crate) fn find_external_data_files(dir: &Path, onnx_bytes: &[u8]) -> Vec<std::path::PathBuf> {
    let mut names: BTreeSet<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            let lower = n.to_ascii_lowercase();
            lower.ends_with(".onnx_data") || lower.ends_with(".onnx.data")
        })
        .collect();
    names.extend(scan_onnx_external_data_names(onnx_bytes));

    let mut found: Vec<std::path::PathBuf> = names
        .into_iter()
        .map(|name| dir.join(name))
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found
}

/// Read the external-initializer sidecar files of an ONNX model as
/// `(file_name, bytes)` pairs — the shape fed to
/// `UserDefinedEmbeddingModel::with_external_initializer` (P93.2).
///
/// The `file_name` of each entry must match the name the model's
/// `model.onnx_data_location` references, so the raw file name is preserved
/// verbatim.
///
/// # Errors
///
/// Returns [`EmbeddingError::InitFailed`] if a discovered file cannot be read.
fn read_external_initializers(dir: &Path, onnx_bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, EmbeddingError> {
    find_external_data_files(dir, onnx_bytes)
        .into_iter()
        .map(|path| {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let buffer = std::fs::read(&path).map_err(|e| {
                EmbeddingError::InitFailed(format!("failed to read external initializer {}: {e}", path.display()))
            })?;
            Ok((file_name, buffer))
        })
        .collect()
}

/// Extract candidate external-data filenames from raw ONNX model bytes.
///
/// The ONNX wire format embeds `TensorProto.external_data` location strings as
/// plain UTF-8, so filename-shaped printable runs are surfaced directly. Runs
/// with path separators or leading dots (likely metadata/proto noise) and
/// overlong names are filtered. Callers must still verify existence on disk.
fn scan_onnx_external_data_names(onnx_bytes: &[u8]) -> Vec<String> {
    onnx_bytes
        .split(|&b| !b.is_ascii_graphic())
        .filter_map(|run| std::str::from_utf8(run).ok())
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('.') && !s.contains('/') && !s.contains('\\') && s.len() <= 255)
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
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
        std::fs::write(dir.path().join("model.onnx_data"), b"weights").unwrap();
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
        assert_eq!(
            model.external_initializers,
            [("model.onnx_data".to_string(), b"weights".to_vec())]
        );
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

    #[test]
    fn test_find_external_data_convention() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("model.onnx"), b"onnx").unwrap();
        std::fs::write(dir.path().join("model.onnx_data"), b"weights").unwrap();
        std::fs::write(dir.path().join("model_quantized.onnx_data"), b"q-weights").unwrap();
        std::fs::write(dir.path().join("model.onnx.data"), b"alt-weights").unwrap();
        std::fs::write(dir.path().join("tokenizer.json"), b"{}").unwrap();

        let found = find_external_data_files(dir.path(), b"<no locations>");
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["model.onnx.data", "model.onnx_data", "model_quantized.onnx_data"]
        );
    }

    #[test]
    fn test_find_external_data_referenced_location() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("model.onnx"), b"onnx").unwrap();

        // A companion file whose name only appears inside the model's
        // `model.onnx_data_location` (proto `location` strings), not in the
        // conventional *.onnx_data/*.onnx.data layout.
        std::fs::write(dir.path().join("split_weights.bin"), b"w1").unwrap();

        // Fake ONNX wire bytes carrying the location string as plain UTF-8.
        let onnx_bytes = b"\x00\x1fsplit_weights.bin\x00split_weights.bin\x12\x05".to_vec();
        let found = find_external_data_files(dir.path(), &onnx_bytes);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"split_weights.bin".to_string()));
        assert!(!names.iter().any(|n| n == "model.onnx"));

        // A referenced location that does not ship on disk is skipped.
        let dropped = find_external_data_files(dir.path(), b"\x00dropped.bin\x00");
        assert!(dropped.is_empty());
    }
}
