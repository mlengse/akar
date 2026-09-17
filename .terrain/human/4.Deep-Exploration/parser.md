# Deep Exploration — akar-parser

The parser turns Cypher text into a typed AST. It is built on pest.rs using a PEG grammar — a deliberate fork from Kuzu's ANTLR4 choice, kept grammar-compatible so existing Kuzu Cypher queries parse unchanged. The crate produces a `Statement` enum of 33 variants (MATCH, CREATE, MERGE, COPY FROM/TO, CREATE FTS VECTOR INDEX, CREATE FTS INDEX, CALL, LOAD EXTENSION, ATTACH, etc.) plus expression ASTs and parameter placeholders.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `Statement` (33 variants) | Typed AST for every supported Cypher statement | `akar-core/akar-parser/src/lib.rs` |
| `grammar.pest` | Pest PEG grammar for Cypher | `akar-core/akar-parser/src/grammar.pest` |
| expression parser | Binary/unary/list/map/function-call expressions | `akar-core/akar-parser/src/` |
| parameter extraction | Detect `$param` placeholders for prepared statements | `akar-core/akar-parser/src/` |
| COPY tokeniser | FPrepared tokenization for `COPY ... FROM` format options (`FORMAT CSV, HEADER, DELIM`) | `akar-core/akar-parser/src/` |

## Design Decisions

- **pest PEG over ANTLR4 (ADR-001).** Chosen because pest is pure Rust — no codegen step, no external Java toolchain, faster compile-time iteration. The cost is grammar authoring inside `grammar.pest` rather than ANTLR's `.g4`, but Kuzu's grammar shape was ported almost verbatim, easing parity (`akar-core/akar-parser/README.md` documents the parity tokens).
- **Grammar compatibility = drop-in.** Keeping the accepted Cypher surface identical to Kuzu means existing agent memory schemas and migration dumps (exported by `akar-neo4j`) parse without rewriting.

## Why It Matters

Every query begins here. Parsing errors surface as user-facing syntax diagnostics; grammar gaps block entire feature subsets. The parser is also the gate for new syntax (e.g. the FTS `MATCH ... USING FTEDER` and vector `vector_similarity` forms added via grammar extensions), making the `Statement` enum the contract every downstream stage must handle.