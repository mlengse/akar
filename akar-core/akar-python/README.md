# akar-python — Python bindings (PyO3)

Python bindings for [Akar](https://github.com/mlengse/akar) — a pure Rust embedded graph database for AI agent memory.

Provides a **drop-in replacement for KuzuDB** in the [Kairos](https://github.com/mlengse/kairos) project via a Cypher-to-SQL translation layer, or direct Akar API access.

## Quick start

```python
import akar

db = akar.Database("/path/to/db")
conn = akar.Connection(db)

# Execute Cypher queries
conn.query("""
    CREATE NODE TABLE Memory(id INT64, content STRING, embedding FLOAT[],
    PRIMARY KEY(id))
""")

# Parameterized queries
result = conn.execute(
    "MATCH (m:Memory {id: $id}) RETURN m.content",
    {"id": 42},
)

while result.has_next():
    print(result.get_next())

# Get all results as list of dicts
rows = result.get_all()

db.close()
```

## API surface

### `Database(path: str)`

- `close()` — release file lock, drop all connections
- `__repr__()` — display path

### `Connection(database: Database)`

- `query(cypher: str) -> QueryResult` — execute Cypher, no parameters
- `execute(cypher: str, params: dict = None) -> QueryResult` — execute Cypher with `$param` interpolation
- `close()` — drop connection (DB stays open)

### `QueryResult`

- `has_next() -> bool` — check for more rows
- `get_next() -> dict | list | scalar` — fetch next row (returns dict when `rows_as_dict=True`, default)
- `get_all() -> list` — fetch all rows
- `get_column_names() -> list[str]` — column names
- `rows_as_dict(state: bool)` — toggle dict vs tuple mode
- `__iter__()` / `__len__()` / `__bool__()` — iteration and checks
- `close()` — drop result

## Kairos compatibility shim

For drop-in replacement of KuzuDB in Kairos, use the shim at `kairos/kuzu.py`:

```python
# In Kairos codebase:
import kuzu  # transparently becomes `import akar`

db = kuzu.Database("/path/to/db")
conn = kuzu.Connection(db)
```

The shim registers `sys.modules` aliases for `kuzu` and `ladybug` packages.

## Supported Cypher features

- DDL: `CREATE NODE TABLE`, `CREATE REL TABLE`, `DROP TABLE`, `ALTER TABLE`, `CREATE/DROP INDEX`
- DML: `MATCH`, `WHERE`, `RETURN`, `CREATE`, `SET`, `DELETE`, `MERGE` (ON CREATE/ON MATCH)
- Composition: `WITH`, `ORDER BY`, `LIMIT`, `SKIP`, `UNION ALL`, `UNWIND`, `OPTIONAL MATCH`, `FOREACH`
- Data loading: `COPY FROM` (CSV, Parquet), `COPY TO`, `EXPORT DATABASE`, `IMPORT DATABASE`
- Parameters: `$param` interpolation (string, int, float, bool, list, dict, null)
- Translation: Kuzu syntax auto-converted to Akar SQL (`IF NOT EXISTS`, `FLOAT[n]` → `FLOAT[]`, `INSTALL/LOAD EXTENSION` no-op, vector index calls)

## Build from source

This crate is a **standalone workspace** (not a member of `akar-core`'s workspace), so it does not affect the `test [akar-core]` gate or CI.

### Prerequisites

- Rust toolchain (stable)
- Python >= 3.9
- [maturin](https://github.com/PyO3/maturin) >= 1.7

### Development build

```bash
cd akar-core/akar-python

python -m venv .venv
.venv/Scripts/python -m pip install maturin pytest
$env:PYO3_PYTHON = ".venv\Scripts\python.exe"   # Windows PowerShell
# export PYO3_PYTHON=".venv/bin/python"           # Linux/macOS

.venv/Scripts/maturin develop    # build + install into venv
.venv/Scripts/python -c "import akar; print(akar.__name__)"
```

### Release build (wheel)

```bash
.venv/Scripts/maturin build --release
# Output: target/wheels/akar-0.1.0-cp39-abi3-<platform>.whl
```

### Source distribution

```bash
.venv/Scripts/maturin sdist
# Output: target/wheels/akar-0.1.0.tar.gz
```

## Tests

### Rust unit tests (cargo)

```bash
cd akar-core/akar-python
cargo test --lib
```

25 tests covering: translation layer, parameter interpolation, value conversion, schema bootstrap, vector index, lock management, connection lifecycle.

### Python compatibility harness (pytest)

```bash
cd akar-core/akar-python
.venv/Scripts/pytest tests/test_kuzu_compat.py -v
```

53 tests validating drop-in replacement against real Kairos `KuzuDBStore` / `KuzuDBDreamBackend` methods (schema bootstrap, store, search, connections, export/repair, CHECKPOINT).

## Project structure

```
akar-core/akar-python/
├── Cargo.toml          # pyo3 0.29.2 (abi3-py39), standalone workspace
├── pyproject.toml      # maturin build config
├── src/
│   ├── lib.rs          # PyO3 module: Database, Connection, QueryResult
│   ├── translation.rs  # Kuzu Cypher → Akar SQL translation
│   └── param_interp.rs # $param → literal interpolation
└── tests/
    ├── conftest.py     # pytest fixtures + gap-report hooks
    └── test_kuzu_compat.py  # 53 Kairos drop-in tests
```

## License

GPL-3.0-or-later
