# Akar CLI

Interactive Cypher shell (REPL) for the Akar database engine.

**Features:**
- Read-Eval-Print-Loop with history
- Database path argument for persistent storage
- In-memory mode when no path given
- Full query pipeline: parse → bind → plan → optimize → execute
- Result display with column headers and row formatting
- Error display with context

**Usage:**
```bash
cargo run --bin akar-cli           # in-memory
cargo run --bin akar-cli -- /path/to/db  # persistent
```
