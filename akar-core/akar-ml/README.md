# Akar ML Extension

Local ML inference and training for the Akar database engine.

Self-contained 1-layer LSTM implementation for sequence prediction (pure Rust, no FFI).

**Components:**
- `LstmCell` — single LSTM gate computation (forget/input/candidate/output)
- `LstmModel` — 1-layer LSTM with input→hidden→output projection
- `train` — BPTT training loop with configurable epochs/learning rate
- `save_model` / `load_model` — serde JSON serialization

**Tests:** 5

## Embeddings (ONNX, feature `onnx-embedding`)

In-process text embedding over ONNX Runtime (module [`embed`](src/embed.rs)):
`FastEmbedProvider` (dense), `SparseEmbedProvider`, `Bgem3Provider` (dense +
sparse + ColBERT), and `RerankProvider`. Sessions build lazily on first use.

### DirectML GPU on Windows (feature `directml`)

Enable the additive `directml` Cargo feature and pass
`embed::directml_execution_provider()` into a provider config's
`execution_providers` list to prefer the DirectML (DirectX 12) GPU provider.
Known constraints, honored automatically:

- DirectML disables ORT's **memory-pattern optimization** and **parallel
  execution** on the session — fastembed applies `with_memory_pattern(false)`
  + `with_parallel_execution(false)` when it detects the DirectML EP.
- No GPU present: EP registration fails at session build. Use
  `dispatch.fail_silently()` to keep ORT's CPU provider as the fallback; ops
  the DirectML provider cannot place fall back to CPU per-node by default.
- The SBYO sparse offline path builds its own native ort session that takes no
  execution providers and always runs on CPU.
