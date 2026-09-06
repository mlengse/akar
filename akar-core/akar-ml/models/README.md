# akar-ml Bundled Model Assets (air-gapped)

Canonical layout for shipping lightweight ONNX + tokenizer models with the
crate so offline deployments never touch the network (P98). Models are never
downloaded at build or test time, and model blobs are **never committed to
git**.

## Canonical bundle: `models/<name>/`

A bundle is a flat directory carrying exactly the file set that
`SbyoLoad::from_dir` reads:

```
models/<name>/
├── model.onnx                  # the ONNX graph (mandatory)
├── tokenizer.json              # tokenizer definition
├── config.json                 # model configuration
├── special_tokens_map.json     # special-token mapping
├── tokenizer_config.json       # tokenizer configuration
└── manifest.json               # metadata (optional, P98.3)
```

`model.onnx` is the extraction sentinel: a bundle directory is considered
present exactly when `model.onnx` exists (`assets::bundled_model_dir`).

## How a bundle gets on disk

`build.rs` (minimal, deterministic, idempotent, zero `[build-dependencies]`,
no network, no toolchain):

1. Feature **`bundle-default-models`** off → the script exits immediately and
   never touches build output. The default `test [akar-core]` gate is
   byte-identical.
2. Feature on → staged bundles from the git-ignored tree
   `models/.staging/<name>/` (flat, same file set as the canonical layout) are
   copied into `$OUT_DIR/assets/<name>/`. A directory whose `model.onnx`
   already exists is left untouched (idempotent; an interrupted copy is
   completed on the next build).
3. The extracted root is exposed to the library via `cargo:rustc-env` →
   `AKAR_ML_ASSET_DIR`, consumed at compile time by
   `assets::bundled_model_dir(name)`.

**Preparing staging** (on a networked machine, once): download a model snapshot
(e.g. Hugging Face `Xenova/bge-small-en-v1.5`), flatten its files into
`models/.staging/<name>/`, and record `manifest.json` (P98.2 wires real
models; P98.3 adds license + blob sizes).

**Refreshing a bundle:** staging is only re-read when `cargo:rerun-if-changed`
fires (staging files change). To force a refresh remove the extracted
directory (`$OUT_DIR/assets/<name>`) and rebuild.

## Degradation

- `models/.staging/` missing or empty at build time → `cargo:warning`, build
  still succeeds; `assets::bundled_model_dir(name)` then returns `None`.
- Unknown or partial bundle → `None` / `is_complete_bundle == false` — callers
  fail fast with a clear cause instead of a dangling path.

## Registry: `manifest.json`

`models/manifest.json` (repo root of this folder) lists every registered
bundle. Entries are added as real models are bundled (P98.2/P98.3):

```json
{
  "format": 1,
  "models": [
    {
      "name": "bge-small-en-v1.5",
      "source": "https://huggingface.co/Xenova/bge-small-en-v1.5",
      "license": "apache-2.0",
      "files": { "model.onnx": 133763373, "tokenizer.json": 466103 }
    }
  ]
}
```

`manifest.json` is a reference record (metadata, license, blob sizes); the
build-time staging is the source of truth for what gets copied.