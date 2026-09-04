//! In-process ML facade for the akar engine (P89.5).
//!
//! This module is the bridge between the optional [`akar-ml`] extension and the
//! rest of the engine. It exposes the process-wide embedding provider built by
//! the ML extension so in-process consumers (e.g. the akar-server dream engine)
//! can reuse the same model that backs the `embed_text` UDF.
//!
//! Gated behind the `ml-extension` feature and excluded from the WASM build
//! (ONNX Runtime is not available there).

use std::sync::Arc;

/// Return the process-wide shared embedding provider, if one can be built.
///
/// Never panics: returns `None` when the `ml-extension` feature is compiled out,
/// when ONNX Runtime is unavailable, or when the model fails to initialise. This
/// lets callers degrade gracefully to a no-embedding path.
pub fn shared_embedding_provider() -> Option<Arc<dyn akar_ml::embed::EmbeddingProvider>> {
    akar_ml::extension::shared_embedding_provider()
}
