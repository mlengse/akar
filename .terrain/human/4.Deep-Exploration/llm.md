# Deep Exploration — akar-llm

`akar-llm` provides `create_embedding` — a function that calls the OpenAI or Ollama embedding APIs and returns dense vectors. It supports seven input variants (file, string, HTTP URL, and their prefixed forms), streaming responses to avoid timeouts on large inputs.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `create_embedding` | Function: send text(s) to OpenAI/Ollama, return embedding vectors | `akar-core/akar-llm/src/lib.rs` |
| Input variants | Enum/File/String/HttpUrl/PrefixedFile/PrefixedString/PrefixedHttpUrl | `akar-core/akar-llm/src/` |
| streaming client | Chunked HTTP streaming for large payloads | `akar-core/akar-llm/src/` |

## Design Decisions

- **Two providers, one interface.** OpenAI (`api.openai.com`) and Ollama (`localhost:11434`) share the same function signature; `llm_provider` parameter selects. Chosen to cover both local (Ollama) and hosted (OpenAI) use cases.
- **Prefixed input for bulk.** `prefixed-file`, `prefixed-string`, `prefixed-http-url` variants enable bulk embedding in a single call (important for batch-memory ingestion).

## Why It Matters

`create_embedding` is the ingestion-side complement to `akar-vector`: before an agent can run `cosine_similarity`, it must embed its memories. This crate provides the embedding source without requiring a separate Python script — embeddings stay inside the same Cypher query as the rest of the memory pipeline.