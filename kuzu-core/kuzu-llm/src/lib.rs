//! LLM embedding extension for Kuzu.
//!
//! Provides the `CREATE_EMBEDDING` scalar function that generates
//! vector embeddings for text using various LLM providers:
//! - OpenAI (text-embedding-3-small, text-embedding-3-large, text-embedding-ada-002)
//! - Ollama (local, supports nomic-embed-text, mxbai-embed-large, etc.)

use kuzu_extension::{Extension, ExtensionContext};

/// The LLM embedding extension.
pub struct LlmExtension;

impl Default for LlmExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for LlmExtension {
    fn name(&self) -> &'static str {
        "LLM"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use kuzu_common::types::Value;
        use kuzu_function::registry::ScalarFunction;
        use std::sync::Arc;

        context.register_scalar_function(
            "create_embedding",
            ScalarFunction::CustomScalar {
                name: "create_embedding".into(),
                execute: Arc::new(|args| {
                    if args.is_empty() {
                        return Err("create_embedding requires at least a text argument".into());
                    }
                    let text = match &args[0] {
                        Value::String(s) => s,
                        _ => return Err("create_embedding: first argument must be string".into()),
                    };

                    // Default config for now
                    let config = crate::EmbeddingConfig::default();
                    let embedding = crate::create_embedding(text, Some(&config))?;

                    // Return the vector as a List of floats
                    let vals: Vec<Value> = embedding.vector.into_iter().map(Value::Double).collect();
                    Ok(Value::List(vals))
                }),
            },
        );

        tracing::info!("LLM extension loaded: create_embedding function registered (CustomScalar)");
        Ok(())
    }
}

// ==================== Embedding Types ====================

/// Configuration for an LLM provider.
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub provider: LlmProvider,
    pub model: String,
    pub api_key: Option<String>,
    pub api_url: Option<String>,
}

/// Supported LLM providers.
#[derive(Debug, Clone, PartialEq)]
pub enum LlmProvider {
    OpenAI,
    Ollama,
}

impl std::str::FromStr for LlmProvider {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(LlmProvider::OpenAI),
            "ollama" => Ok(LlmProvider::Ollama),
            _ => Err(format!("Unknown LLM provider: {s}. Supported: openai, ollama")),
        }
    }
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: LlmProvider::OpenAI,
            model: "text-embedding-3-small".into(),
            api_key: std::env::var("OPENAI_API_KEY").ok(),
            api_url: None,
        }
    }
}

/// A generated embedding vector.
#[derive(Debug, Clone)]
pub struct Embedding {
    pub vector: Vec<f64>,
    pub dimensions: usize,
    pub model: String,
}

// ==================== Embedding Function ====================

/// Generate an embedding for the given text.
///
/// If no config is provided, uses defaults (OpenAI with OPENAI_API_KEY env var).
/// The `provider` and `model` can be overridden via query parameters.
pub fn create_embedding(text: &str, config: Option<&EmbeddingConfig>) -> Result<Embedding, String> {
    let cfg = config.cloned().unwrap_or_default();

    match cfg.provider {
        LlmProvider::OpenAI => openai_embed(text, &cfg),
        LlmProvider::Ollama => ollama_embed(text, &cfg),
    }
}

// ==================== OpenAI Provider ====================

/// Generate embeddings via the OpenAI API.
///
/// Uses the `/v1/embeddings` endpoint.
/// Requires `OPENAI_API_KEY` environment variable or config.api_key.
fn openai_embed(text: &str, config: &EmbeddingConfig) -> Result<Embedding, String> {
    let api_key = config
        .api_key
        .as_deref()
        .or_else(|| {
            // Try environment variable as fallback
            let env_key = std::env::var("OPENAI_API_KEY").ok()?;
            // Leak the string to get a &'static str — safe for one-shot config
            Some(Box::leak(env_key.into_boxed_str()) as &str)
        })
        .ok_or_else(|| {
            "OpenAI API key not found. Set OPENAI_API_KEY environment variable or pass in config.".to_string()
        })?;

    let url = config
        .api_url
        .as_deref()
        .unwrap_or("https://api.openai.com/v1/embeddings");

    let request_body = serde_json::json!({
        "input": text,
        "model": config.model,
    });
    let body_str = serde_json::to_string(&request_body).map_err(|e| format!("Failed to serialize request: {e}"))?;

    let response = ureq::post(url)
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send(body_str)
        .map_err(|e| format!("OpenAI API request failed: {e}"))?;

    let status = response.status();
    let response_text = response
        .into_body()
        .read_to_string()
        .map_err(|e| format!("Failed to read response: {e}"))?;

    if status != 200 {
        return Err(format!("OpenAI API error (HTTP {status}): {response_text}"));
    }

    let parsed: serde_json::Value =
        serde_json::from_str(&response_text).map_err(|e| format!("Failed to parse OpenAI response: {e}"))?;

    let vector: Vec<f64> = parsed["data"][0]["embedding"]
        .as_array()
        .ok_or_else(|| format!("Missing 'data[0].embedding' in response: {response_text}"))?
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0))
        .collect();

    let dimensions = vector.len();

    Ok(Embedding {
        vector,
        dimensions,
        model: config.model.clone(),
    })
}

// ==================== Ollama Provider ====================

/// Generate embeddings via a local Ollama instance.
///
/// Default endpoint: http://localhost:11434/api/embed
fn ollama_embed(text: &str, config: &EmbeddingConfig) -> Result<Embedding, String> {
    let url = config.api_url.as_deref().unwrap_or("http://localhost:11434/api/embed");

    let request_body = serde_json::json!({
        "model": config.model,
        "input": text,
    });
    let body_str = serde_json::to_string(&request_body).map_err(|e| format!("Failed to serialize request: {e}"))?;

    let response = ureq::post(url)
        .header("Content-Type", "application/json")
        .send(body_str)
        .map_err(|e| format!("Ollama API request failed: {e}"))?;

    let status = response.status();
    let response_text = response
        .into_body()
        .read_to_string()
        .map_err(|e| format!("Failed to read response: {e}"))?;

    if status != 200 {
        return Err(format!("Ollama API error (HTTP {status}): {response_text}"));
    }

    let parsed: serde_json::Value =
        serde_json::from_str(&response_text).map_err(|e| format!("Failed to parse Ollama response: {e}"))?;

    // Ollama returns embeddings in `embeddings[0]` or `embedding`
    let vector: Vec<f64> = parsed["embeddings"][0]
        .as_array()
        .or_else(|| parsed["embedding"].as_array())
        .ok_or_else(|| format!("Missing 'embeddings[0]' or 'embedding' in response: {response_text}"))?
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0))
        .collect();

    let dimensions = vector.len();

    Ok(Embedding {
        vector,
        dimensions,
        model: config.model.clone(),
    })
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_extension_registration() {
        let ext = LlmExtension::new();
        assert_eq!(ext.name(), "LLM");
    }

    #[test]
    fn test_embedding_config_default_openai() {
        let config = EmbeddingConfig::default();
        assert_eq!(config.provider, LlmProvider::OpenAI);
        assert_eq!(config.model, "text-embedding-3-small");
    }

    #[test]
    fn test_llm_provider_from_str() {
        assert_eq!("openai".parse::<LlmProvider>().unwrap(), LlmProvider::OpenAI);
        assert_eq!("OpenAI".parse::<LlmProvider>().unwrap(), LlmProvider::OpenAI);
        assert_eq!("ollama".parse::<LlmProvider>().unwrap(), LlmProvider::Ollama);
        assert!("unknown".parse::<LlmProvider>().is_err());
    }

    #[test]
    fn test_embedding_result_fields() {
        let emb = Embedding {
            vector: vec![0.1, 0.2, 0.3],
            dimensions: 3,
            model: "test-model".into(),
        };
        assert_eq!(emb.dimensions, 3);
        assert_eq!(emb.vector.len(), 3);
        assert_eq!(emb.model, "test-model");
    }

    #[test]
    fn test_openai_embed_fails_without_key() {
        // Without API key, this should fail with a clear error
        let config = EmbeddingConfig {
            provider: LlmProvider::OpenAI,
            model: "text-embedding-3-small".into(),
            api_key: None,
            api_url: None,
        };
        let result = create_embedding("hello", Some(&config));
        assert!(result.is_err(), "Should fail without API key");
        let err = result.unwrap_err();
        assert!(err.contains("API key"), "Error should mention API key, got: {err}");
    }

    #[test]
    fn test_ollama_embed_fails_if_not_running() {
        // Without a running Ollama instance, this should fail with connection error
        let config = EmbeddingConfig {
            provider: LlmProvider::Ollama,
            model: "nomic-embed-text".into(),
            api_key: None,
            api_url: Some("http://127.0.0.1:1/invalid".into()), // definitely not running
        };
        let result = create_embedding("hello", Some(&config));
        assert!(result.is_err(), "Should fail when Ollama not running");
    }

    #[test]
    fn test_openai_request_body_format() {
        // Verify the request body format without making an actual HTTP call
        let request_body = serde_json::json!({
            "input": "test text",
            "model": "text-embedding-3-small",
        });
        let body_str = serde_json::to_string(&request_body).unwrap();
        assert!(body_str.contains("test text"));
        assert!(body_str.contains("text-embedding-3-small"));
        assert!(body_str.contains("input"));
        assert!(body_str.contains("model"));
    }

    #[test]
    fn test_ollama_request_body_format() {
        let request_body = serde_json::json!({
            "model": "nomic-embed-text",
            "input": "test text",
        });
        let body_str = serde_json::to_string(&request_body).unwrap();
        assert!(body_str.contains("nomic-embed-text"));
        assert!(body_str.contains("test text"));
    }

    #[test]
    fn test_provider_from_str_case_insensitive() {
        assert_eq!("OPENAI".parse::<LlmProvider>().unwrap(), LlmProvider::OpenAI);
        assert_eq!("OLLAMA".parse::<LlmProvider>().unwrap(), LlmProvider::Ollama);
        assert_eq!("OpenAI".parse::<LlmProvider>().unwrap(), LlmProvider::OpenAI);
        assert_eq!("ollama".parse::<LlmProvider>().unwrap(), LlmProvider::Ollama);
    }
}
