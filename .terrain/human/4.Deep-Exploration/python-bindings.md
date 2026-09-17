# Deep Exploration — akar-python

Python bindings via PyO3 make Akar's full surface available to Python: the core database API (`akar.connect()`, query/prepare/execute/rollback), the kuzu-compatibility layer (Result.get_as_df/Arrow/pl), and the entire memory layer — spread, knn_fused_score, dream, graph_operation, lstm (the `LstmModel` class), create_embedding, greedy_search, agentic_decomposition, random_splitter, load_tantivy_index.

## Key Components

| Component | Purpose | Source |
|-----------|---------|--------|
| `akar.connect(db_path)` | Open/create database | `akar-core/akar-python/src/lib.rs:18` |
| `AkarDatabase` | Database handle with `rename`, `drop` | `akar-core/akar-python/src/lib.rs:26` |
| `AkarConnection` | query/prepare/execute/rollback/close | `akar-core/akar-python/src/lib.rs:42` |
| `Result` (kuzu compat) | get_as_df / get_as_arrow / get_as_pl | `akar-core/akar-python/src/lib.rs:104` |
| `knn_fused_score` | Hybrid vector+FTS recall with alpha weighting | `akar-core/akar-python/src/functions/knn_fused_score.rs` |
| `spread` | Temporal spread (delta graph query) | `akar-core/akar-python/src/functions/spread.rs` |
| `dream` | Consolidation cycle | `akar-core/akar-python/src/functions/dream.rs` |
| `graph_operation` | page_rank / community / centrality | `akar-core/akar-python/src/functions/graph_operation.rs` |
| `LstmModel` (akar.lstm) | OO LSTM: forward_cell/forward_sequence/forward_sequence_hidden, static `train`, online `train_pair` (in-place single-pair BPTT, P116.2), JSON `save`/`load` + binary `save_bin`/`load_bin` (P117.1), `num_layers` depth; `akar.lstm` registered in `sys.modules` for `import akar.lstm` (P118.2) | `akar-core/akar-python/src/lstm.rs` |
| `create_embedding` | Local ONNX embedding | `akar-core/akar-python/src/functions/embedding.rs` |
| `greedy_search` / `agentic_decomposition` | Agentic tool-use with LLM | `akar-core/akar-python/src/functions/` |
| `random_splitter` | Text chunking | `akar-core/akar-python/src/functions/random_splitter.rs` |

## Design Decisions

- **Memory layer lives in Python bindings, not in core.** Functions like `dream`, `spread`, `graph_operation`, and the `lstm` module (`LstmModel`) are `#[pyfunction]`/`#[pyclass]`-wrapped but call into core crates (dream/algo/ml/graph) through normal Rust calls — the binding layer is where the PyDict/PyList conversion happens.
- **Submodule registration in `sys.modules` (P118.2).** Because `akar` is a PyO3 extension module (no `__path__`), `m.add_submodule` only attaches `lstm` as a parent attribute — it does *not* make `import akar.lstm` work. The `register` function therefore also registers the full dotted name `akar.lstm` in `sys.modules` so both `import akar.lstm` and `from akar.lstm import LstmModel` resolve.
- **Kuzu compatibility layer.** `get_as_df()`, `get_as_arrow()`, `get_as_pl()` mirror Kuzu's API exactly, making existing Kuzu-based agent code drop-in swappable.
- **17 tests** (12 core + 5 async/reranker/fastembed/embedding/spread), runnable as `cargo test` and `pytest`.

## Why It Matters

Python is the dominant agent runtime. This crate is how agents embed their entire memory graph: `akar.connect("./memory.db")`, `conn.query(...)`, `dream(dream_dict)` — the full loop, no separate daemon required. The OpenTelemetry and sulur daemon integration tests live here too, validating multi-process operation end-to-end.