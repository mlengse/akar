# Deep Exploration — akar-ml

`akar-ml` provides the local model layer: embedding creation (dense/sparse/ColBERT reranking via fastembed/ort ONNX), embedding caching, and local LSTM training/inference for temporal pattern learning. This is what lets consolidation and reranking run without a cloud dependency.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| embedding pipeline | Dense (cl-tohoku, BERT, NER, ColBERT reranker), sparse, image, audio embeddings | `akar-core/akar-ml/src/lib.rs` |
| embedding cache | Cache embeddings by text to avoid recomputation | `akar-core/akar-ml/src/` |
| LSTM trainer | `LstmModel<F: Float>` (default f64; `LstmModelF32` for f32), multi-layer via `num_layers`, batch `train` BPTT + online `train_pair` single-pair BPTT, `forward_sequence_hidden` for raw hidden-state output (P118.1), JSON `save`/`load` + binary `save_bin`/`load_bin` persistence (P117.1) | `akar-core/akar-ml/src/lstm.rs` |
| feature generation | Temporal/interaction features for LSTM | `akar-core/akar-ml/src/` |
| Python API | `akar.lstm.LstmModel` (forward_cell/forward_sequence/forward_sequence_hidden, static `train`, online `train_pair`, JSON `save`/`load` + binary `save_bin`/`load_bin`) | `akar-core/akar-python/src/lstm.rs` |

## Design Decisions

- **Local-first execution.** All model inference/training uses local ONNX/fastembed artifacts — no network round-trip for embeddings or reranking. The alternative (cloud-only) was rejected because akar's memory agent must work offline and with privacy guarantees.
- **ColBERT-style reranking.** The embedding family includes a ColBERT reranker — retrieve-wide (hybrid recall in `akar-search`) then refine with a cross-encoder-style model locally.
- **Model serialization to a directory (JSON + binary).** LSTM checkpoints write to `output_dir` in two formats: JSON `save`/`load` and, since P117.1, a compact binary `save_bin`/`load_bin` (magic header `"LSTM"` + version + dimensions, weights in native f32/f64 precision). Binary is bit-exact and more compact than JSON for large models, so trained temporal models survive restarts and can be versioned either way.
- **Depth and precision are configurable.** Since P114/P115, `LstmModel` is generic over the float type (`LstmModelF64` default, `LstmModelF32` for C++-LSTM parity) and stacks `num_layers` hidden layers (layer N's hidden state feeds layer N+1). `train_pair` adds online single-pair BPTT that updates all layers in place, so temporal models can be trained incrementally as new memories arrive instead of only in offline epochs.
- **Hidden-state output mode (P118.1).** `forward_sequence_hidden` returns the final projected output **and** the last layer's raw hidden state at every timestep. This mirrors the sulur C++ LSTM's hidden-state output (Finding #34-LSTM) — consumers that need the hidden state itself (e.g. feature extraction for downstream ML) no longer have to extract it from `LstmCell` objects manually.

## Why It Matters

`akar-ml` is what makes consolidation *intelligent*: embeddings for vector recall, LSTM for temporal pattern detection (e.g. "this memory is repeated every day → consolidate"), and reranking for the final ranking stage. Without it, `akar-search` would have no embedding source and `akar-dream` no temporal prior — it closes the loop on the AI-memory story.