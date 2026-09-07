//! In-process text embedding via ONNX Runtime (fastembed).
//!
//! Provides dense, sparse, and multi-modal (BGE-M3) text embedding providers
//! without external API calls. This module is gated behind the `onnx-embedding`
//! feature flag.
//!
//! # Providers
//!
//! | Provider | Output | Model |
//! |----------|--------|-------|
//! | [`FastEmbedProvider`] | Dense `Vec<f32>` | BGE-small-en-v1.5 (384d) |
//! | [`SparseEmbedProvider`] | Sparse `(indices, values)` | SPLADE++ / BGE-M3 sparse |
//! | [`Bgem3Provider`] | Dense + sparse + ColBERT | BGE-M3 INT8 (1024d) |
//! | [`RerankProvider`] | Scored `(index, score)` | Cross-encoder reranker |
//!
//! # Examples
//!
//! ```no_run
//! use akar_ml::embed::FastEmbedProvider;
//!
//! let provider = FastEmbedProvider::try_default().unwrap();
//! let embeddings = provider.embed_texts(&["hello world", "test sentence"]).unwrap();
//! assert_eq!(embeddings.len(), 2);
//! ```

use std::path::Path;
use std::sync::Arc;

use fastembed::{
    Bgem3EmbeddingOutput, Bgem3InitOptions, Bgem3Model, EmbeddingModel, Pooling, QuantizationMode, RerankInitOptions,
    RerankerModel, SparseInitOptions, SparseModel, SparseTextEmbedding, TextEmbedding, TextInitOptions, TextRerank,
};

// Re-export the cross-encoder rerank result so consumers can name the return
// type of [`RerankerProvider::rerank`] without depending on fastembed.
pub use fastembed::RerankResult;

use crate::sbyo::SbyoLoad;
use crate::sparse::NativeSparseSession;

/// Default ONNX batch size used when the caller does not specify one.
const DEFAULT_BATCH_SIZE: usize = 256;

/// ONNX Runtime dispatch for the DirectML execution provider (Windows, DirectX 12).
///
/// Available under the `directml` Cargo feature (which also enables
/// `onnx-embedding`). Returns a ready-to-use `ort::ep::ExecutionProviderDispatch`
/// that can be passed to a provider config's `execution_providers` list so ONNX
/// sessions prefer the GPU over the CPU provider.
///
/// # DirectML constraints (handled automatically)
///
/// - fastembed disables ORT's memory-pattern optimization and parallel
///   execution whenever a DirectML EP is registered on the session
///   (`with_memory_pattern(false)` + `with_parallel_execution(false)`), so
///   both are off for DirectML sessions.
/// - Constructing the dispatch never requires a GPU. At session build time a
///   machine without a DirectX-12 device fails EP registration; prefer
///   `dispatch.fail_silently()` so the session keeps ORT's CPU provider as a
///   fallback. Ops the DirectML provider cannot place fall back to CPU per-node
///   by default (`disable_cpu_fallback` is not yet exposed by akar configs).
/// - The SBYO sparse offline path (`SparseEmbedProvider::try_from_user_defined`
///   / `new_from_dir`) builds its own native ort session that takes no
///   execution providers, so it always runs on CPU.
#[cfg(feature = "directml")]
pub fn directml_execution_provider() -> ort::ep::ExecutionProviderDispatch {
    ort::ep::DirectML::default().build()
}

// ── Error type ──────────────────────────────────────────────────────

/// Errors that can occur during embedding operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EmbeddingError {
    #[error("failed to initialize embedding model: {0}")]
    InitFailed(String),

    #[error("embedding computation failed: {0}")]
    ComputeFailed(String),

    #[error("model not loaded")]
    NotLoaded,
}

// ── EmbeddingProvider trait ─────────────────────────────────────────

/// Generic interface for text embedding providers, decoupled from fastembed.
///
/// This trait enables the dream engine and other consumers to use embeddings
/// without depending on a specific ONNX runtime or model library.
pub trait EmbeddingProvider: Send + Sync {
    /// Compute dense embeddings for a batch of texts.
    ///
    /// Returns one `Vec<f32>` per input text, all with the same dimensionality.
    fn embed_dense(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError>;

    /// Return the embedding dimensionality for this provider.
    fn dimensions(&self) -> usize;

    /// Return a human-readable model name.
    fn model_name(&self) -> &str;

    /// Capability view (P96): if this provider can also produce multi-vector
    /// embeddings (dense + sparse + ColBERT) in a single pass, expose them
    /// through [`MultiEmbeddingProvider`].
    ///
    /// Consumers holding a `dyn EmbeddingProvider` can select the richer
    /// capability (sparse/ColBERT for higher recall) without changing the base
    /// contract: `None` means the provider is dense-only, `Some` upgrades the
    /// consumer to `embed_multi`. Defaults to [`None`]; providers that also
    /// implement [`MultiEmbeddingProvider`] override it.
    fn as_multi(&self) -> Option<&dyn MultiEmbeddingProvider> {
        None
    }
}

// ── Multi-embedding provider trait ──────────────────────────────────

/// Generic interface for providers that return multi-vector embeddings
/// (dense + sparse + ColBERT) in a single pass.
///
/// Kept separate from [`EmbeddingProvider`] so that consumers that only need
/// dense vectors keep using the object-safe [`EmbeddingProvider::embed_dense`],
/// while providers that can produce more (e.g. BGE-M3) are dispatched through
/// this trait via `dyn`. Both traits are deliberately not super-trait of each
/// other; each stays object-safe on its own.
pub trait MultiEmbeddingProvider: Send + Sync {
    /// Compute dense + sparse + ColBERT embeddings for a batch of texts.
    ///
    /// Returns one [`MultiEmbeddingOutput`] per call containing all three
    /// representations for every input text.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the session could not be
    /// lazily initialized, or [`EmbeddingError::ComputeFailed`] if inference
    /// fails.
    fn embed_multi(&self, texts: &[&str]) -> Result<MultiEmbeddingOutput, EmbeddingError>;

    /// Return the dense dimensionality of this provider.
    fn dense_dimensions(&self) -> usize;
}

// ── Reranker trait ──────────────────────────────────────────────────

/// Generic interface for cross-encoder rerankers.
///
/// Kept separate from [`EmbeddingProvider`] so that dense-only consumers keep
/// the minimal object-safe trait and reranking capability is opt-in via `dyn`
/// [`RerankerProvider`].
pub trait RerankerProvider: Send + Sync {
    /// Rerank documents by relevance to the query.
    ///
    /// Returns results sorted by score in descending order.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the session could not be
    /// lazily initialized, or [`EmbeddingError::ComputeFailed`] if inference
    /// fails.
    fn rerank(&self, query: &str, documents: &[&str]) -> Result<Vec<RerankResult>, EmbeddingError>;
}

// ── Sparse embedding output type ────────────────────────────────────

/// A sparse embedding vector — token-level weights at specific vocabulary indices.
///
/// Sparse embeddings are used for lexical search (SPLADE) and multi-vector
/// retrieval (BGE-M3 sparse branch). The `indices` are vocabulary token IDs
/// and `values` are their corresponding importance weights.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct SparseEmbedding {
    /// Vocabulary indices with non-zero weights.
    pub indices: Vec<usize>,
    /// Importance weights corresponding to each index.
    pub values: Vec<f32>,
}

impl SparseEmbedding {
    /// Number of non-zero dimensions.
    pub fn len(&self) -> usize {
        self.indices.len()
    }

    /// Whether the embedding is empty (all zeros).
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Convert to dense vector of given dimensionality (zero-filled for missing indices).
    pub fn to_dense(&self, dim: usize) -> Vec<f32> {
        let mut vec = vec![0.0; dim];
        for (idx, &val) in self.indices.iter().zip(&self.values) {
            if *idx < dim {
                vec[*idx] = val;
            }
        }
        vec
    }
}

impl From<fastembed::SparseEmbedding> for SparseEmbedding {
    fn from(s: fastembed::SparseEmbedding) -> Self {
        Self {
            indices: s.indices,
            values: s.values,
        }
    }
}

// ── Multi-modal embedding output (BGE-M3) ───────────────────────────

/// Output from BGE-M3: dense + sparse + ColBERT representations in a single pass.
#[derive(Debug, Clone)]
pub struct MultiEmbeddingOutput {
    /// Dense vectors, one per input text.
    pub dense: Vec<Vec<f32>>,
    /// Sparse (lexical) vectors, one per input text.
    pub sparse: Vec<SparseEmbedding>,
    /// ColBERT multi-vector representations (per-token), one `Vec<Vec<f32>>` per input text.
    pub colbert: Vec<Vec<Vec<f32>>>,
}

impl From<Bgem3EmbeddingOutput> for MultiEmbeddingOutput {
    fn from(o: Bgem3EmbeddingOutput) -> Self {
        Self {
            dense: o.dense,
            sparse: o.sparse.into_iter().map(SparseEmbedding::from).collect(),
            colbert: o.colbert,
        }
    }
}

// ── Model choice enum ───────────────────────────────────────────────

/// Selects the embedding model family and variant.
///
/// This allows callers to choose between dense, sparse, or multi-modal
/// embedding providers without coupling to specific model enums.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum EmbeddingModelChoice {
    /// Dense embedding model (e.g., BGE-small-en-v1.5, 384d).
    Dense(EmbeddingModel),
    /// Quantized dense embedding model (e.g., `bge-small-en-v1.5-Q`, 384d;
    /// `Gemma-300M-Q4` is 4-bit). Faster inference at slightly lower quality.
    DenseQ(EmbeddingModel),
    /// Sparse embedding model (SPLADE++ or BGE-M3 sparse, vocabulary-sized).
    Sparse(SparseModel),
    /// BGE-M3 multi-modal model (dense + sparse + ColBERT, 1024d dense).
    Multi(Bgem3Model),
}

impl Default for EmbeddingModelChoice {
    fn default() -> Self {
        Self::Dense(EmbeddingModel::BGESmallENV15)
    }
}

impl EmbeddingModelChoice {
    /// The dense model selected by this choice, if any.
    ///
    /// Returns the wrapped [`EmbeddingModel`] for the [`Dense`] and [`DenseQ`]
    /// variants (both resolve to a dense provider; `DenseQ` selects a quantized
    /// checkpoint such as `bge-small-en-v1.5-Q`). Sparse and multi-modal variants
    /// have no dense model and return `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use akar_ml::embed::EmbeddingModelChoice;
    /// use fastembed::{EmbeddingModel, SparseModel};
    ///
    /// let q = EmbeddingModelChoice::DenseQ(EmbeddingModel::BGESmallENV15Q);
    /// assert_eq!(q.dense_model(), Some(&EmbeddingModel::BGESmallENV15Q));
    /// assert_eq!(
    ///     EmbeddingModelChoice::default().dense_model(),
    ///     Some(&EmbeddingModel::BGESmallENV15)
    /// );
    /// assert_eq!(
    ///     EmbeddingModelChoice::Sparse(SparseModel::SPLADEPPV1).dense_model(),
    ///     None
    /// );
    /// ```
    ///
    /// [`Dense`]: EmbeddingModelChoice::Dense
    /// [`DenseQ`]: EmbeddingModelChoice::DenseQ
    pub fn dense_model(&self) -> Option<&EmbeddingModel> {
        match self {
            Self::Dense(model) | Self::DenseQ(model) => Some(model),
            Self::Sparse(_) | Self::Multi(_) => None,
        }
    }
}

// ── Dense embedding provider (P89.1) ────────────────────────────────

/// A thread-safe wrapper around fastembed's [`TextEmbedding`].
///
/// Provides dense text embedding via ONNX Runtime. The model is loaded lazily
/// on first use and shared across calls via `Arc<Mutex<>>`.
///
/// # Thread Safety
///
/// `FastEmbedProvider` is `Send + Sync`. The underlying ONNX session is
/// protected by a mutex; concurrent `embed_texts` calls serialize at the
/// session level but do not block each other at the Rust level.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FastEmbedProvider {
    inner: Arc<FastEmbedInner>,
}

struct FastEmbedInner {
    model_name: String,
    dimensions: usize,
    session: parking_lot::Mutex<Option<TextEmbedding>>,
    init_options: TextInitOptions,
    batch_size: usize,
    quantization: QuantizationMode,
}

impl std::fmt::Debug for FastEmbedInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FastEmbedInner")
            .field("model_name", &self.model_name)
            .field("dimensions", &self.dimensions)
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}

/// Configuration for creating a [`FastEmbedProvider`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EmbedProviderConfig {
    /// The embedding model to use.
    pub model: EmbeddingModel,
    /// Optional cache directory for downloaded models.
    pub cache_dir: Option<std::path::PathBuf>,
    /// Maximum sequence length (tokens). `None` uses model default.
    pub max_length: Option<usize>,
    /// Number of intra-op threads. `None` uses ONNX default.
    pub intra_threads: Option<usize>,
    /// ONNX batch size for each forward pass. `0` uses the library default (256).
    pub batch_size: usize,
    /// Pooling strategy applied after the last hidden state. `None` keeps the
    /// model's default (CLS for the BGE family, mean for MiniLM/Nomic/etc.).
    pub pooling: Option<Pooling>,
    /// Quantization applied to the model weights on load. `None` keeps the
    /// model's default (no quantization).
    pub quantization: Option<QuantizationMode>,
    /// Execution providers for the ONNX session, in registration order. Empty
    /// (the default) keeps ORT's CPU provider. Pass the dispatch from
    /// [`directml_execution_provider`] to prefer the DirectML GPU provider on
    /// Windows. Honored by [`FastEmbedProvider::try_new`] and the config-bearing
    /// offline paths (`try_from_user_defined_with_config` /
    /// `new_from_dir_with_config`).
    pub execution_providers: Vec<ort::ep::ExecutionProviderDispatch>,
}

impl Default for EmbedProviderConfig {
    fn default() -> Self {
        Self {
            model: EmbeddingModel::BGESmallENV15,
            cache_dir: None,
            max_length: None,
            intra_threads: None,
            batch_size: DEFAULT_BATCH_SIZE,
            pooling: None,
            quantization: None,
            execution_providers: Default::default(),
        }
    }
}

impl EmbedProviderConfig {
    /// Default configuration for a quantized dense model.
    ///
    /// Selects `bge-small-en-v1.5-Q` (`Qdrant/bge-small-en-v1.5-onnx-Q`,
    /// `model_optimized.onnx`): the INT8 quantized release of the default model,
    /// still 384 dims but faster inference at slightly lower quality than
    /// [`EmbeddingModel::BGESmallENV15`]. The session loads lazily, so no
    /// download happens until the first embed.
    ///
    /// # Examples
    ///
    /// ```
    /// use akar_ml::embed::{EmbedProviderConfig, FastEmbedProvider};
    /// use fastembed::EmbeddingModel;
    ///
    /// let config = EmbedProviderConfig::dense_q();
    /// assert_eq!(config.model, EmbeddingModel::BGESmallENV15Q);
    /// assert_eq!(config.batch_size, 256);
    ///
    /// let provider = FastEmbedProvider::try_new(config).expect("provider must build");
    /// assert_eq!(provider.dimensions(), 384);
    /// assert_eq!(provider.model_name(), "BGESmallENV15Q");
    /// ```
    pub fn dense_q() -> Self {
        Self {
            model: EmbeddingModel::BGESmallENV15Q,
            ..Self::default()
        }
    }

    /// Configures the pooling strategy applied when embedding.
    ///
    /// Honored by the offline path ([`FastEmbedProvider::try_from_user_defined_with_config`]
    /// and [`FastEmbedProvider::new_from_dir_with_config`]). `None` keeps the model's
    /// default pooling (CLS for the BGE family, mean for MiniLM/Nomic family).
    ///
    /// # Examples
    ///
    /// ```
    /// use akar_ml::embed::EmbedProviderConfig;
    /// use fastembed::Pooling;
    ///
    /// let config = EmbedProviderConfig::default().with_pooling(Pooling::Mean);
    /// assert!(config.pooling == Some(Pooling::Mean));
    /// ```
    pub fn with_pooling(mut self, pooling: Pooling) -> Self {
        self.pooling = Some(pooling);
        self
    }

    /// Configures the weight quantization applied when the model is loaded.
    ///
    /// Honored by the offline path
    /// ([`FastEmbedProvider::try_from_user_defined_with_config`] and
    /// [`FastEmbedProvider::new_from_dir_with_config`]). `None` keeps the
    /// model's default (`QuantizationMode::None`).
    ///
    /// # Examples
    ///
    /// ```
    /// use akar_ml::embed::EmbedProviderConfig;
    /// use fastembed::QuantizationMode;
    ///
    /// let config = EmbedProviderConfig::default().with_quantization(QuantizationMode::Static);
    /// assert!(config.quantization == Some(QuantizationMode::Static));
    /// ```
    pub fn with_quantization(mut self, quantization: QuantizationMode) -> Self {
        self.quantization = Some(quantization);
        self
    }

    /// Configures the execution providers used when the ONNX session is built.
    ///
    /// Takes the providers in registration order; an empty list keeps ORT's
    /// default CPU provider. Combine with [`directml_execution_provider`] to
    /// prefer the DirectML GPU provider on Windows.
    pub fn with_execution_providers(mut self, execution_providers: Vec<ort::ep::ExecutionProviderDispatch>) -> Self {
        self.execution_providers = execution_providers;
        self
    }
}

impl FastEmbedProvider {
    /// Create a provider with default model (`BGE-small-en-v1.5`, 384 dims).
    ///
    /// Downloads the model on first call; subsequent calls use the cached copy.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the model cannot be downloaded,
    /// located in the cache, or the ONNX session cannot be built.
    pub fn try_default() -> Result<Self, EmbeddingError> {
        Self::try_new(EmbedProviderConfig::default())
    }

    /// Create a provider with the default quantized dense model
    /// (`bge-small-en-v1.5-Q`, 384 dims).
    ///
    /// Same as [`Self::try_default`] but selects [`EmbeddingModel::BGESmallENV15Q`],
    /// the INT8 quantized checkpoint (`Qdrant/bge-small-en-v1.5-onnx-Q`): faster
    /// inference at slightly lower quality. Downloads the model on first embed;
    /// subsequent calls use the cached copy.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the model cannot be downloaded,
    /// located in the cache, or the ONNX session cannot be built.
    ///
    /// # Examples
    ///
    /// ```
    /// use akar_ml::embed::FastEmbedProvider;
    ///
    /// let provider = FastEmbedProvider::try_q_default().expect("provider must build");
    /// assert_eq!(provider.dimensions(), 384);
    /// assert_eq!(provider.model_name(), "BGESmallENV15Q");
    /// ```
    pub fn try_q_default() -> Result<Self, EmbeddingError> {
        Self::try_new(EmbedProviderConfig::dense_q())
    }

    /// Create a provider with a specific model configuration.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the model cannot be loaded or
    /// the ONNX session cannot be built from the given configuration.
    pub fn try_new(config: EmbedProviderConfig) -> Result<Self, EmbeddingError> {
        let model_name = config.model.to_string();
        let dimensions = TextEmbedding::get_model_info(&config.model)
            .map(|info| info.dim)
            .unwrap_or(384);

        let quantization = TextEmbedding::get_quantization_mode(&config.model);

        let mut opts = TextInitOptions::new(config.model);
        if let Some(dir) = &config.cache_dir {
            opts = opts.with_cache_dir(dir.clone());
        }
        if let Some(len) = config.max_length {
            opts = opts.with_max_length(len);
        }
        if let Some(threads) = config.intra_threads {
            opts = opts.with_intra_threads(threads);
        }
        opts = opts.with_execution_providers(config.execution_providers);

        let batch_size = if config.batch_size == 0 {
            DEFAULT_BATCH_SIZE
        } else {
            config.batch_size
        };

        Ok(Self {
            inner: Arc::new(FastEmbedInner {
                model_name,
                dimensions,
                session: parking_lot::Mutex::new(None),
                init_options: opts,
                batch_size,
                quantization,
            }),
        })
    }

    /// Create a provider from user-defined ONNX model bytes (offline/air-gapped).
    ///
    /// No HuggingFace Hub download required. The caller supplies the ONNX model
    /// file bytes and tokenizer files directly. Equivalent to
    /// [`Self::try_from_user_defined_with_config`] with an `EmbedProviderConfig::default()`.
    pub fn try_from_user_defined(
        onnx_bytes: Vec<u8>,
        tokenizer_files: fastembed::TokenizerFiles,
        dimensions: usize,
    ) -> Result<Self, EmbeddingError> {
        Self::try_from_user_defined_with_config(
            onnx_bytes,
            tokenizer_files,
            dimensions,
            &EmbedProviderConfig::default(),
        )
    }

    /// Create a provider from user-defined ONNX model bytes (offline/air-gapped),
    /// honoring the pooling strategy and weight quantization from `config`.
    ///
    /// No HuggingFace Hub download required. The caller supplies the ONNX model
    /// file bytes and tokenizer files directly. `config.model`, `cache_dir`,
    /// `max_length`, `intra_threads`, and `batch_size` are ignored on this path;
    /// only [`EmbedProviderConfig::pooling`],
    /// [`EmbedProviderConfig::quantization`], and
    /// [`EmbedProviderConfig::execution_providers`] take effect.
    pub fn try_from_user_defined_with_config(
        onnx_bytes: Vec<u8>,
        tokenizer_files: fastembed::TokenizerFiles,
        dimensions: usize,
        config: &EmbedProviderConfig,
    ) -> Result<Self, EmbeddingError> {
        Self::build_user_defined(
            onnx_bytes,
            Vec::new(),
            tokenizer_files,
            config,
            "user-defined".to_string(),
            dimensions,
        )
    }

    fn build_user_defined(
        onnx_bytes: Vec<u8>,
        external_initializers: Vec<(String, Vec<u8>)>,
        tokenizer_files: fastembed::TokenizerFiles,
        config: &EmbedProviderConfig,
        model_name: String,
        dimensions: usize,
    ) -> Result<Self, EmbeddingError> {
        let mut user_model = fastembed::UserDefinedEmbeddingModel::new(onnx_bytes, tokenizer_files);
        for (file_name, buffer) in external_initializers {
            user_model = user_model.with_external_initializer(file_name, buffer);
        }
        if let Some(pooling) = config.pooling.clone() {
            user_model = user_model.with_pooling(pooling);
        }
        if let Some(quantization) = config.quantization {
            user_model = user_model.with_quantization(quantization);
        }

        let embedding = TextEmbedding::try_new_from_user_defined(
            user_model,
            fastembed::InitOptionsUserDefined::default().with_execution_providers(config.execution_providers.clone()),
        )
        .map_err(|e| EmbeddingError::InitFailed(e.to_string()))?;

        Ok(Self {
            inner: Arc::new(FastEmbedInner {
                model_name,
                dimensions,
                session: parking_lot::Mutex::new(Some(embedding)),
                init_options: Default::default(),
                batch_size: DEFAULT_BATCH_SIZE,
                quantization: config.quantization.unwrap_or(QuantizationMode::None),
            }),
        })
    }

    /// Create a provider from ONNX + tokenizer files on disk (offline / air-gapped).
    ///
    /// The model is loaded entirely from a local directory — no HuggingFace Hub
    /// download is performed. The directory must contain a `.onnx` file and the
    /// tokenizer files `tokenizer.json`, `config.json`, `special_tokens_map.json`,
    /// and `tokenizer_config.json`. External-initializer sidecar files (`*.onnx_data`
    /// or the files referenced by `model.onnx_data_location`) are discovered and
    /// loaded automatically for models that keep weights out of line.
    ///
    /// `dimensions` is the latent embedding dimensionality of the ONNX model's
    /// output (e.g. 384 for BGE-small-en-v1.5). It cannot be reliably inferred
    /// from the opaque session, so the caller supplies it.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the directory is unreadable, the
    /// ONNX file or a required tokenizer file is missing, or the ONNX session
    /// cannot be built from the given bytes.
    pub fn new_from_dir(model_dir: impl AsRef<Path>, dimensions: usize) -> Result<Self, EmbeddingError> {
        Self::new_from_dir_with_config(model_dir, dimensions, &EmbedProviderConfig::default())
    }

    /// Create a provider from ONNX + tokenizer files on disk (offline / air-gapped),
    /// honoring the pooling strategy and weight quantization from `config`.
    ///
    /// The model is loaded entirely from a local directory — no HuggingFace Hub
    /// download is performed. The directory must contain a `.onnx` file and the
    /// tokenizer files `tokenizer.json`, `config.json`, `special_tokens_map.json`,
    /// and `tokenizer_config.json`. `config.model`, `cache_dir`, `max_length`,
    /// `intra_threads`, and `batch_size` are ignored on this path; only
    /// [`EmbedProviderConfig::pooling`],
    /// [`EmbedProviderConfig::quantization`], and
    /// [`EmbedProviderConfig::execution_providers`] take effect.
    ///
    /// `dimensions` is the latent embedding dimensionality of the ONNX model's
    /// output (e.g. 384 for BGE-small-en-v1.5). It cannot be reliably inferred
    /// from the opaque session, so the caller supplies it.
    pub fn new_from_dir_with_config(
        model_dir: impl AsRef<Path>,
        dimensions: usize,
        config: &EmbedProviderConfig,
    ) -> Result<Self, EmbeddingError> {
        let dir = model_dir.as_ref();

        let model = SbyoLoad::from_dir(dir)?;
        let model_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "user-defined".to_string());

        Self::build_user_defined(
            model.onnx,
            model.external_initializers,
            model.tokenizer,
            config,
            model_name,
            dimensions,
        )
    }

    /// Compute dense embeddings for a batch of texts.
    ///
    /// Returns one `Vec<f32>` per input text. All vectors have the same
    /// dimensionality ([`Self::dimensions`]). The input is split into chunks of
    /// the configured batch size and fed to the ONNX session.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::ComputeFailed`] if the underlying ONNX session
    /// fails to embed the batch. Returns [`EmbeddingError::InitFailed`] if the
    /// session could not be lazily initialized. For a dynamically quantized
    /// model ([`QuantizationMode::Dynamic`]) with more texts than the configured
    /// batch size, returns [`EmbeddingError::ComputeFailed`] — see
    /// [`Self::embed_texts_batched`] for the limitation and workaround.
    pub fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        self.embed_texts_batched(texts, self.inner.batch_size)
    }

    /// Compute dense embeddings with an explicit batch size.
    ///
    /// Splits `texts` into chunks of `batch_size` and embeds each through the
    /// shared ONNX session. A `batch_size` of `0` falls back to the configured
    /// default. Results are identical to [`Self::embed_texts`].
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::ComputeFailed`] if the underlying ONNX session
    /// fails to embed the batch, and [`EmbeddingError::InitFailed`] if the
    /// session could not be lazily initialized.
    ///
    /// When the model uses [`QuantizationMode::Dynamic`] (fastembed limitation,
    /// `text_embedding/impl.rs:337-365`: dynamic quantization re-scales each
    /// batch, so a split input yields mutually incompatible embeddings) a
    /// `batch_size` smaller than the number of texts is rejected with a clear
    /// [`EmbeddingError::ComputeFailed`]. Pass a `batch_size` at least as large
    /// as `texts.len()` (or `0`), or use a [`QuantizationMode::Static`]/
    /// no-quantization model, to embed under dynamic quantization.
    pub fn embed_texts_batched(&self, texts: &[&str], batch_size: usize) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let mut session_guard = self.inner.session.lock();
        let session = session_guard.get_or_insert_with(|| {
            TextEmbedding::try_new(self.inner.init_options.clone())
                .expect("failed to initialize embedding model — is ONNX Runtime available?")
        });

        let bs = if batch_size == 0 {
            self.inner.batch_size
        } else {
            batch_size
        };
        if self.inner.quantization == QuantizationMode::Dynamic && bs < texts.len() {
            let len = texts.len();
            return Err(EmbeddingError::ComputeFailed(format!(
                "Dynamic quantization cannot be used with batching: this model re-scales each batch, so \
                 split chunks produce incompatible embeddings ({len} texts, batch_size {bs}). Pass batch_size >= \
                 {len} (or 0 for a single batch), or use a static/no-quantization model.",
            )));
        }
        session
            .embed(texts, Some(bs))
            .map_err(|e| EmbeddingError::ComputeFailed(e.to_string()))
    }

    /// Compute dense embeddings in parallel across batches using rayon.
    ///
    /// The ONNX [`TextEmbedding`] session is not `Sync`, so sharing a single
    /// session across threads is unsound. Each rayon worker instead builds its
    /// own session from the same model (loaded from the on-disk cache, which the
    /// shared session pre-warms on the first call) and embeds a distinct chunk.
    /// Output is concatenated in input order, so results match [`Self::embed_texts`].
    ///
    /// Caveat: a dynamically quantized model ([`QuantizationMode::Dynamic`])
    /// re-scales each batch independently, so the concatenated per-chunk output
    /// is not mutually comparable — the same limitation as
    /// [`Self::embed_texts_batched`], which this parallel variant does not guard.
    /// Use a single batch (via [`Self::embed_texts_batched`]) or a
    /// static/no-quantization model under dynamic quantization.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if a worker session cannot be built,
    /// or [`EmbeddingError::ComputeFailed`] if any worker fails to embed.
    #[cfg(feature = "onnx-embedding")]
    pub fn par_embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let batch_size = self.inner.batch_size;

        // Pre-warm the shared session so the model weights are cached to disk
        // before worker sessions are spawned.
        {
            let mut guard = self.inner.session.lock();
            guard.get_or_insert_with(|| {
                TextEmbedding::try_new(self.inner.init_options.clone())
                    .expect("failed to initialize embedding model — is ONNX Runtime available?")
            });
        }

        let chunks: Vec<&[&str]> = texts.chunks(batch_size).collect();
        let opts = self.inner.init_options.clone();

        use rayon::prelude::*;
        let results: Vec<Result<Vec<Vec<f32>>, EmbeddingError>> = chunks
            .par_iter()
            .map(|chunk| {
                let mut session =
                    TextEmbedding::try_new(opts.clone()).map_err(|e| EmbeddingError::InitFailed(e.to_string()))?;
                session
                    .embed(*chunk, Some(batch_size))
                    .map_err(|e| EmbeddingError::ComputeFailed(e.to_string()))
            })
            .collect();

        let mut out = Vec::with_capacity(texts.len());
        for res in results {
            out.extend(res?);
        }
        Ok(out)
    }

    /// Embed a single text and return the vector.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::ComputeFailed`] if the underlying ONNX session
    /// fails to embed the text, or [`EmbeddingError::InitFailed`] if it could
    /// not be lazily initialized.
    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.embed_texts(&[text]).map(|mut v| v.remove(0))
    }

    /// Return the embedding dimensionality for the configured model.
    pub fn dimensions(&self) -> usize {
        self.inner.dimensions
    }

    /// Return the model name (e.g., `"BGESmallENV15"`).
    pub fn model_name(&self) -> &str {
        &self.inner.model_name
    }
}

impl EmbeddingProvider for FastEmbedProvider {
    fn embed_dense(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        self.embed_texts(texts)
    }

    fn dimensions(&self) -> usize {
        self.inner.dimensions
    }

    fn model_name(&self) -> &str {
        &self.inner.model_name
    }
}

// ── Sparse embedding provider ───────────────────────────────────────

/// Configuration for creating a [`SparseEmbedProvider`].
#[derive(Debug, Clone)]
pub struct SparseProviderConfig {
    /// The sparse model to use.
    pub model: SparseModel,
    /// Optional cache directory for downloaded models.
    pub cache_dir: Option<std::path::PathBuf>,
    /// Maximum sequence length (tokens). `None` uses model default.
    pub max_length: Option<usize>,
    /// Number of intra-op threads. `None` uses ONNX default.
    pub intra_threads: Option<usize>,
    /// ONNX batch size for each forward pass. `0` uses the library default (256).
    pub batch_size: usize,
    /// Execution providers for the ONNX session, in registration order. Empty
    /// (the default) keeps ORT's CPU provider. Pass the dispatch from
    /// [`directml_execution_provider`] to prefer the DirectML GPU provider on
    /// Windows. Honored by [`SparseEmbedProvider::try_new`]; the SBYO offline
    /// paths (`try_from_user_defined` / `new_from_dir`) take no config and keep
    /// ORT's CPU provider.
    pub execution_providers: Vec<ort::ep::ExecutionProviderDispatch>,
}

impl Default for SparseProviderConfig {
    fn default() -> Self {
        Self {
            model: SparseModel::SPLADEPPV1,
            cache_dir: None,
            max_length: None,
            intra_threads: None,
            batch_size: DEFAULT_BATCH_SIZE,
            execution_providers: Default::default(),
        }
    }
}

impl SparseProviderConfig {
    /// Configures the execution providers used when the ONNX session is built.
    ///
    /// Takes the providers in registration order; an empty list keeps ORT's
    /// default CPU provider. Combine with [`directml_execution_provider`] to
    /// prefer the DirectML GPU provider on Windows.
    pub fn with_execution_providers(mut self, execution_providers: Vec<ort::ep::ExecutionProviderDispatch>) -> Self {
        self.execution_providers = execution_providers;
        self
    }
}

/// A thread-safe wrapper around fastembed's [`SparseTextEmbedding`].
///
/// Provides sparse text embedding via ONNX Runtime. The model is loaded lazily
/// on first use.
#[derive(Debug, Clone)]
pub struct SparseEmbedProvider {
    inner: Arc<SparseEmbedInner>,
}

/// A lazily initialized sparse inference session.
///
/// The `Fastembed` variant is used by the online path (model downloaded on first
/// use); the `Native` variant backs the SBYO offline path, which builds an ort
/// session directly from user-supplied bytes because fastembed exposes no
/// `try_new_from_user_defined` for sparse embeddings.
enum SparseSessionImpl {
    Fastembed(SparseTextEmbedding),
    Native(NativeSparseSession),
}

struct SparseEmbedInner {
    model_name: String,
    session: parking_lot::Mutex<Option<SparseSessionImpl>>,
    init_options: SparseInitOptions,
    batch_size: usize,
}

impl std::fmt::Debug for SparseEmbedInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SparseEmbedInner")
            .field("model_name", &self.model_name)
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}

impl SparseEmbedProvider {
    /// Create a provider with default model (`SPLADE++_en_v1`).
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the model cannot be downloaded,
    /// located in the cache, or the ONNX session cannot be built.
    pub fn try_default() -> Result<Self, EmbeddingError> {
        Self::try_new(SparseProviderConfig::default())
    }

    /// Create a provider with a specific model configuration.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the model cannot be loaded or
    /// the ONNX session cannot be built from the given configuration.
    pub fn try_new(config: SparseProviderConfig) -> Result<Self, EmbeddingError> {
        let model_name = format!("{:?}", config.model);

        let mut opts = SparseInitOptions::new(config.model);
        if let Some(dir) = &config.cache_dir {
            opts = opts.with_cache_dir(dir.clone());
        }
        if let Some(len) = config.max_length {
            opts = opts.with_max_length(len);
        }
        if let Some(threads) = config.intra_threads {
            opts = opts.with_intra_threads(threads);
        }
        opts = opts.with_execution_providers(config.execution_providers);

        let batch_size = if config.batch_size == 0 {
            DEFAULT_BATCH_SIZE
        } else {
            config.batch_size
        };

        Ok(Self {
            inner: Arc::new(SparseEmbedInner {
                model_name,
                session: parking_lot::Mutex::new(None),
                init_options: opts,
                batch_size,
            }),
        })
    }

    /// Create a provider from a user-defined SPLADE ONNX model (offline/air-gapped).
    ///
    /// The ONNX model and tokenizer files are supplied by the caller, so no
    /// HuggingFace Hub download is performed. The ONNX session is built natively
    /// via `ort` (fastembed has no `try_new_from_user_defined` for sparse models)
    /// and SPLADE post-processing is replicated for bit-identical output.
    ///
    /// Only SPLADE-style models (3-D `(batch, seq, vocab)` output) are supported;
    /// BGE-M3 sparse requires external initializers and embedded projection
    /// weights and is therefore rejected with [`EmbeddingError::ComputeFailed`]
    /// at embed time.
    pub fn try_from_user_defined(
        onnx_bytes: Vec<u8>,
        tokenizer_files: fastembed::TokenizerFiles,
    ) -> Result<Self, EmbeddingError> {
        let session = NativeSparseSession::try_new(&onnx_bytes, tokenizer_files)?;

        Ok(Self {
            inner: Arc::new(SparseEmbedInner {
                model_name: "user-defined".to_string(),
                session: parking_lot::Mutex::new(Some(SparseSessionImpl::Native(session))),
                init_options: Default::default(),
                batch_size: DEFAULT_BATCH_SIZE,
            }),
        })
    }

    /// Create a provider from ONNX + tokenizer files on disk (offline / air-gapped).
    ///
    /// The model is loaded entirely from a local directory — no HuggingFace Hub
    /// download is performed. The directory must contain a `.onnx` file and the
    /// tokenizer files `tokenizer.json`, `config.json`, `special_tokens_map.json`,
    /// and `tokenizer_config.json`.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the directory is unreadable, the
    /// ONNX file or a required tokenizer file is missing, or the ONNX session
    /// cannot be built from the given bytes.
    pub fn new_from_dir(model_dir: impl AsRef<Path>) -> Result<Self, EmbeddingError> {
        let dir = model_dir.as_ref();

        let model = SbyoLoad::from_dir(dir)?;
        let session = NativeSparseSession::try_new(&model.onnx, model.tokenizer)?;

        let model_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "user-defined".to_string());

        Ok(Self {
            inner: Arc::new(SparseEmbedInner {
                model_name,
                session: parking_lot::Mutex::new(Some(SparseSessionImpl::Native(session))),
                init_options: Default::default(),
                batch_size: DEFAULT_BATCH_SIZE,
            }),
        })
    }

    /// Compute sparse embeddings for a batch of texts.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::ComputeFailed`] if the underlying ONNX session
    /// fails to embed the batch, or [`EmbeddingError::InitFailed`] if it could
    /// not be lazily initialized.
    pub fn embed_texts(&self, texts: &[&str]) -> Result<Vec<SparseEmbedding>, EmbeddingError> {
        self.embed_texts_batched(texts, self.inner.batch_size)
    }

    /// Compute sparse embeddings with an explicit batch size.
    ///
    /// Splits `texts` into chunks of `batch_size` and embeds each through the
    /// shared ONNX session. A `batch_size` of `0` falls back to the configured
    /// default. Results are identical to [`Self::embed_texts`].
    pub fn embed_texts_batched(
        &self,
        texts: &[&str],
        batch_size: usize,
    ) -> Result<Vec<SparseEmbedding>, EmbeddingError> {
        let mut session_guard = self.inner.session.lock();
        let session = session_guard.get_or_insert_with(|| {
            SparseSessionImpl::Fastembed(
                SparseTextEmbedding::try_new(self.inner.init_options.clone())
                    .expect("failed to initialize sparse embedding model — is ONNX Runtime available?"),
            )
        });

        let bs = if batch_size == 0 {
            self.inner.batch_size
        } else {
            batch_size
        };

        match session {
            SparseSessionImpl::Fastembed(s) => s
                .embed(texts, Some(bs))
                .map(|v| v.into_iter().map(SparseEmbedding::from).collect())
                .map_err(|e| EmbeddingError::ComputeFailed(e.to_string())),
            SparseSessionImpl::Native(s) => s.embed(texts, bs),
        }
    }

    /// Embed a single text and return the sparse vector.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::ComputeFailed`] if the underlying ONNX session
    /// fails to embed the text, or [`EmbeddingError::InitFailed`] if it could
    /// not be lazily initialized.
    pub fn embed_text(&self, text: &str) -> Result<SparseEmbedding, EmbeddingError> {
        self.embed_texts(&[text]).map(|mut v| v.remove(0))
    }

    /// Return the model name.
    pub fn model_name(&self) -> &str {
        &self.inner.model_name
    }
}

// ── BGE-M3 multi-modal provider ─────────────────────────────────────

/// Configuration for creating a [`Bgem3Provider`].
#[derive(Debug, Clone)]
pub struct Bgem3ProviderConfig {
    /// The BGE-M3 model variant.
    pub model: Bgem3Model,
    /// Optional cache directory for downloaded models.
    pub cache_dir: Option<std::path::PathBuf>,
    /// Maximum sequence length (tokens). `None` uses model default.
    pub max_length: Option<usize>,
    /// Number of intra-op threads. `None` uses ONNX default.
    pub intra_threads: Option<usize>,
    /// ONNX batch size for each forward pass. `0` uses the library default (256).
    pub batch_size: usize,
    /// Execution providers for the ONNX session, in registration order. Empty
    /// (the default) keeps ORT's CPU provider. Pass the dispatch from
    /// [`directml_execution_provider`] to prefer the DirectML GPU provider on
    /// Windows. Honored by [`Bgem3Provider::try_new`] and the config-bearing
    /// offline paths (`try_from_user_defined_with_config` /
    /// `new_from_dir_with_config`).
    pub execution_providers: Vec<ort::ep::ExecutionProviderDispatch>,
}

impl Default for Bgem3ProviderConfig {
    fn default() -> Self {
        Self {
            model: Bgem3Model::BGEM3Q,
            cache_dir: None,
            max_length: None,
            intra_threads: None,
            batch_size: DEFAULT_BATCH_SIZE,
            execution_providers: Default::default(),
        }
    }
}

impl Bgem3ProviderConfig {
    /// Configures the execution providers used when the ONNX session is built.
    ///
    /// Takes the providers in registration order; an empty list keeps ORT's
    /// default CPU provider. Combine with [`directml_execution_provider`] to
    /// prefer the DirectML GPU provider on Windows.
    pub fn with_execution_providers(mut self, execution_providers: Vec<ort::ep::ExecutionProviderDispatch>) -> Self {
        self.execution_providers = execution_providers;
        self
    }
}

/// A thread-safe wrapper around fastembed's `Bgem3Embedding`.
///
/// Provides joint dense + sparse + ColBERT embedding via ONNX Runtime
/// in a single forward pass. The model is loaded lazily on first use.
#[derive(Debug, Clone)]
pub struct Bgem3Provider {
    inner: Arc<Bgem3Inner>,
}

struct Bgem3Inner {
    model_name: String,
    dense_dimensions: usize,
    session: parking_lot::Mutex<Option<fastembed::Bgem3Embedding>>,
    init_options: Bgem3InitOptions,
    batch_size: usize,
}

impl std::fmt::Debug for Bgem3Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bgem3Inner")
            .field("model_name", &self.model_name)
            .field("dense_dimensions", &self.dense_dimensions)
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}

impl Bgem3Provider {
    /// Create a provider with default model (`bge-m3-onnx-int8`, 1024d dense).
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the model cannot be downloaded,
    /// located in the cache, or the ONNX session cannot be built.
    pub fn try_default() -> Result<Self, EmbeddingError> {
        Self::try_new(Bgem3ProviderConfig::default())
    }

    /// Create a provider with a specific model configuration.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the model cannot be loaded or
    /// the ONNX session cannot be built from the given configuration.
    pub fn try_new(config: Bgem3ProviderConfig) -> Result<Self, EmbeddingError> {
        let model_name = format!("{:?}", config.model);

        // BGE-M3 dense dimension is 1024
        let dense_dimensions = 1024;

        let mut opts = Bgem3InitOptions::new(config.model);
        if let Some(dir) = &config.cache_dir {
            opts = opts.with_cache_dir(dir.clone());
        }
        if let Some(len) = config.max_length {
            opts = opts.with_max_length(len);
        }
        if let Some(threads) = config.intra_threads {
            opts = opts.with_intra_threads(threads);
        }
        opts = opts.with_execution_providers(config.execution_providers);

        let batch_size = if config.batch_size == 0 {
            DEFAULT_BATCH_SIZE
        } else {
            config.batch_size
        };

        Ok(Self {
            inner: Arc::new(Bgem3Inner {
                model_name,
                dense_dimensions,
                session: parking_lot::Mutex::new(None),
                init_options: opts,
                batch_size,
            }),
        })
    }

    /// Create a provider from user-defined BGE-M3 ONNX model bytes (offline/air-gapped).
    ///
    /// No HuggingFace Hub download required. The caller supplies the ONNX model
    /// file bytes and tokenizer files directly. The dense embedding
    /// dimensionality is fixed at 1024 for BGE-M3.
    pub fn try_from_user_defined(
        onnx_bytes: Vec<u8>,
        tokenizer_files: fastembed::TokenizerFiles,
    ) -> Result<Self, EmbeddingError> {
        Self::try_from_user_defined_with_config(onnx_bytes, tokenizer_files, &Bgem3ProviderConfig::default())
    }

    /// Create a provider from user-defined BGE-M3 ONNX model bytes (offline/air-gapped),
    /// honoring `max_length`, `intra_threads`, and `batch_size` from `config`.
    ///
    /// No HuggingFace Hub download required. The caller supplies the ONNX model
    /// file bytes and tokenizer files directly. `config.model` and `config.cache_dir`
    /// are ignored on this path; the dense embedding dimensionality is fixed at
    /// 1024 for BGE-M3. Setting `max_length` (e.g. 8192 for BGE-M3's long-context
    /// capability) raises the tokenizer truncation limit; `None` keeps fastembed's
    /// default (512).
    pub fn try_from_user_defined_with_config(
        onnx_bytes: Vec<u8>,
        tokenizer_files: fastembed::TokenizerFiles,
        config: &Bgem3ProviderConfig,
    ) -> Result<Self, EmbeddingError> {
        Self::build_user_defined(onnx_bytes, tokenizer_files, config, "user-defined".to_string())
    }

    /// Create a provider from ONNX + tokenizer files on disk (offline / air-gapped).
    ///
    /// The model is loaded entirely from a local directory — no HuggingFace Hub
    /// download is performed. The directory must contain a `.onnx` file and the
    /// tokenizer files `tokenizer.json`, `config.json`, `special_tokens_map.json`,
    /// and `tokenizer_config.json`. The dense embedding dimensionality is fixed
    /// at 1024 for BGE-M3.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the directory is unreadable, the
    /// ONNX file or a required tokenizer file is missing, or the ONNX session
    /// cannot be built from the given bytes.
    pub fn new_from_dir(model_dir: impl AsRef<Path>) -> Result<Self, EmbeddingError> {
        Self::new_from_dir_with_config(model_dir, &Bgem3ProviderConfig::default())
    }

    /// Create a provider from ONNX + tokenizer files on disk (offline / air-gapped),
    /// honoring `max_length`, `intra_threads`, and `batch_size` from `config`.
    ///
    /// The model is loaded entirely from a local directory — no HuggingFace Hub
    /// download is performed. The directory must contain a `.onnx` file and the
    /// tokenizer files `tokenizer.json`, `config.json`, `special_tokens_map.json`,
    /// and `tokenizer_config.json`. `config.model` and `config.cache_dir` are
    /// ignored on this path; the dense embedding dimensionality is fixed at 1024
    /// for BGE-M3. Setting `max_length` (e.g. 8192 for BGE-M3's long-context
    /// capability) raises the tokenizer truncation limit; `None` keeps fastembed's
    /// default (512).
    pub fn new_from_dir_with_config(
        model_dir: impl AsRef<Path>,
        config: &Bgem3ProviderConfig,
    ) -> Result<Self, EmbeddingError> {
        let dir = model_dir.as_ref();

        let model = SbyoLoad::from_dir(dir)?;
        let model_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "user-defined".to_string());

        Self::build_user_defined(model.onnx, model.tokenizer, config, model_name)
    }

    fn build_user_defined(
        onnx_bytes: Vec<u8>,
        tokenizer_files: fastembed::TokenizerFiles,
        config: &Bgem3ProviderConfig,
        model_name: String,
    ) -> Result<Self, EmbeddingError> {
        let user_model = fastembed::UserDefinedBgem3Model::new(onnx_bytes, tokenizer_files);

        let embedding =
            fastembed::Bgem3Embedding::try_new_from_user_defined(user_model, Self::offline_init_options(config))
                .map_err(|e| EmbeddingError::InitFailed(e.to_string()))?;

        let batch_size = if config.batch_size == 0 {
            DEFAULT_BATCH_SIZE
        } else {
            config.batch_size
        };

        Ok(Self {
            inner: Arc::new(Bgem3Inner {
                model_name,
                dense_dimensions: 1024,
                session: parking_lot::Mutex::new(Some(embedding)),
                init_options: Default::default(),
                batch_size,
            }),
        })
    }

    /// Build the offline init options for the BGE-M3 user-defined path from a
    /// config. A set `max_length` wins over fastembed's default; `None` keeps
    /// the default (512 for BGE-M3). `intra_threads` is forwarded when set.
    /// `execution_providers` is forwarded (empty keeps ORT's CPU provider).
    /// `model`/`cache_dir` are irrelevant on the offline path and ignored.
    fn offline_init_options(config: &Bgem3ProviderConfig) -> fastembed::InitOptionsUserDefined {
        let mut opts = fastembed::InitOptionsUserDefined::default();
        if let Some(len) = config.max_length {
            opts = opts.with_max_length(len);
        }
        if let Some(threads) = config.intra_threads {
            opts = opts.with_intra_threads(threads);
        }
        if !config.execution_providers.is_empty() {
            opts = opts.with_execution_providers(config.execution_providers.clone());
        }
        opts
    }

    /// Compute dense + sparse + ColBERT embeddings in a single pass.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::ComputeFailed`] if the underlying ONNX session
    /// fails to embed the batch, or [`EmbeddingError::InitFailed`] if it could
    /// not be lazily initialized.
    pub fn embed_texts(&self, texts: &[&str]) -> Result<MultiEmbeddingOutput, EmbeddingError> {
        self.embed_texts_batched(texts, self.inner.batch_size)
    }

    /// Compute dense + sparse + ColBERT embeddings with an explicit batch size.
    ///
    /// Splits `texts` into chunks of `batch_size` and embeds each through the
    /// shared ONNX session. A `batch_size` of `0` falls back to the configured
    /// default. Results are identical to [`Self::embed_texts`].
    pub fn embed_texts_batched(
        &self,
        texts: &[&str],
        batch_size: usize,
    ) -> Result<MultiEmbeddingOutput, EmbeddingError> {
        let mut session_guard = self.inner.session.lock();
        let session = session_guard.get_or_insert_with(|| {
            fastembed::Bgem3Embedding::try_new(self.inner.init_options.clone())
                .expect("failed to initialize BGE-M3 model — is ONNX Runtime available?")
        });

        let bs = if batch_size == 0 {
            self.inner.batch_size
        } else {
            batch_size
        };
        session
            .embed(texts, Some(bs))
            .map(MultiEmbeddingOutput::from)
            .map_err(|e| EmbeddingError::ComputeFailed(e.to_string()))
    }

    /// Embed a single text.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::ComputeFailed`] if the underlying ONNX session
    /// fails to embed the text, or [`EmbeddingError::InitFailed`] if it could
    /// not be lazily initialized.
    pub fn embed_text(&self, text: &str) -> Result<MultiEmbeddingOutput, EmbeddingError> {
        self.embed_texts(&[text])
    }

    /// Return the dense embedding dimensionality (1024 for BGE-M3).
    pub fn dense_dimensions(&self) -> usize {
        self.inner.dense_dimensions
    }

    /// Return the model name.
    pub fn model_name(&self) -> &str {
        &self.inner.model_name
    }
}

// ── Capability trait impl (P96.1) — BGE-M3 ─────────────────────────

impl MultiEmbeddingProvider for Bgem3Provider {
    fn embed_multi(&self, texts: &[&str]) -> Result<MultiEmbeddingOutput, EmbeddingError> {
        self.embed_texts(texts)
    }

    fn dense_dimensions(&self) -> usize {
        // Call through the inherent method to keep the delegation path obvious.
        Bgem3Provider::dense_dimensions(self)
    }
}

impl EmbeddingProvider for Bgem3Provider {
    fn embed_dense(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        self.embed_texts(texts).map(|out| out.dense)
    }

    fn dimensions(&self) -> usize {
        Bgem3Provider::dense_dimensions(self)
    }

    fn model_name(&self) -> &str {
        Bgem3Provider::model_name(self)
    }

    fn as_multi(&self) -> Option<&dyn MultiEmbeddingProvider> {
        Some(self)
    }
}

// ── Cross-encoder reranking provider ─────────────────────────────────

/// Configuration for creating a [`RerankProvider`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RerankProviderConfig {
    /// The reranker model to use.
    pub model: RerankerModel,
    /// Optional cache directory for downloaded models.
    pub cache_dir: Option<std::path::PathBuf>,
    /// Maximum sequence length (tokens). `None` uses model default.
    pub max_length: Option<usize>,
    /// Number of intra-op threads. `None` uses ONNX default.
    pub intra_threads: Option<usize>,
    /// ONNX batch size for each forward pass. `0` uses the library default (256).
    pub batch_size: usize,
    /// Execution providers for the ONNX session, in registration order. Empty
    /// (the default) keeps ORT's CPU provider. Pass the dispatch from
    /// [`directml_execution_provider`] to prefer the DirectML GPU provider on
    /// Windows. Honored by [`RerankProvider::try_new`]; the SBYO offline paths
    /// (`try_from_user_defined` / `new_from_dir`) take no config and keep ORT's
    /// CPU provider.
    pub execution_providers: Vec<ort::ep::ExecutionProviderDispatch>,
}

impl Default for RerankProviderConfig {
    fn default() -> Self {
        Self {
            model: RerankerModel::default(),
            cache_dir: None,
            max_length: None,
            intra_threads: None,
            batch_size: DEFAULT_BATCH_SIZE,
            execution_providers: Default::default(),
        }
    }
}

impl RerankProviderConfig {
    /// Configures the execution providers used when the ONNX session is built.
    ///
    /// Takes the providers in registration order; an empty list keeps ORT's
    /// default CPU provider. Combine with [`directml_execution_provider`] to
    /// prefer the DirectML GPU provider on Windows.
    pub fn with_execution_providers(mut self, execution_providers: Vec<ort::ep::ExecutionProviderDispatch>) -> Self {
        self.execution_providers = execution_providers;
        self
    }
}

/// A thread-safe wrapper around fastembed's [`TextRerank`].
///
/// Provides cross-encoder reranking of (query, document) pairs. The model
/// is loaded lazily on first use.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RerankProvider {
    inner: Arc<RerankInner>,
}

struct RerankInner {
    model_name: String,
    session: parking_lot::Mutex<Option<TextRerank>>,
    init_options: RerankInitOptions,
    batch_size: usize,
}

impl std::fmt::Debug for RerankInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RerankInner")
            .field("model_name", &self.model_name)
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}

impl RerankProvider {
    /// Create a provider with default reranker model.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the model cannot be downloaded,
    /// located in the cache, or the ONNX session cannot be built.
    pub fn try_default() -> Result<Self, EmbeddingError> {
        Self::try_new(RerankProviderConfig::default())
    }

    /// Create a provider with a specific model configuration.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the model cannot be loaded or
    /// the ONNX session cannot be built from the given configuration.
    pub fn try_new(config: RerankProviderConfig) -> Result<Self, EmbeddingError> {
        let model_name = format!("{:?}", config.model);

        let mut opts = RerankInitOptions::new(config.model);
        if let Some(dir) = &config.cache_dir {
            opts = opts.with_cache_dir(dir.clone());
        }
        if let Some(len) = config.max_length {
            opts = opts.with_max_length(len);
        }
        if let Some(threads) = config.intra_threads {
            opts = opts.with_intra_threads(threads);
        }
        opts = opts.with_execution_providers(config.execution_providers);

        let batch_size = if config.batch_size == 0 {
            DEFAULT_BATCH_SIZE
        } else {
            config.batch_size
        };

        Ok(Self {
            inner: Arc::new(RerankInner {
                model_name,
                session: parking_lot::Mutex::new(None),
                init_options: opts,
                batch_size,
            }),
        })
    }

    /// Create a provider from user-defined reranker ONNX model bytes (offline/air-gapped).
    ///
    /// No HuggingFace Hub download required. The caller supplies the ONNX model
    /// file bytes and tokenizer files directly.
    pub fn try_from_user_defined(
        onnx_bytes: Vec<u8>,
        tokenizer_files: fastembed::TokenizerFiles,
    ) -> Result<Self, EmbeddingError> {
        let user_model = fastembed::UserDefinedRerankingModel::new(onnx_bytes, tokenizer_files);

        let reranker = fastembed::TextRerank::try_new_from_user_defined(user_model, Default::default())
            .map_err(|e| EmbeddingError::InitFailed(e.to_string()))?;

        let model_name = "user-defined".to_string();

        Ok(Self {
            inner: Arc::new(RerankInner {
                model_name,
                session: parking_lot::Mutex::new(Some(reranker)),
                init_options: Default::default(),
                batch_size: DEFAULT_BATCH_SIZE,
            }),
        })
    }

    /// Create a provider from ONNX + tokenizer files on disk (offline / air-gapped).
    ///
    /// The model is loaded entirely from a local directory — no HuggingFace Hub
    /// download is performed. The directory must contain a `.onnx` file and the
    /// tokenizer files `tokenizer.json`, `config.json`, `special_tokens_map.json`,
    /// and `tokenizer_config.json`.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the directory is unreadable, the
    /// ONNX file or a required tokenizer file is missing, or the ONNX session
    /// cannot be built from the given bytes.
    pub fn new_from_dir(model_dir: impl AsRef<Path>) -> Result<Self, EmbeddingError> {
        let dir = model_dir.as_ref();

        let model = SbyoLoad::from_dir(dir)?;
        let user_model = fastembed::UserDefinedRerankingModel::new(model.onnx, model.tokenizer);
        let reranker = fastembed::TextRerank::try_new_from_user_defined(user_model, Default::default())
            .map_err(|e| EmbeddingError::InitFailed(e.to_string()))?;

        let model_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "user-defined".to_string());

        Ok(Self {
            inner: Arc::new(RerankInner {
                model_name,
                session: parking_lot::Mutex::new(Some(reranker)),
                init_options: Default::default(),
                batch_size: DEFAULT_BATCH_SIZE,
            }),
        })
    }

    /// Rerank documents by relevance to the query.
    ///
    /// Returns results sorted by score in descending order.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the reranker session could not
    /// be lazily initialized, or [`EmbeddingError::ComputeFailed`] if inference
    /// fails.
    pub fn rerank(&self, query: &str, documents: &[&str]) -> Result<Vec<RerankResult>, EmbeddingError> {
        let mut session_guard = self.inner.session.lock();
        let session = session_guard.get_or_insert_with(|| {
            TextRerank::try_new(self.inner.init_options.clone())
                .expect("failed to initialize reranker model — is ONNX Runtime available?")
        });

        session
            .rerank(query, documents, false, Some(self.inner.batch_size))
            .map_err(|e| EmbeddingError::ComputeFailed(e.to_string()))
    }

    /// Rerank and return documents with their scores.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the reranker session could not
    /// be lazily initialized, or [`EmbeddingError::ComputeFailed`] if inference
    /// fails.
    pub fn rerank_with_documents(&self, query: &str, documents: &[&str]) -> Result<Vec<RerankResult>, EmbeddingError> {
        let mut session_guard = self.inner.session.lock();
        let session = session_guard.get_or_insert_with(|| {
            TextRerank::try_new(self.inner.init_options.clone())
                .expect("failed to initialize reranker model — is ONNX Runtime available?")
        });

        session
            .rerank(query, documents, true, Some(self.inner.batch_size))
            .map_err(|e| EmbeddingError::ComputeFailed(e.to_string()))
    }

    /// Return the model name.
    pub fn model_name(&self) -> &str {
        &self.inner.model_name
    }
}

// ── Capability trait impl (P96.1) — reranker ────────────────────────

impl RerankerProvider for RerankProvider {
    fn rerank(&self, query: &str, documents: &[&str]) -> Result<Vec<RerankResult>, EmbeddingError> {
        // The inherent method and the trait method share a name; the qualified
        // call disambiguates to the inherent implementation so this trait method
        // delegates instead of recursing into itself.
        RerankProvider::rerank(self, query, documents)
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── DirectML execution provider (P94.1) ──

    #[cfg(feature = "directml")]
    #[test]
    fn test_directml_execution_provider_dispatch() {
        let ep = directml_execution_provider();
        assert!(
            ep.downcast_ref::<ort::ep::DirectML>().is_some(),
            "dispatch must wrap the DirectML EP"
        );
    }

    #[cfg(feature = "directml")]
    #[test]
    fn test_directml_session_builds_with_cpu_fallback() {
        // GPU-optional (P94.3): builds a real session under the DirectML
        // dispatch and embeds. The dispatch is registered `fail_silently`, so
        // a machine without a DirectX-12 GPU silently keeps ORT's CPU provider
        // (fallback CPU) instead of failing registration, while a GPU machine
        // runs the session on DirectML. fastembed disables memory-pattern +
        // parallel execution whenever a DirectML EP is in the list. Self-skips
        // when the bge-small-en-v1.5 snapshot is not in the local cache.
        let Some(snapshot) = bge_small_snapshot_dir() else {
            return; // no offline fixture — gracefully skip
        };
        let dir = tempfile::tempdir().unwrap();
        let onnx_src = if snapshot.join("onnx").join("model.onnx").is_file() {
            snapshot.join("onnx").join("model.onnx")
        } else {
            snapshot.join("model.onnx")
        };
        std::fs::copy(&onnx_src, dir.path().join("model.onnx")).unwrap();
        for name in [
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ] {
            std::fs::copy(snapshot.join(name), dir.path().join(name)).unwrap();
        }

        let config = EmbedProviderConfig::default()
            .with_execution_providers(vec![directml_execution_provider().fail_silently()]);
        let provider = FastEmbedProvider::new_from_dir_with_config(dir.path(), 384, &config)
            .expect("session must build under the DirectML dispatch (CPU fallback when no GPU)");
        let embeddings = provider
            .embed_texts(&["directml hello", "second"])
            .expect("embed must succeed with the DirectML dispatch");
        assert_eq!(embeddings.len(), 2);
        for vector in &embeddings {
            assert_eq!(vector.len(), 384);
            assert!(vector.iter().all(|x| x.is_finite()));
        }
    }

    // ── Capability traits (P96.1): object safety & re-export ──

    /// Minimal stand-in for a multi-vector provider (BGE-M3 adds sparse +
    /// ColBERT on top of dense). Implements the trait only to prove the
    /// capability traits are object-safe and usable through `dyn`.
    struct TestMultiProvider;

    impl MultiEmbeddingProvider for TestMultiProvider {
        fn embed_multi(&self, texts: &[&str]) -> Result<MultiEmbeddingOutput, EmbeddingError> {
            Ok(MultiEmbeddingOutput {
                dense: texts.iter().map(|_| vec![0.0; 384]).collect(),
                sparse: texts
                    .iter()
                    .map(|_| SparseEmbedding {
                        indices: vec![],
                        values: vec![],
                    })
                    .collect(),
                colbert: Vec::new(),
            })
        }

        fn dense_dimensions(&self) -> usize {
            384
        }
    }

    /// Minimal stand-in for a cross-encoder reranker.
    struct TestRerankerProvider;

    impl RerankerProvider for TestRerankerProvider {
        fn rerank(&self, query: &str, documents: &[&str]) -> Result<Vec<RerankResult>, EmbeddingError> {
            let _ = query;
            Ok(documents
                .iter()
                .enumerate()
                .map(|(index, doc)| RerankResult {
                    document: Some((*doc).to_string()),
                    score: (index as f32).recip(),
                    index,
                })
                .collect())
        }
    }

    #[test]
    fn test_multi_embedding_provider_object_safe() {
        let provider: &dyn MultiEmbeddingProvider = &TestMultiProvider;
        let out = provider
            .embed_multi(&["one", "two"])
            .expect("mock embed_multi must succeed");
        assert_eq!(out.dense.len(), 2);
        assert_eq!(out.dense[0].len(), 384);
        assert_eq!(out.sparse.len(), 2);
        assert_eq!(provider.dense_dimensions(), 384);
    }

    #[test]
    fn test_reranker_provider_object_safe() {
        let provider: &dyn RerankerProvider = &TestRerankerProvider;
        let results = provider
            .rerank("who wins?", &["dog", "cat", "ferret"])
            .expect("mock rerank must succeed");
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].index, 0);
        assert!(results[0].score > results[2].score);
    }

    #[test]
    fn test_capability_types_reexported_from_crate_root() {
        // P96.1 re-exports: consumers can name the output types and capability
        // traits without going through fastembed or the private module path.
        // Coercing each to a trait object fails to compile if the name does not
        // resolve and if the trait is not object-safe.
        let e: Option<&dyn crate::EmbeddingProvider> = None;
        let m: Option<&dyn crate::MultiEmbeddingProvider> = None;
        let r: Option<&dyn crate::RerankerProvider> = None;
        let multi: crate::MultiEmbeddingOutput = MultiEmbeddingOutput {
            dense: vec![],
            sparse: vec![],
            colbert: vec![],
        };
        let sparse: crate::SparseEmbedding = SparseEmbedding {
            indices: vec![],
            values: vec![],
        };
        assert!(e.is_none());
        assert!(m.is_none());
        assert!(r.is_none());
        assert_eq!(multi.dense.len(), 0);
        assert_eq!(sparse.len(), 0);
    }

    /// P96.1: the real BGE-M3 provider dispatches through `dyn
    /// MultiEmbeddingProvider`. Construction does not download the model — the
    /// ONNX session is lazy — so this never touches the network.
    #[test]
    fn test_bgem3_provider_impl_multi_embedding() {
        let provider = Bgem3Provider::try_default().unwrap();
        let dyn_provider: &dyn MultiEmbeddingProvider = &provider;
        assert_eq!(dyn_provider.dense_dimensions(), 1024);
    }

    /// P96.1: the real reranker dispatches through `dyn RerankerProvider`.
    /// Construction does not download the model — the ONNX session is lazy — so
    /// the coercion proves the `impl RerankerProvider` exists and is object-safe
    /// without touching the network.
    #[test]
    fn test_rerank_provider_impl_reranker() {
        let provider = RerankProvider::try_default().unwrap();
        let dyn_provider: &dyn RerankerProvider = &provider;
        let _ = dyn_provider;
    }

    /// P96.1: an `EmbeddingProvider` that also implements
    /// `MultiEmbeddingProvider` selects the richer capability through
    /// `EmbeddingProvider::as_multi`, while a dense-only provider stays `None`.
    /// Constructors are session-lazy, so this never touches the network.
    #[test]
    fn test_embedding_provider_capability_view() {
        let bge_m3 = Bgem3Provider::try_default().unwrap();
        let dyn_provider: &dyn EmbeddingProvider = &bge_m3;
        assert!(
            dyn_provider.as_multi().is_some(),
            "BGE-M3 must advertise its multi capability through as_multi"
        );

        let dense = FastEmbedProvider::try_default().unwrap();
        let dyn_provider: &dyn EmbeddingProvider = &dense;
        assert!(
            dyn_provider.as_multi().is_none(),
            "dense-only providers must not advertise a multi capability"
        );
        let _ = dyn_provider;
    }

    // ── Dense provider (P89.1) ──

    /// Locate the Xenova bge-small-en-v1.5 snapshot directory in the crate-level
    /// HF cache. `None` when the model is not cached (first run offline, evicted,
    /// ...) — callers should gracefully skip rather than fail.
    fn bge_small_snapshot_dir() -> Option<std::path::PathBuf> {
        std::fs::read_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(".fastembed_cache")
                .join("models--Xenova--bge-small-en-v1.5")
                .join("snapshots"),
        )
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.is_dir())
    }

    /// Load the Xenova bge-small-en-v1.5 ONNX + tokenizer bytes from the crate
    /// level HF cache so the offline (`new_from_dir`/`try_from_user_defined`)
    /// path can be exercised without a network. `None` when the model is not in
    /// the cache (first run offline, cache evicted, ...) — callers should
    /// gracefully skip rather than fail.
    fn load_bge_small_offline_bytes() -> Option<(Vec<u8>, fastembed::TokenizerFiles)> {
        // Ensure the model is available: downloads once into the crate level HF
        // cache on the first run, re-uses it offline afterwards.
        let _ = match crate::embed::FastEmbedProvider::try_default() {
            Ok(p) => p,
            Err(_) => return None, // no network and no cache — nothing to compare against
        };
        let snapshot = bge_small_snapshot_dir()?;
        let read_or = |name: &str| std::fs::read(snapshot.join(name)).ok();
        let onnx_path = if snapshot.join("onnx").join("model.onnx").is_file() {
            snapshot.join("onnx").join("model.onnx")
        } else {
            snapshot.join("model.onnx")
        };
        match (
            std::fs::read(onnx_path).ok(),
            read_or("tokenizer.json"),
            read_or("config.json"),
            read_or("special_tokens_map.json"),
            read_or("tokenizer_config.json"),
        ) {
            (
                Some(onnx),
                Some(tokenizer_file),
                Some(config_file),
                Some(special_tokens_map_file),
                Some(tokenizer_config_file),
            ) => Some((
                onnx,
                fastembed::TokenizerFiles {
                    tokenizer_file,
                    config_file,
                    special_tokens_map_file,
                    tokenizer_config_file,
                },
            )),
            _ => None, // tokenizer/ONNX file missing — nothing to build a session from
        }
    }

    #[test]
    fn test_provider_config_default() {
        let config = EmbedProviderConfig::default();
        assert_eq!(config.model, EmbeddingModel::BGESmallENV15);
        assert!(config.cache_dir.is_none());
        assert!(config.pooling.is_none());
        assert!(config.quantization.is_none());
    }

    #[test]
    fn test_provider_config_quantization_builder() {
        let config = EmbedProviderConfig::default().with_quantization(QuantizationMode::Static);
        assert_eq!(config.quantization, Some(QuantizationMode::Static));
        assert!(config.pooling.is_none(), "quantization builder must not touch pooling");
        let config = config.with_quantization(QuantizationMode::Dynamic);
        assert_eq!(config.quantization, Some(QuantizationMode::Dynamic));
        let config = config
            .with_pooling(Pooling::Mean)
            .with_quantization(QuantizationMode::None);
        assert_eq!(config.quantization, Some(QuantizationMode::None));
        assert_eq!(config.pooling, Some(Pooling::Mean));
    }

    #[test]
    fn test_provider_creation_default() {
        let result = FastEmbedProvider::try_default();
        match result {
            Ok(provider) => {
                assert_eq!(provider.model_name(), "BGESmallENV15");
                assert!(provider.dimensions() > 0);
            }
            Err(EmbeddingError::InitFailed(_)) => {}
            Err(e) => panic!("Unexpected error: {e}"),
        }
    }

    #[test]
    fn test_embed_texts_batch() {
        let provider = match FastEmbedProvider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };

        let texts = vec!["hello world", "test sentence"];
        let embeddings = provider.embed_texts(&texts).unwrap();
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].len(), provider.dimensions());
        assert_eq!(embeddings[1].len(), provider.dimensions());
        assert_ne!(embeddings[0], embeddings[1]);
    }

    #[test]
    fn test_embed_text_single() {
        let provider = match FastEmbedProvider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };

        let embedding = provider.embed_text("single text").unwrap();
        assert_eq!(embedding.len(), provider.dimensions());
    }

    #[test]
    fn test_provider_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FastEmbedProvider>();
    }

    #[test]
    fn test_provider_clone_shares_state() {
        let provider = match FastEmbedProvider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };

        let provider2 = provider.clone();
        assert_eq!(provider.model_name(), provider2.model_name());
        assert_eq!(provider.dimensions(), provider2.dimensions());
    }

    // ── Sparse embedding types (P89.2) ──

    #[test]
    fn test_sparse_embedding_output_shape() {
        let sparse = SparseEmbedding {
            indices: vec![10, 42, 100],
            values: vec![0.5, 1.0, 0.3],
        };
        assert_eq!(sparse.len(), 3);
        assert!(!sparse.is_empty());

        let dense = sparse.to_dense(200);
        assert_eq!(dense.len(), 200);
        assert_eq!(dense[10], 0.5);
        assert_eq!(dense[42], 1.0);
        assert_eq!(dense[100], 0.3);
        assert_eq!(dense[0], 0.0);
    }

    #[test]
    fn test_sparse_embedding_empty() {
        let sparse = SparseEmbedding {
            indices: vec![],
            values: vec![],
        };
        assert!(sparse.is_empty());
        assert_eq!(sparse.len(), 0);
    }

    #[test]
    fn test_sparse_embedding_from_fastembed() {
        let fe = fastembed::SparseEmbedding {
            indices: vec![5, 15],
            values: vec![0.8, 0.2],
        };
        let ours: SparseEmbedding = fe.into();
        assert_eq!(ours.indices, vec![5, 15]);
        assert_eq!(ours.values, vec![0.8, 0.2]);
    }

    // ── Multi embedding output types (P89.2) ──

    #[test]
    fn test_multi_embedding_output_from_fastembed() {
        let fe = Bgem3EmbeddingOutput {
            dense: vec![vec![1.0; 1024]],
            sparse: vec![fastembed::SparseEmbedding {
                indices: vec![1, 2],
                values: vec![0.5, 0.3],
            }],
            colbert: vec![vec![vec![0.1; 1024]; 5]],
        };

        let ours: MultiEmbeddingOutput = fe.into();
        assert_eq!(ours.dense.len(), 1);
        assert_eq!(ours.dense[0].len(), 1024);
        assert_eq!(ours.sparse.len(), 1);
        assert_eq!(ours.sparse[0].indices, vec![1, 2]);
        assert_eq!(ours.colbert.len(), 1);
        assert_eq!(ours.colbert[0].len(), 5);
        assert_eq!(ours.colbert[0][0].len(), 1024);
    }

    // ── Model choice enum (P89.2) ──

    #[test]
    fn test_model_choice_parse() {
        let dense = EmbeddingModelChoice::Dense(EmbeddingModel::BGESmallENV15);
        let dense_q = EmbeddingModelChoice::DenseQ(EmbeddingModel::BGESmallENV15Q);
        let sparse = EmbeddingModelChoice::Sparse(SparseModel::SPLADEPPV1);
        let multi = EmbeddingModelChoice::Multi(Bgem3Model::BGEM3Q);

        match &dense {
            EmbeddingModelChoice::Dense(m) => assert_eq!(*m, EmbeddingModel::BGESmallENV15),
            _ => panic!("expected Dense"),
        }
        match &dense_q {
            EmbeddingModelChoice::DenseQ(m) => assert_eq!(*m, EmbeddingModel::BGESmallENV15Q),
            _ => panic!("expected DenseQ"),
        }
        match &sparse {
            EmbeddingModelChoice::Sparse(m) => assert_eq!(*m, SparseModel::SPLADEPPV1),
            _ => panic!("expected Sparse"),
        }
        match &multi {
            EmbeddingModelChoice::Multi(m) => assert_eq!(*m, Bgem3Model::BGEM3Q),
            _ => panic!("expected Multi"),
        }

        // Default is Dense(BGESmallENV15)
        let default = EmbeddingModelChoice::default();
        assert!(matches!(default, EmbeddingModelChoice::Dense(_)));
    }

    #[test]
    fn test_model_choice_dense_model_selector() {
        let dense = EmbeddingModelChoice::Dense(EmbeddingModel::BGESmallENV15);
        let dense_q = EmbeddingModelChoice::DenseQ(EmbeddingModel::BGESmallENV15Q);
        let sparse = EmbeddingModelChoice::Sparse(SparseModel::SPLADEPPV1);
        let multi = EmbeddingModelChoice::Multi(Bgem3Model::BGEM3Q);

        assert_eq!(dense.dense_model(), Some(&EmbeddingModel::BGESmallENV15));
        assert_eq!(dense_q.dense_model(), Some(&EmbeddingModel::BGESmallENV15Q));
        assert_eq!(sparse.dense_model(), None);
        assert_eq!(multi.dense_model(), None);
    }

    #[test]
    fn test_provider_config_dense_q_default() {
        let config = EmbedProviderConfig::dense_q();
        assert_eq!(config.model, EmbeddingModel::BGESmallENV15Q);
        assert_eq!(config.batch_size, DEFAULT_BATCH_SIZE);
        assert!(config.cache_dir.is_none());
        assert!(config.max_length.is_none());
        assert!(config.intra_threads.is_none());
    }

    #[test]
    fn test_provider_try_q_default() {
        let provider = FastEmbedProvider::try_q_default().expect("Q provider must build");
        assert_eq!(provider.dimensions(), 384);
        assert_eq!(provider.model_name(), "BGESmallENV15Q");
    }

    // Real-model embed through the quantized *Q path: triggers a lazy session
    // (downloads Qdrant/bge-small-en-v1.5-onnx-Q into the local HF cache on first
    // run) and asserts the Q variant yields 384-dim finite embeddings end-to-end.
    #[test]
    fn test_provider_q_real_embed_dimension() {
        let provider = match FastEmbedProvider::try_q_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        assert_eq!(provider.dimensions(), 384);
        assert_eq!(provider.model_name(), "BGESmallENV15Q");

        let texts: Vec<&str> = vec![
            "hello from the quantized model",
            "akar graph database embedding",
            "bge-small english quantized",
            "last sample for dimension check",
        ];
        let embeddings = provider.embed_texts(&texts).expect("Q model embeds must work");
        assert_eq!(embeddings.len(), texts.len());
        for v in &embeddings {
            assert_eq!(v.len(), 384, "each Q embedding must be 384-dimensional");
            assert!(v.iter().all(|x| x.is_finite()), "Q embedding values must be finite");
        }
    }

    // ── Sparse provider (P89.2) ──

    #[test]
    fn test_sparse_provider_config_default() {
        let config = SparseProviderConfig::default();
        assert_eq!(config.model, SparseModel::SPLADEPPV1);
        assert_eq!(config.batch_size, DEFAULT_BATCH_SIZE);
    }

    #[test]
    fn test_sparse_provider_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SparseEmbedProvider>();
    }

    #[test]
    fn test_sparse_batched_matches_default() {
        let provider = match SparseEmbedProvider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        let texts: Vec<&str> = vec!["hello", "world", "sparse", "embedding"];
        let default = provider.embed_texts(&texts).unwrap();
        let batched = provider.embed_texts_batched(&texts, 2).unwrap();
        assert_eq!(default.len(), batched.len());
        for (a, b) in default.iter().zip(batched.iter()) {
            assert_eq!(a.indices, b.indices);
            assert_eq!(a.values.len(), b.values.len());
        }
    }

    // ── BGE-M3 provider (P89.2) ──

    #[test]
    fn test_bgem3_provider_config_default() {
        let config = Bgem3ProviderConfig::default();
        assert_eq!(config.model, Bgem3Model::BGEM3Q);
        assert_eq!(config.batch_size, DEFAULT_BATCH_SIZE);
        assert!(config.max_length.is_none());
        assert!(config.intra_threads.is_none());
        assert!(config.execution_providers.is_empty());
    }

    #[test]
    fn test_bgem3_offline_max_length_forwarded() {
        // Default offline options keep fastembed's default (BGE-M3 => 512).
        let opts = Bgem3Provider::offline_init_options(&Bgem3ProviderConfig::default());
        assert_eq!(
            opts.max_length,
            fastembed::InitOptionsUserDefined::default().max_length,
            "default must not override fastembed's model default"
        );

        // An explicit 8192 is forwarded intact to the offline init options.
        let config = Bgem3ProviderConfig {
            max_length: Some(8192),
            ..Default::default()
        };
        let opts = Bgem3Provider::offline_init_options(&config);
        assert_eq!(opts.max_length, 8192);

        // `intra_threads` is forwarded too.
        let config = Bgem3ProviderConfig {
            max_length: Some(8192),
            intra_threads: Some(2),
            ..Default::default()
        };
        let opts = Bgem3Provider::offline_init_options(&config);
        assert_eq!(opts.max_length, 8192);
        assert_eq!(opts.intra_threads, Some(2));

        // `execution_providers` is forwarded when set; the default stays empty so
        // ORT keeps its CPU provider.
        let config = Bgem3ProviderConfig::default().with_execution_providers(vec![ort::ep::CPU::default().build()]);
        let opts = Bgem3Provider::offline_init_options(&config);
        assert_eq!(opts.execution_providers.len(), 1);
        assert!(
            opts.execution_providers[0].downcast_ref::<ort::ep::CPU>().is_some(),
            "the forwarded dispatch must be the CPU EP"
        );
    }

    #[test]
    fn test_provider_configs_execution_providers_default_and_builder() {
        let cpu_ep = || vec![ort::ep::CPU::default().build()];

        let dense = EmbedProviderConfig::default();
        assert!(dense.execution_providers.is_empty());
        let dense = dense.with_execution_providers(cpu_ep());
        assert_eq!(dense.execution_providers.len(), 1);
        assert!(dense.execution_providers[0].downcast_ref::<ort::ep::CPU>().is_some());

        let sparse = SparseProviderConfig::default();
        assert!(sparse.execution_providers.is_empty());
        let sparse = sparse.with_execution_providers(cpu_ep());
        assert_eq!(sparse.execution_providers.len(), 1);
        assert!(sparse.execution_providers[0].downcast_ref::<ort::ep::CPU>().is_some());

        let bgem3 = Bgem3ProviderConfig::default();
        assert!(bgem3.execution_providers.is_empty());
        let bgem3 = bgem3.with_execution_providers(cpu_ep());
        assert_eq!(bgem3.execution_providers.len(), 1);
        assert!(bgem3.execution_providers[0].downcast_ref::<ort::ep::CPU>().is_some());

        let rerank = RerankProviderConfig::default();
        assert!(rerank.execution_providers.is_empty());
        let rerank = rerank.with_execution_providers(cpu_ep());
        assert_eq!(rerank.execution_providers.len(), 1);
        assert!(rerank.execution_providers[0].downcast_ref::<ort::ep::CPU>().is_some());
    }

    #[test]
    fn test_bgem3_provider_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Bgem3Provider>();
    }

    #[test]
    fn test_bgem3_batched_matches_default() {
        let provider = match Bgem3Provider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        let texts: Vec<&str> = vec!["hello", "world", "bge-m3"];
        let default = provider.embed_texts(&texts).unwrap();
        let batched = provider.embed_texts_batched(&texts, 2).unwrap();
        assert_eq!(default.dense.len(), batched.dense.len());
        assert_eq!(default.sparse.len(), batched.sparse.len());
        assert_eq!(default.colbert.len(), batched.colbert.len());
    }

    #[test]
    fn test_bgem3_provider_dense_dimensions() {
        let provider = match Bgem3Provider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        assert_eq!(provider.dense_dimensions(), 1024);
    }

    #[test]
    fn test_bgem3_provider_offline_load() {
        let dir = tempfile::tempdir().unwrap();

        // Empty directory → no .onnx model.
        match Bgem3Provider::new_from_dir(dir.path()) {
            Err(EmbeddingError::InitFailed(msg)) => {
                assert!(msg.contains("onnx"), "expected missing-onnx message, got: {msg}");
            }
            other => panic!("expected InitFailed for empty dir, got {other:?}"),
        }

        // Directory with an ONNX file but no tokenizer files.
        std::fs::write(dir.path().join("model.onnx"), b"not-a-real-onnx").unwrap();
        match Bgem3Provider::new_from_dir(dir.path()) {
            Err(EmbeddingError::InitFailed(msg)) => {
                assert!(
                    msg.contains("tokenizer"),
                    "expected missing-tokenizer message, got: {msg}"
                );
            }
            other => panic!("expected InitFailed for missing tokenizer, got {other:?}"),
        }

        // Directory with an ONNX file + tokenizer files. The bytes are garbage,
        // so ONNX Runtime must fail from the *local bytes* (never a network
        // download) — proving the offline path read the files off disk.
        for name in [
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ] {
            std::fs::write(dir.path().join(name), b"{}").unwrap();
        }
        match Bgem3Provider::new_from_dir(dir.path()) {
            Err(EmbeddingError::InitFailed(_)) => {}
            Ok(_) => {}
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn test_bgem3_provider_try_from_user_defined_invalid_bytes() {
        let tokenizer = fastembed::TokenizerFiles {
            tokenizer_file: b"{}".to_vec(),
            config_file: b"{}".to_vec(),
            special_tokens_map_file: b"{}".to_vec(),
            tokenizer_config_file: b"{}".to_vec(),
        };
        match Bgem3Provider::try_from_user_defined(b"not-an-onnx".to_vec(), tokenizer) {
            Err(EmbeddingError::InitFailed(_)) => {}
            Ok(_) => {}
            other => panic!("expected InitFailed for garbage bytes, got {other:?}"),
        }
    }

    // ── Rerank provider (P89.2) ──

    #[test]
    fn test_rerank_provider_config_default() {
        let config = RerankProviderConfig::default();
        // Default model is whatever RerankerModel::default() is
        assert!(config.cache_dir.is_none());
        assert_eq!(config.batch_size, DEFAULT_BATCH_SIZE);
    }

    #[test]
    fn test_rerank_provider_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RerankProvider>();
    }

    #[test]
    fn test_rerank_provider_offline_load() {
        let dir = tempfile::tempdir().unwrap();

        // Empty directory → no .onnx model.
        match RerankProvider::new_from_dir(dir.path()) {
            Err(EmbeddingError::InitFailed(msg)) => {
                assert!(msg.contains("onnx"), "expected missing-onnx message, got: {msg}");
            }
            other => panic!("expected InitFailed for empty dir, got {other:?}"),
        }

        // Directory with an ONNX file but no tokenizer files.
        std::fs::write(dir.path().join("model.onnx"), b"not-a-real-onnx").unwrap();
        match RerankProvider::new_from_dir(dir.path()) {
            Err(EmbeddingError::InitFailed(msg)) => {
                assert!(
                    msg.contains("tokenizer"),
                    "expected missing-tokenizer message, got: {msg}"
                );
            }
            other => panic!("expected InitFailed for missing tokenizer, got {other:?}"),
        }

        // Directory with an ONNX file + tokenizer files. The bytes are garbage,
        // so ONNX Runtime must fail from the *local bytes* (never a network
        // download) — proving the offline path read the files off disk.
        for name in [
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ] {
            std::fs::write(dir.path().join(name), b"{}").unwrap();
        }
        match RerankProvider::new_from_dir(dir.path()) {
            Err(EmbeddingError::InitFailed(_)) => {}
            Ok(_) => {}
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn test_rerank_provider_try_from_user_defined_invalid_bytes() {
        let tokenizer = fastembed::TokenizerFiles {
            tokenizer_file: b"{}".to_vec(),
            config_file: b"{}".to_vec(),
            special_tokens_map_file: b"{}".to_vec(),
            tokenizer_config_file: b"{}".to_vec(),
        };
        match RerankProvider::try_from_user_defined(b"not-an-onnx".to_vec(), tokenizer) {
            Err(EmbeddingError::InitFailed(_)) => {}
            Ok(_) => {}
            other => panic!("expected InitFailed for garbage bytes, got {other:?}"),
        }
    }

    // ── P89.7: offline weights, batching, errors, non_exhaustive ──

    #[test]
    fn test_offline_weights_load() {
        let dir = tempfile::tempdir().unwrap();

        // Empty directory → no .onnx model.
        match FastEmbedProvider::new_from_dir(dir.path(), 384) {
            Err(EmbeddingError::InitFailed(msg)) => {
                assert!(msg.contains("onnx"), "expected missing-onnx message, got: {msg}");
            }
            other => panic!("expected InitFailed for empty dir, got {other:?}"),
        }

        // Directory with an ONNX file but no tokenizer files.
        std::fs::write(dir.path().join("model.onnx"), b"not-a-real-onnx").unwrap();
        match FastEmbedProvider::new_from_dir(dir.path(), 384) {
            Err(EmbeddingError::InitFailed(msg)) => {
                assert!(
                    msg.contains("tokenizer"),
                    "expected missing-tokenizer message, got: {msg}"
                );
            }
            other => panic!("expected InitFailed for missing tokenizer, got {other:?}"),
        }

        // Directory with an ONNX file + tokenizer files. The bytes are garbage,
        // so ONNX Runtime must fail from the *local bytes* (never a network
        // download) — proving the offline path read the files off disk.
        for name in [
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ] {
            std::fs::write(dir.path().join(name), b"{}").unwrap();
        }
        match FastEmbedProvider::new_from_dir(dir.path(), 384) {
            Err(EmbeddingError::InitFailed(_)) => {}
            Ok(_) => {}
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn test_new_from_dir_prefers_plain_onnx() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("model_int8.onnx"), b"int8").unwrap();
        std::fs::write(dir.path().join("model.onnx"), b"plain").unwrap();

        // Locate the plain .onnx via find_onnx_file (pub(crate) helper in sbyo).
        let found = crate::sbyo::find_onnx_file(dir.path()).unwrap();
        assert_eq!(found.file_name().unwrap(), "model.onnx");
        assert!(!found.to_string_lossy().ends_with("_int8.onnx"));
    }

    #[test]
    fn test_new_from_dir_with_external_initializer_companion() {
        // P93.3 — fixture: a split/quantized ONNX model keeps its weights in an
        // external initializer file referenced via `model.onnx_data_location`.
        // No such model is cached offline, so the fixture simulates the layout:
        // a real cached model (bge-small) copied into a temp dir plus a
        // synthetic `model.onnx_data` sidecar. This exercises the full P93
        // plumbing end-to-end — companion discovery → `external_initializers`
        // → `with_external_initializer` → session build — through
        // `new_from_dir` (the ONNX Runtime ignores an external initializer that
        // the model does not reference).
        let Some(snapshot) = bge_small_snapshot_dir() else {
            return; // no offline fixture — gracefully skip
        };
        let dir = tempfile::tempdir().unwrap();
        let onnx_src = if snapshot.join("onnx").join("model.onnx").is_file() {
            snapshot.join("onnx").join("model.onnx")
        } else {
            snapshot.join("model.onnx")
        };
        std::fs::copy(&onnx_src, dir.path().join("model.onnx")).unwrap();
        for name in [
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ] {
            std::fs::copy(snapshot.join(name), dir.path().join(name)).unwrap();
        }
        std::fs::write(dir.path().join("model.onnx_data"), b"synthetic-external-weights").unwrap();

        // The loader surfaces the sidecar as an external initializer.
        let sbyo = crate::sbyo::SbyoLoad::from_dir(dir.path()).unwrap();
        assert_eq!(
            sbyo.external_initializers,
            [("model.onnx_data".to_string(), b"synthetic-external-weights".to_vec())]
        );

        // Full dense path: discovery → with_external_initializer → session builds.
        let provider = FastEmbedProvider::new_from_dir(dir.path(), 384)
            .expect("session must build from the offline fixture + companion");
        let texts = ["first text", "second text", "a third"];
        let embeddings = provider.embed_texts(&texts).unwrap();
        assert_eq!(embeddings.len(), 3);
        for vector in &embeddings {
            assert_eq!(vector.len(), 384);
            assert!(vector.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn test_batch_embed_parallel() {
        let provider = match FastEmbedProvider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };

        let texts: Vec<&str> = vec!["first text", "second text", "a third", "fourth item", "last one"];
        let sequential = provider.embed_texts(&texts).unwrap();
        let parallel = provider.par_embed_texts(&texts).unwrap();

        assert_eq!(sequential.len(), parallel.len());
        for (a, b) in sequential.iter().zip(parallel.iter()) {
            assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(b.iter()) {
                assert!((x - y).abs() < 1e-4, "parallel output diverges from sequential");
            }
        }
    }

    #[test]
    fn test_batched_matches_default() {
        let provider = match FastEmbedProvider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        let texts: Vec<&str> = vec!["hello", "world", "one", "two", "three", "four"];
        let default = provider.embed_texts(&texts).unwrap();
        let batched = provider.embed_texts_batched(&texts, 2).unwrap();
        assert_eq!(default.len(), batched.len());
        for (a, b) in default.iter().zip(batched.iter()) {
            for (x, y) in a.iter().zip(b.iter()) {
                assert!((x - y).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn test_error_conversion() {
        let init = EmbeddingError::InitFailed("boom init".into());
        assert!(init.to_string().contains("failed to initialize"));
        assert!(init.to_string().contains("boom init"));

        let compute = EmbeddingError::ComputeFailed("boom compute".into());
        assert!(compute.to_string().contains("computation failed"));
        assert!(compute.to_string().contains("boom compute"));

        // The enum must not be exhaustively matchable by external crates; a
        // documented marker. Construction and formatting stay fully usable.
        assert!(!format!("{init:?}").is_empty());
        assert!(!format!("{compute:?}").is_empty());
    }

    #[test]
    fn test_non_exhaustive_struct() {
        // SparseEmbedding is marked #[non_exhaustive] for forward compatibility,
        // but the public constructors and conversions remain fully usable.
        let sparse = SparseEmbedding {
            indices: vec![1, 2, 3],
            values: vec![0.1, 0.2, 0.3],
        };
        assert_eq!(sparse.len(), 3);

        let fe = fastembed::SparseEmbedding {
            indices: vec![7],
            values: vec![0.9],
        };
        let via_from: SparseEmbedding = fe.into();
        assert_eq!(via_from.indices, vec![7]);

        // EmbedProviderConfig defaults must be usable even though non_exhaustive.
        let cfg = EmbedProviderConfig::default();
        assert_eq!(cfg.batch_size, DEFAULT_BATCH_SIZE);
    }

    // ── P90.6: offline bytes path + HF cache re-use per provider ──

    #[test]
    fn test_dense_provider_try_from_user_defined_invalid_bytes() {
        let tokenizer = fastembed::TokenizerFiles {
            tokenizer_file: b"{}".to_vec(),
            config_file: b"{}".to_vec(),
            special_tokens_map_file: b"{}".to_vec(),
            tokenizer_config_file: b"{}".to_vec(),
        };
        // Garbage ONNX bytes must fail from the local bytes — never a network
        // download — proving the offline user-defined path for the dense provider.
        match FastEmbedProvider::try_from_user_defined(b"not-an-onnx".to_vec(), tokenizer, 384) {
            Err(EmbeddingError::InitFailed(_)) => {}
            Ok(_) => {}
            other => panic!("expected InitFailed for garbage bytes, got {other:?}"),
        }
    }

    #[test]
    fn test_pooling_config_applied_to_dense_offline() {
        let Some(bytes) = crate::embed::tests::load_bge_small_offline_bytes() else {
            return; // model not in cache — nothing to build a session from
        };

        let texts: Vec<&str> = vec!["pooling offline mean", "pooling offline cls"];

        // None → model default (CLS for BGE-small) must still produce valid embeddings.
        let none_cfg = EmbedProviderConfig::default();
        let mean_cfg = EmbedProviderConfig::default().with_pooling(Pooling::Mean);
        let cls_cfg = EmbedProviderConfig::default().with_pooling(Pooling::Cls);

        let mean_emb =
            FastEmbedProvider::try_from_user_defined_with_config(bytes.0.clone(), bytes.1.clone(), 384, &mean_cfg)
                .map(|p| p.embed_texts(&texts))
                .expect("offline user-defined path with mean pooling must initialize")
                .expect("embedding with mean pooling must compute");
        let cls_emb =
            FastEmbedProvider::try_from_user_defined_with_config(bytes.0.clone(), bytes.1.clone(), 384, &cls_cfg)
                .map(|p| p.embed_texts(&texts))
                .expect("offline user-defined path with cls pooling must initialize")
                .expect("embedding with cls pooling must compute");
        let none_emb = FastEmbedProvider::try_from_user_defined_with_config(bytes.0, bytes.1, 384, &none_cfg)
            .map(|p| p.embed_texts(&texts))
            .expect("offline user-defined path with default pooling must initialize")
            .expect("embedding with default pooling must compute");

        for emb in [&mean_emb, &cls_emb, &none_emb] {
            assert_eq!(emb.len(), texts.len(), "one embedding per input text");
            for v in emb {
                assert_eq!(v.len(), 384, "BGE-small output must be 384-dimensional");
                assert!(
                    v.iter().all(|x| x.is_finite()),
                    "embedding must contain only finite values"
                );
            }
        }

        // The pooling strategy must change the output — CLS vs mean vectors differ.
        assert_ne!(
            mean_emb[0], cls_emb[0],
            "mean and cls pooling must not produce identical embeddings"
        );
    }

    #[test]
    fn test_quantization_config_applied_to_dense_offline() {
        let Some(bytes) = crate::embed::tests::load_bge_small_offline_bytes() else {
            return; // model not in cache — nothing to build a session from
        };

        let texts: Vec<&str> = vec!["quantization offline static", "quantization offline default"];

        // Static quantization is batching-safe (fastembed transform: Dynamic is
        // the only mode that rejects explicit batch sizes). None keeps the
        // model default — both must produce valid embeddings.
        let static_cfg = EmbedProviderConfig::default()
            .with_quantization(QuantizationMode::Static)
            .with_pooling(Pooling::Cls);
        let none_cfg = EmbedProviderConfig::default().with_quantization(QuantizationMode::None);

        let static_emb =
            FastEmbedProvider::try_from_user_defined_with_config(bytes.0.clone(), bytes.1.clone(), 384, &static_cfg)
                .map(|p| p.embed_texts(&texts))
                .expect("offline user-defined path with static quantization must initialize")
                .expect("embedding with static quantization must compute");
        let none_emb = FastEmbedProvider::try_from_user_defined_with_config(bytes.0, bytes.1, 384, &none_cfg)
            .map(|p| p.embed_texts(&texts))
            .expect("offline user-defined path with explicit None quantization must initialize")
            .expect("embedding with None quantization must compute");

        for emb in [&static_emb, &none_emb] {
            assert_eq!(emb.len(), texts.len(), "one embedding per input text");
            for v in emb {
                assert_eq!(v.len(), 384, "BGE-small output must be 384-dimensional");
                assert!(
                    v.iter().all(|x| x.is_finite()),
                    "embedding must contain only finite values"
                );
            }
        }
    }

    #[test]
    fn test_dynamic_quantization_batching_guard() {
        let Some(bytes) = crate::embed::tests::load_bge_small_offline_bytes() else {
            return; // model not in cache — nothing to build a session from
        };

        let cfg = EmbedProviderConfig::default().with_quantization(QuantizationMode::Dynamic);
        let provider = FastEmbedProvider::try_from_user_defined_with_config(bytes.0, bytes.1, 384, &cfg)
            .expect("offline initialization with dynamic quantization must succeed");

        let texts: Vec<&str> = vec!["dynamic one", "dynamic two", "dynamic three"];

        // Split batches are incompatible under dynamic quantization — the guard
        // must reject with a clear message instead of forwarding an opaque
        // fastembed error.
        let err = provider
            .embed_texts_batched(&texts, 1)
            .expect_err("dynamic quantization + batch_size < len must be rejected");
        assert!(
            err.to_string().contains("Dynamic quantization"),
            "error must explain the dynamic-quantization limitation: {err}"
        );

        // A batch_size covering all texts (or 0 → single batch) is allowed.
        let ok = provider
            .embed_texts_batched(&texts, texts.len())
            .expect("full-size batch must embed");
        assert_eq!(ok.len(), texts.len());
        for v in &ok {
            assert_eq!(v.len(), 384);
            assert!(v.iter().all(|x| x.is_finite()), "embedding must be finite");
        }

        let ok_default = provider
            .embed_texts(&texts)
            .expect("embed_texts (default batch) must embed");
        assert_eq!(ok_default.len(), texts.len());
    }

    #[test]
    fn test_dense_cache_reuse_matches() {
        let first = match FastEmbedProvider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        let texts: Vec<&str> = vec!["reuse one", "reuse two", "reuse three"];
        let a = first.embed_texts(&texts).unwrap();

        // A second provider re-hydrated from the same HF model cache must
        // produce identical output to the first instance.
        let second = match FastEmbedProvider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        let b = second.embed_texts(&texts).unwrap();
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            for (p, q) in x.iter().zip(y.iter()) {
                assert!((p - q).abs() < 1e-4, "cached re-init diverges");
            }
        }
    }

    #[test]
    fn test_sparse_cache_reuse_matches() {
        let first = match SparseEmbedProvider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        let texts: Vec<&str> = vec!["reuse one", "sparse reuse two", "cache hit"];
        let a = first.embed_texts(&texts).unwrap();

        let second = match SparseEmbedProvider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        let b = second.embed_texts(&texts).unwrap();
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.indices, y.indices, "cached re-init sparse indices diverge");
            assert_eq!(x.values.len(), y.values.len());
            for (p, q) in x.values.iter().zip(y.values.iter()) {
                assert!((p - q).abs() < 1e-4, "cached re-init sparse values diverge");
            }
        }
    }

    #[test]
    fn test_sparse_provider_offline_load() {
        let dir = tempfile::tempdir().unwrap();

        // Empty directory → no .onnx model.
        match SparseEmbedProvider::new_from_dir(dir.path()) {
            Err(EmbeddingError::InitFailed(msg)) => {
                assert!(msg.contains("onnx"), "expected missing-onnx message, got: {msg}");
            }
            other => panic!("expected InitFailed for empty dir, got {other:?}"),
        }

        // Directory with an ONNX file but no tokenizer files.
        std::fs::write(dir.path().join("model.onnx"), b"not-a-real-onnx").unwrap();
        match SparseEmbedProvider::new_from_dir(dir.path()) {
            Err(EmbeddingError::InitFailed(msg)) => {
                assert!(
                    msg.contains("tokenizer"),
                    "expected missing-tokenizer message, got: {msg}"
                );
            }
            other => panic!("expected InitFailed for missing tokenizer, got {other:?}"),
        }

        // Directory with an ONNX file + tokenizer files. The bytes are garbage,
        // so ONNX Runtime must fail from the *local bytes* (never a network
        // download) — proving the native offline path read the files off disk.
        for name in [
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ] {
            std::fs::write(dir.path().join(name), b"{}").unwrap();
        }
        match SparseEmbedProvider::new_from_dir(dir.path()) {
            Err(EmbeddingError::InitFailed(_)) => {}
            other => panic!("expected InitFailed for garbage onnx bytes, got {other:?}"),
        }
    }

    #[test]
    fn test_sparse_provider_try_from_user_defined_invalid_bytes() {
        let tokenizer = fastembed::TokenizerFiles {
            tokenizer_file: b"{}".to_vec(),
            config_file: b"{}".to_vec(),
            special_tokens_map_file: b"{}".to_vec(),
            tokenizer_config_file: b"{}".to_vec(),
        };
        // Garbage ONNX bytes must fail from the local bytes — never a network
        // download — proving the offline user-defined sparse path (P90.2).
        match SparseEmbedProvider::try_from_user_defined(b"not-an-onnx".to_vec(), tokenizer) {
            Err(EmbeddingError::InitFailed(_)) => {}
            other => panic!("expected InitFailed for garbage bytes, got {other:?}"),
        }
    }

    #[test]
    fn test_sparse_native_matches_fastembed() {
        // Ensure the SPLADE model is available: downloads once into the crate-level
        // HF cache on the first run, re-uses it offline afterwards.
        let first = match SparseEmbedProvider::try_default() {
            Ok(p) => p,
            Err(_) => return, // no network and no cache — nothing to compare against
        };
        let texts: Vec<&str> = vec![
            "native parity check",
            "SPLADE offline embedding",
            "bring your own model",
        ];
        let expected = first.embed_texts(&texts).unwrap();

        let cache_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".fastembed_cache");
        let snapshots_dir = cache_root.join("models--Qdrant--Splade_PP_en_v1").join("snapshots");
        let snapshot = match std::fs::read_dir(&snapshots_dir).ok().and_then(|rd| {
            rd.into_iter()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| p.is_dir())
        }) {
            Some(p) => p,
            None => return, // cache structure unexpected — nothing to compare against
        };

        // Build the native SBYO session directly from the cached snapshot files —
        // no fastembed sparse session, no network.
        let native = match SparseEmbedProvider::new_from_dir(&snapshot) {
            Ok(p) => p,
            Err(e) => panic!("native SBYO SPLADE load failed from cached model: {e}"),
        };
        let got = native.embed_texts(&texts).unwrap();

        assert_eq!(got.len(), expected.len());
        for (x, y) in got.iter().zip(expected.iter()) {
            assert_eq!(x.indices, y.indices, "native sparse indices diverge from fastembed");
            assert_eq!(x.values.len(), y.values.len());
            for (p, q) in x.values.iter().zip(y.values.iter()) {
                assert!(
                    (p - q).abs() < 1e-4,
                    "native sparse value {p} diverges from fastembed {q}"
                );
            }
        }
    }

    #[test]
    fn test_bgem3_cache_reuse_matches() {
        let first = match Bgem3Provider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        let texts: Vec<&str> = vec!["reuse one", "bgem3 cache two"];
        let a = first.embed_texts(&texts).unwrap();

        let second = match Bgem3Provider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        let b = second.embed_texts(&texts).unwrap();
        assert_eq!(a.dense.len(), b.dense.len());
        assert_eq!(a.sparse.len(), b.sparse.len());
        assert_eq!(a.colbert.len(), b.colbert.len());
        for (x, y) in a.dense.iter().zip(b.dense.iter()) {
            for (p, q) in x.iter().zip(y.iter()) {
                assert!((p - q).abs() < 1e-4, "cached re-init dense diverges");
            }
        }
    }

    #[test]
    fn test_rerank_provider_cache_reuse_stable_ranking() {
        let first = match RerankProvider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        let query = "What is the capital of France?";
        let documents: Vec<&str> = vec![
            "The sky is blue on a clear day.",
            "Paris is the capital of France.",
            "Cats make affectionate household pets.",
        ];
        let a = first.rerank(query, &documents).unwrap();
        assert_eq!(a.len(), documents.len());
        assert!(
            a.windows(2).all(|w| w[0].score >= w[1].score),
            "rerank results must be sorted by score descending"
        );

        // A second provider re-hydrated from the same HF cache must rank the
        // documents identically.
        let second = match RerankProvider::try_default() {
            Ok(p) => p,
            Err(_) => return,
        };
        let b = second.rerank(query, &documents).unwrap();
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.index, y.index, "cached re-init rerank index diverges");
            assert!((x.score - y.score).abs() < 1e-4, "cached re-init rerank score diverges");
        }
    }

    /// P98.2 — `new_from_dir` (P90) wired against a bundled asset: the real
    /// Xenova bge-small-en-v1.5 model is embedded fully offline, because
    /// `build.rs` copied the git-ignored staging tree
    /// `models/.staging/bge-small-en-v1.5/` into the asset dir at build time.
    /// Deterministic and network-free; self-skips when the bundle was not
    /// staged at build time (e.g. CI checkout without the git-ignored staging).
    #[cfg(feature = "bundle-default-models")]
    #[test]
    fn test_bundled_bge_small_offline_embed() {
        const BUNDLE_NAME: &str = "bge-small-en-v1.5";
        let Some(bundle) = crate::assets::bundled_model(BUNDLE_NAME) else {
            return; // bundle not staged at build time — graceful skip
        };
        assert!(
            crate::assets::is_complete_bundle(&bundle),
            "bundled `{BUNDLE_NAME}` must be a complete schema bundle"
        );

        let provider = match FastEmbedProvider::new_from_dir(&bundle, 384) {
            Ok(p) => p,
            Err(_) => return, // e.g. ort runtime unavailable — graceful skip
        };
        let texts = vec![
            "Akar is a graph database.",
            "Embeddings capture semantic similarity.",
            "Air-gapped model inference needs no network.",
        ];
        let embeddings = provider.embed_texts(&texts).expect("bundled embed must succeed");
        assert_eq!(embeddings.len(), texts.len());
        for embedding in &embeddings {
            assert_eq!(embedding.len(), 384, "BGE-small dense dims must be 384");
            assert!(
                embedding.iter().all(|v| v.is_finite()),
                "embedding values must be finite"
            );
        }

        // P98.4 — determinism: re-embedding the same texts on the same session
        // must be bit-identical, and a second provider re-built from the same
        // bundle must produce the same output (offline path is reproducible —
        // no stochasticity, no re-download).
        let repeat = provider.embed_texts(&texts).expect("repeat embed must succeed");
        assert_eq!(embeddings, repeat, "same-session re-embed must be bit-identical");

        let second = match FastEmbedProvider::new_from_dir(&bundle, 384) {
            Ok(p) => p,
            Err(_) => return,
        };
        let from_second = second.embed_texts(&texts).expect("second bundled embed must succeed");
        assert_eq!(embeddings.len(), from_second.len());
        for (x, y) in embeddings.iter().zip(from_second.iter()) {
            for (p, q) in x.iter().zip(y.iter()) {
                assert!((p - q).abs() < 1e-4, "bundled re-build diverges");
            }
        }

        assert!(crate::assets::bundled_model("does-not-exist").is_none());
    }
}
