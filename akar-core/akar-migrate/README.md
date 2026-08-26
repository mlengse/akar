# Akar Migration Tool

Command-line tool to migrate an Akar C++ database to the pure-Rust Akar database.

**Usage:**
```bash
akar-migrate --from <cpp-db-dir> --to <rust-db-dir>
```

**Flags:**
- `--from <path>` — source C++ database directory
- `--to <path>` — destination Rust database directory
- `--skip-extract` — skip the Python extraction step (assumes `schema.json` and Parquet files already present)

**How it works:**
1. Extract schema + data from the C++ DB via Python (`export_cpp.py`)
2. Connect to the destination Rust Akar database
3. Reconstruct DDL (node tables, then rel tables) and `COPY` data from Parquet
4. Clean up temporary files

Idempotent: re-running skips tables already present in the destination.
