//! In-process ML extension for Akar.
//!
//! Provides the `embed_text` scalar function that generates dense vector
//! embeddings locally via ONNX Runtime (fastembed), with no external API
//! calls. This is the local counterpart to `akar-llm`'s `create_embedding`
//! (which uses OpenAI/Ollama over HTTP).
//!
//! The embedding model is resolved lazily on first invocation from the
//! `AKAR_EMBED_MODEL` environment variable, defaulting to `BGE-small-en-v1.5`
//! (384 dims). The provider is shared across calls via a process-wide
//! singleton, so the model is downloaded/loaded at most once.

use std::sync::Arc;
use std::sync::OnceLock;

use akar_common::types::Value;
use akar_extension::{Extension, ExtensionContext};
use akar_function::registry::ScalarFunction;

use crate::embed::{EmbeddingProvider, FastEmbedProvider};

/// The ML embedding extension.
pub struct MlExtension;

impl Default for MlExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl MlExtension {
    pub fn new() -> Self {
        Self
    }
}

/// Process-wide shared embedding provider, initialized lazily on first use.
static EMBED_PROVIDER: OnceLock<Arc<dyn EmbeddingProvider>> = OnceLock::new();

/// Resolve the embedding model name from `AKAR_EMBED_MODEL` or the default.
fn default_model_name() -> String {
    std::env::var("AKAR_EMBED_MODEL").unwrap_or_else(|_| "BGESmallENV15".to_string())
}

/// Build a fresh embedding provider from the configured model.
fn build_provider() -> Result<Arc<dyn EmbeddingProvider>, String> {
    let model_name = default_model_name();
    let model = model_name
        .parse::<fastembed::EmbeddingModel>()
        .map_err(|e: String| format!("embed_text: {e}"))?;
    let config = crate::embed::EmbedProviderConfig {
        model,
        cache_dir: None,
        max_length: None,
        intra_threads: None,
    };
    let provider = FastEmbedProvider::try_new(config)
        .map_err(|e| format!("embed_text: failed to init model '{model_name}': {e}"))?;
    Ok(Arc::new(provider) as Arc<dyn EmbeddingProvider>)
}

/// Get (or lazily initialize) the shared embedding provider.
fn shared_provider() -> Result<&'static Arc<dyn EmbeddingProvider>, String> {
    if let Some(provider) = EMBED_PROVIDER.get() {
        return Ok(provider);
    }
    let provider = build_provider()?;
    Ok(EMBED_PROVIDER.get_or_init(|| provider))
}

/// Return the process-wide shared embedding provider, if it can be initialised.
///
/// This is used by the dream engine and other in-process consumers to reuse the
/// same model that backs the `embed_text` UDF. It never panics: if the model
/// cannot be built (e.g. no ONNX Runtime or a bad `AKAR_EMBED_MODEL`), it
/// returns `None` so callers degrade gracefully.
pub fn shared_embedding_provider() -> Option<Arc<dyn EmbeddingProvider>> {
    shared_provider().ok().cloned()
}

impl Extension for MlExtension {
    fn name(&self) -> &'static str {
        "ML"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        context.register_scalar_function(
            "embed_text",
            ScalarFunction::CustomScalar {
                name: "embed_text".into(),
                execute: Arc::new(|args| {
                    if args.is_empty() {
                        return Err("embed_text requires at least a text argument".into());
                    }
                    let text = match &args[0] {
                        Value::String(s) => s,
                        _ => return Err("embed_text: first argument must be string".into()),
                    };

                    let provider = shared_provider()?;
                    let vector = provider
                        .embed_dense(&[text.as_str()])
                        .map_err(|e| format!("embed_text: {e}"))?;
                    let vector = vector
                        .into_iter()
                        .next()
                        .ok_or_else(|| "embed_text: empty embedding result".to_string())?;
                    let vals: Vec<Value> = vector.into_iter().map(|v| Value::Double(v as f64)).collect();
                    Ok(Value::List(vals))
                }),
            },
        );

        tracing::info!("ML extension loaded: embed_text function registered (CustomScalar)");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_common::file_system::VirtualFileSystemRegistry;
    use akar_extension::ExtensionContext;
    use akar_function::registry::FunctionRegistry;
    use std::sync::{Arc, Mutex};

    fn test_context() -> ExtensionContext {
        ExtensionContext::new(
            Arc::new(Mutex::new(FunctionRegistry::new())),
            Arc::new(Mutex::new(akar_catalog::Catalog::new())),
            Arc::new(VirtualFileSystemRegistry::new()),
        )
    }

    #[test]
    fn test_ml_extension_name() {
        let ext = MlExtension::new();
        assert_eq!(ext.name(), "ML");
    }

    #[test]
    fn test_embed_text_function_registered() {
        let ctx = test_context();
        let ext = MlExtension::new();
        ext.load(&ctx).unwrap();

        let reg = ctx.function_registry().lock().unwrap();
        assert!(
            reg.get_scalar("embed_text").is_some(),
            "embed_text should be registered"
        );
    }

    #[test]
    fn test_embed_text_invalid_arg() {
        let ctx = test_context();
        let ext = MlExtension::new();
        ext.load(&ctx).unwrap();

        let reg = ctx.function_registry().lock().unwrap();
        let func = reg.get_scalar("embed_text").unwrap();
        let err = match func {
            ScalarFunction::CustomScalar { execute, .. } => {
                execute(&[Value::Int64(42)]).expect_err("non-string must error")
            }
            _ => panic!("embed_text must be a CustomScalar"),
        };
        assert!(err.contains("must be string"), "unexpected error: {err}");
    }
}
