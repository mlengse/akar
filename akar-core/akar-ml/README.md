# Akar ML Extension

Local ML inference and training for the Akar database engine.

Self-contained 1-layer LSTM implementation for sequence prediction (pure Rust, no FFI).

**Components:**
- `LstmCell` — single LSTM gate computation (forget/input/candidate/output)
- `LstmModel` — 1-layer LSTM with input→hidden→output projection
- `train` — BPTT training loop with configurable epochs/learning rate
- `save_model` / `load_model` — serde JSON serialization

**Tests:** 5
