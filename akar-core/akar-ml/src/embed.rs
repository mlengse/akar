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
    Bgem3EmbeddingOutput, Bgem3InitOptions, Bgem3Model, EmbeddingModel, RerankInitOptions, RerankResult, RerankerModel,
    SparseInitOptions, SparseModel, SparseTextEmbedding, TextEmbedding, TextInitOptions, TextRerank,
    UserDefinedEmbeddingModel,
};

use crate::sbyo::SbyoLoad;
use crate::sparse::NativeSparseSession;

/// Default ONNX batch size used when the caller does not specify one.
const DEFAULT_BATCH_SIZE: usize = 256;

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
}

// ── Sparse embedding output type ────────────────────────────────────

/// A sparse embedding vector — token-level weights at specific vocabulary indices.
///
/// Sparse embeddings are used for lexical search (SPLADE) and multi-vector
/// retrieval (BGE-M3 sparse branch). The `indices` are vocabulary token IDs
/// and `values` are their corresponding importance weights.
#[derive(Debug, Clone, PartialEq)]
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
}

impl Default for EmbedProviderConfig {
    fn default() -> Self {
        Self {
            model: EmbeddingModel::BGESmallENV15,
            cache_dir: None,
            max_length: None,
            intra_threads: None,
            batch_size: DEFAULT_BATCH_SIZE,
        }
    }
}

impl FastEmbedProvider {
    /// Create a provider with default model (`BGE-small-en-v1.5`, 384 dims).
    ///
    /// Downloads the model on first call; subsequent calls use the cached copy.
    pub fn try_default() -> Result<Self, EmbeddingError> {
        Self::try_new(EmbedProviderConfig::default())
    }

    /// Create a provider with a specific model configuration.
    pub fn try_new(config: EmbedProviderConfig) -> Result<Self, EmbeddingError> {
        let model_name = config.model.to_string();
        let dimensions = TextEmbedding::get_model_info(&config.model)
            .map(|info| info.dim)
            .unwrap_or(384);

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
            }),
        })
    }

    /// Create a provider from user-defined ONNX model bytes (offline/air-gapped).
    ///
    /// No HuggingFace Hub download required. The caller supplies the ONNX model
    /// file bytes and tokenizer files directly.
    pub fn try_from_user_defined(
        onnx_bytes: Vec<u8>,
        tokenizer_files: fastembed::TokenizerFiles,
        dimensions: usize,
    ) -> Result<Self, EmbeddingError> {
        let user_model = fastembed::UserDefinedEmbeddingModel::new(onnx_bytes, tokenizer_files);

        let embedding = TextEmbedding::try_new_from_user_defined(user_model, Default::default())
            .map_err(|e| EmbeddingError::InitFailed(e.to_string()))?;

        let model_name = "user-defined".to_string();

        Ok(Self {
            inner: Arc::new(FastEmbedInner {
                model_name,
                dimensions,
                session: parking_lot::Mutex::new(Some(embedding)),
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
        let dir = model_dir.as_ref();

        let model = SbyoLoad::from_dir(dir)?;
        let user_model = UserDefinedEmbeddingModel::new(model.onnx, model.tokenizer);
        let embedding = TextEmbedding::try_new_from_user_defined(user_model, Default::default())
            .map_err(|e| EmbeddingError::InitFailed(e.to_string()))?;

        let model_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "user-defined".to_string());

        Ok(Self {
            inner: Arc::new(FastEmbedInner {
                model_name,
                dimensions,
                session: parking_lot::Mutex::new(Some(embedding)),
                init_options: Default::default(),
                batch_size: DEFAULT_BATCH_SIZE,
            }),
        })
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
    /// session could not be lazily initialized.
    pub fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        self.embed_texts_batched(texts, self.inner.batch_size)
    }

    /// Compute dense embeddings with an explicit batch size.
    ///
    /// Splits `texts` into chunks of `batch_size` and embeds each through the
    /// shared ONNX session. A `batch_size` of `0` falls back to the configured
    /// default. Results are identical to [`Self::embed_texts`].
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
}

impl Default for SparseProviderConfig {
    fn default() -> Self {
        Self {
            model: SparseModel::SPLADEPPV1,
            cache_dir: None,
            max_length: None,
            intra_threads: None,
            batch_size: DEFAULT_BATCH_SIZE,
        }
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
    pub fn try_default() -> Result<Self, EmbeddingError> {
        Self::try_new(SparseProviderConfig::default())
    }

    /// Create a provider with a specific model configuration.
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
}

impl Default for Bgem3ProviderConfig {
    fn default() -> Self {
        Self {
            model: Bgem3Model::BGEM3Q,
            cache_dir: None,
            max_length: None,
            intra_threads: None,
            batch_size: DEFAULT_BATCH_SIZE,
        }
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
    pub fn try_default() -> Result<Self, EmbeddingError> {
        Self::try_new(Bgem3ProviderConfig::default())
    }

    /// Create a provider with a specific model configuration.
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
        let user_model = fastembed::UserDefinedBgem3Model::new(onnx_bytes, tokenizer_files);

        let embedding = fastembed::Bgem3Embedding::try_new_from_user_defined(user_model, Default::default())
            .map_err(|e| EmbeddingError::InitFailed(e.to_string()))?;

        let model_name = "user-defined".to_string();

        Ok(Self {
            inner: Arc::new(Bgem3Inner {
                model_name,
                dense_dimensions: 1024,
                session: parking_lot::Mutex::new(Some(embedding)),
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
    /// and `tokenizer_config.json`. The dense embedding dimensionality is fixed
    /// at 1024 for BGE-M3.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::InitFailed`] if the directory is unreadable, the
    /// ONNX file or a required tokenizer file is missing, or the ONNX session
    /// cannot be built from the given bytes.
    pub fn new_from_dir(model_dir: impl AsRef<Path>) -> Result<Self, EmbeddingError> {
        let dir = model_dir.as_ref();

        let model = SbyoLoad::from_dir(dir)?;
        let user_model = fastembed::UserDefinedBgem3Model::new(model.onnx, model.tokenizer);
        let embedding = fastembed::Bgem3Embedding::try_new_from_user_defined(user_model, Default::default())
            .map_err(|e| EmbeddingError::InitFailed(e.to_string()))?;

        let model_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "user-defined".to_string());

        Ok(Self {
            inner: Arc::new(Bgem3Inner {
                model_name,
                dense_dimensions: 1024,
                session: parking_lot::Mutex::new(Some(embedding)),
                init_options: Default::default(),
                batch_size: DEFAULT_BATCH_SIZE,
            }),
        })
    }

    /// Compute dense + sparse + ColBERT embeddings in a single pass.
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
}

impl Default for RerankProviderConfig {
    fn default() -> Self {
        Self {
            model: RerankerModel::default(),
            cache_dir: None,
            max_length: None,
            intra_threads: None,
            batch_size: DEFAULT_BATCH_SIZE,
        }
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
    pub fn try_default() -> Result<Self, EmbeddingError> {
        Self::try_new(RerankProviderConfig::default())
    }

    /// Create a provider with a specific model configuration.
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

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Dense provider (P89.1) ──

    #[test]
    fn test_provider_config_default() {
        let config = EmbedProviderConfig::default();
        assert_eq!(config.model, EmbeddingModel::BGESmallENV15);
        assert!(config.cache_dir.is_none());
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
}
