//! Native bring-your-own (SBYO) sparse embedding session.
//!
//! fastembed 6.x exposes `try_new_from_user_defined` for dense, BGE-M3, and
//! rerankers but not for [`fastembed::SparseTextEmbedding`]. This module builds
//! an ONNX Runtime session directly (`ort` + `tokenizers` + `ndarray`) and
//! replicates SPLADE post-processing so the output is bit-identical to
//! fastembed's `post_process_splade` — enabling fully offline SPLADE embedding
//! from user-supplied ONNX bytes.
//!
//! Only SPLADE-style models (3-D output `(batch, seq, vocab)`) are supported on
//! this path. BGE-M3 sparse is deliberately rejected: its ONNX carries external
//! initializers (multi-file) and needs embedded projection weights, which is out
//! of scope for the SBYO sparse path.

use fastembed::TokenizerFiles;
use ndarray::{Array, ArrayViewD, Axis, CowArray, Dim};
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Value;
use tokenizers::{AddedToken, PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use crate::embed::{EmbeddingError, SparseEmbedding};

/// Default max sequence length used by the fastembed sparse pipeline.
const DEFAULT_MAX_LENGTH: usize = 512;

/// SPLADE emits full-vocabulary logits (30522 for SPLADE++/BERT). Any output whose
/// last axis is at or below this threshold is not a SPLADE-style vocabulary — the
/// guard rejects e.g. BGE-M3 hidden states (1024) at embed time.
const MIN_VOCAB_SIZE: usize = 1024;

/// A ready-to-run SPLADE inference session built entirely from user-supplied bytes.
pub(crate) struct NativeSparseSession {
    session: Session,
    tokenizer: Tokenizer,
    need_token_type_ids: bool,
}

impl std::fmt::Debug for NativeSparseSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeSparseSession").finish_non_exhaustive()
    }
}

impl NativeSparseSession {
    /// Build a session from raw ONNX bytes and tokenizer files (offline / air-gapped).
    pub(crate) fn try_new(onnx_bytes: &[u8], tokenizer_files: TokenizerFiles) -> Result<Self, EmbeddingError> {
        let mut session_builder = Session::builder()
            .map_err(|e| EmbeddingError::InitFailed(e.to_string()))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| EmbeddingError::InitFailed(e.to_string()))?
            .with_intra_threads(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1))
            .map_err(|e| EmbeddingError::InitFailed(e.to_string()))?;

        let session = session_builder.commit_from_memory(onnx_bytes).map_err(|e| {
            EmbeddingError::InitFailed(format!(
                "failed to build ONNX session from user-defined sparse model: {e}"
            ))
        })?;

        let tokenizer = build_tokenizer(&tokenizer_files, DEFAULT_MAX_LENGTH)?;

        let need_token_type_ids = session.inputs().iter().any(|input| input.name() == "token_type_ids");

        Ok(Self {
            session,
            tokenizer,
            need_token_type_ids,
        })
    }

    /// Embed a batch of texts into SPLADE sparse vectors with SPLADE post-processing.
    pub(crate) fn embed(&mut self, texts: &[&str], batch_size: usize) -> Result<Vec<SparseEmbedding>, EmbeddingError> {
        if batch_size == 0 {
            return Err(EmbeddingError::ComputeFailed(
                "batch_size must be greater than 0".into(),
            ));
        }

        let mut out = Vec::with_capacity(texts.len());
        for batch in texts.chunks(batch_size) {
            let encodings = self
                .tokenizer
                .encode_batch(batch.to_vec(), true)
                .map_err(|e| EmbeddingError::ComputeFailed(format!("failed to encode the batch: {e}")))?;

            let encoding_length = encodings
                .first()
                .ok_or_else(|| EmbeddingError::ComputeFailed("empty tokenization".into()))?
                .len();
            let batch_size = batch.len();

            let mut ids_array = Vec::with_capacity(encoding_length * batch_size);
            let mut mask_array = Vec::with_capacity(encoding_length * batch_size);
            let mut type_ids_array = Vec::with_capacity(encoding_length * batch_size);

            for encoding in &encodings {
                ids_array.extend(encoding.get_ids().iter().map(|x| *x as i64));
                mask_array.extend(encoding.get_attention_mask().iter().map(|x| *x as i64));
                type_ids_array.extend(encoding.get_type_ids().iter().map(|x| *x as i64));
            }

            let inputs_ids_array = Array::from_shape_vec((batch_size, encoding_length), ids_array)
                .map_err(|e| EmbeddingError::ComputeFailed(format!("invalid input_ids shape: {e}")))?;
            let attention_mask_array = Array::from_shape_vec((batch_size, encoding_length), mask_array)
                .map_err(|e| EmbeddingError::ComputeFailed(format!("invalid attention_mask shape: {e}")))?;
            let token_type_ids_array = Array::from_shape_vec((batch_size, encoding_length), type_ids_array)
                .map_err(|e| EmbeddingError::ComputeFailed(format!("invalid token_type_ids shape: {e}")))?;

            let mut session_inputs = ort::inputs![
                "input_ids" => Value::from_array(inputs_ids_array.clone())
                    .map_err(|e| EmbeddingError::ComputeFailed(e.to_string()))?,
                "attention_mask" => Value::from_array(attention_mask_array.clone())
                    .map_err(|e| EmbeddingError::ComputeFailed(e.to_string()))?,
            ];

            if self.need_token_type_ids {
                session_inputs.push((
                    "token_type_ids".into(),
                    Value::from_array(token_type_ids_array)
                        .map_err(|e| EmbeddingError::ComputeFailed(e.to_string()))?
                        .into(),
                ));
            }

            let outputs = self
                .session
                .run(session_inputs)
                .map_err(|e| EmbeddingError::ComputeFailed(e.to_string()))?;

            let last_hidden_state_key = match outputs.len() {
                1 => outputs
                    .keys()
                    .next()
                    .ok_or_else(|| EmbeddingError::ComputeFailed("missing ONNX output".into()))?,
                _ => "last_hidden_state",
            };

            let (shape, data) = outputs[last_hidden_state_key]
                .try_extract_tensor::<f32>()
                .map_err(|e| EmbeddingError::ComputeFailed(e.to_string()))?;
            let shape: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
            if shape.len() != 3 || shape[2] <= MIN_VOCAB_SIZE {
                return Err(EmbeddingError::ComputeFailed(format!(
                    "unexpected SPLADE output shape {shape:?} — the SBYO sparse path supports \
                     SPLADE-style (batch, seq, vocab) ONNX models only; BGE-M3 sparse offline \
                     is not supported"
                )));
            }
            let output_array = ArrayViewD::from_shape(shape.as_slice(), data)
                .map_err(|e| EmbeddingError::ComputeFailed(format!("invalid output shape: {e}")))?;
            let attention_mask_cow = CowArray::from(&attention_mask_array);

            out.extend(post_process_splade(&output_array, &attention_mask_cow));
        }

        Ok(out)
    }
}

/// Build a tokenizer from SBYO tokenizer files with the same configuration fastembed
/// applies in `load_tokenizer`: `BatchLongest` padding, truncation to the effective
/// max length, and special tokens from `special_tokens_map.json`.
fn build_tokenizer(tokenizer_files: &TokenizerFiles, max_length: usize) -> Result<Tokenizer, EmbeddingError> {
    let parse = |name: &str, bytes: &[u8]| -> Result<serde_json::Value, EmbeddingError> {
        serde_json::from_slice(bytes)
            .map_err(|_| EmbeddingError::InitFailed(format!("error building tokenizer — could not parse {name}")))
    };

    let config = parse("config.json", &tokenizer_files.config_file)?;
    let special_tokens_map = parse("special_tokens_map.json", &tokenizer_files.special_tokens_map_file)?;
    let tokenizer_config = parse("tokenizer_config.json", &tokenizer_files.tokenizer_config_file)?;

    let mut tokenizer = Tokenizer::from_bytes(&tokenizer_files.tokenizer_file)
        .map_err(|_| EmbeddingError::InitFailed("error building tokenizer — could not read tokenizer.json".into()))?;

    let model_max_length = tokenizer_config["model_max_length"].as_f64().ok_or_else(|| {
        EmbeddingError::InitFailed("tokenizer_config.json is missing a numeric `model_max_length` field".into())
    })? as usize;
    let max_length = max_length.min(model_max_length);
    let pad_id = config["pad_token_id"].as_u64().unwrap_or(0) as u32;
    let pad_token: String = tokenizer_config["pad_token"]
        .as_str()
        .ok_or_else(|| {
            EmbeddingError::InitFailed("tokenizer_config.json is missing a string `pad_token` field".into())
        })?
        .into();

    let mut tokenizer = tokenizer
        .with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            pad_token,
            pad_id,
            ..Default::default()
        }))
        .with_truncation(Some(TruncationParams {
            max_length,
            ..Default::default()
        }))
        .map_err(|e| EmbeddingError::InitFailed(format!("failed to configure tokenizer: {e}")))?
        .clone();

    if let serde_json::Value::Object(root_object) = special_tokens_map {
        for (_, value) in root_object.iter() {
            if value.is_string() {
                if let Some(content) = value.as_str() {
                    tokenizer.add_special_tokens(&[AddedToken {
                        content: content.into(),
                        special: true,
                        ..Default::default()
                    }]);
                }
            } else if value.is_object() {
                if let (Some(content), Some(single_word), Some(lstrip), Some(rstrip), Some(normalized)) = (
                    value["content"].as_str(),
                    value["single_word"].as_bool(),
                    value["lstrip"].as_bool(),
                    value["rstrip"].as_bool(),
                    value["normalized"].as_bool(),
                ) {
                    tokenizer.add_special_tokens(&[AddedToken {
                        content: content.into(),
                        special: true,
                        single_word,
                        lstrip,
                        rstrip,
                        normalized,
                    }]);
                }
            }
        }
    }

    Ok(tokenizer.into())
}

/// Mirror of fastembed's `post_process_splade`: `log(1 + relu)`, mask, max-pool over
/// the sequence axis, then keep `(index, value)` pairs with a positive weight.
fn post_process_splade(
    model_output: &ArrayViewD<f32>,
    attention_mask: &CowArray<i64, Dim<[usize; 2]>>,
) -> Vec<SparseEmbedding> {
    let relu_log = model_output.mapv(|x| (1.0 + x.max(0.0)).ln());

    let attention_mask = attention_mask.mapv(|x| x as f32).insert_axis(Axis(2));

    let weighted_log = relu_log * attention_mask;

    let scores = weighted_log.fold_axis(Axis(1), f32::NEG_INFINITY, |r, &v| r.max(v));

    scores
        .rows()
        .into_iter()
        .map(|row_scores| {
            let mut values: Vec<f32> = Vec::with_capacity(row_scores.len());
            let mut indices: Vec<usize> = Vec::with_capacity(row_scores.len());

            row_scores.into_iter().enumerate().for_each(|(idx, f)| {
                if *f > 0.0 {
                    values.push(*f);
                    indices.push(idx);
                }
            });

            SparseEmbedding { values, indices }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;
    /// Verify mask handling, `log(1 + relu)` math, max-pooling over the sequence axis,
    /// and the positive-weight filter against hand-computed SPLADE semantics.
    #[test]
    fn test_post_process_splade_rules() {
        let mut buf = Vec::with_capacity(12);
        // batch 0: seq 0 [0,-1], seq 1 [2,0], seq 2 [1,0.5] (seq 2 masked out)
        buf.extend_from_slice(&[0.0f32, -1.0, 2.0, 0.0, 1.0, 0.5]);
        // batch 1: seq 0 [7,-3], seq 1 [0,0], seq 2 [0,1]
        buf.extend_from_slice(&[7.0f32, -3.0, 0.0, 0.0, 0.0, 1.0]);
        let output = ArrayViewD::from_shape(vec![2, 3, 2], buf.as_slice()).expect("shape ok");
        let mask = Array2::from_shape_vec((2, 3), vec![1i64, 1, 0, 1, 1, 1]).unwrap();
        let mask_cow = CowArray::from(&mask);

        let embs = post_process_splade(&output, &mask_cow);

        // row 0: masked seq 2 contributes nothing; max-pool over unmasked seqs.
        // token 0 pools ln(1+2) from seq 1; token 1's sole positive logit lives
        // in the masked seq, so it must not appear.
        assert_eq!(embs[0].indices, vec![0]);
        let l = |x: f32| (1.0 + x.max(0.0)).ln();
        assert!((embs[0].values[0] - l(2.0)).abs() < 1e-6);

        // row 1: no padding; token 0 max-pools ln(1+7) over 0.0; token 1 pools ln(2).
        assert_eq!(embs[1].indices, vec![0, 1]);
        assert!((embs[1].values[0] - l(7.0)).abs() < 1e-6);
        assert!((embs[1].values[1] - l(1.0)).abs() < 1e-6);
    }

    /// Padding positions must never contribute weights, even when their logits are
    /// large after `log(1 + relu)`.
    #[test]
    fn test_post_process_splade_respects_mask() {
        // batch 1, seq 2, vocab 2: seq 0 [5,-10] unmasked, seq 1 [7,7] padding.
        let buf = vec![5.0f32, -10.0, 7.0, 7.0];
        let output = ArrayViewD::from_shape(vec![1, 2, 2], buf.as_slice()).unwrap();
        let mask = Array2::from_shape_vec((1, 2), vec![1i64, 0]).unwrap();

        let embs = post_process_splade(&output, &CowArray::from(&mask));

        // Seq 1 is padding: its high logits (7,7) must not leak into the result.
        assert_eq!(embs[0].indices, vec![0]);
        assert_eq!(embs[0].len(), 1);
        let l = |x: f32| (1.0 + x.max(0.0)).ln();
        assert!((embs[0].values[0] - l(5.0)).abs() < 1e-6);
    }
}
