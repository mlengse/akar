# akar-python — Python bindings (PyO3)

Scaffold ("rumah") untuk drop-in replacement KuzuDB di proyek Kairos.

Modul Python `akar` meniru API surface `kuzu` client sehingga `import kuzu`
→ `import akar` cukup mengubah satu baris:

```python
import akar  # ganti: import kuzu

db = akar.Database("/path/to/db")
conn = akar.Connection(db)
r = conn.execute("MATCH (m:Memory {id: $id}) RETURN m.content", {"id": 42})
while r.has_next():
    print(r.get_next())
```

## Status

- **Scaffold compile-ready** — `Database`, `Connection`, `QueryResult`
  (dengan surface `has_next`/`get_next`/`get_column_names`/`rows_as_dict`/
  `get_all`/`close`/`__bool__`/`__len__`/iterasi), konversi `Value`↔Python.
- **Belum** translation layer dialek Kuzu→Akar (grammar `FLOAT[n]`,
  `IF NOT EXISTS`, CALL vector index, multi-statement `INSTALL; LOAD`,
  `ALTER ... DEFAULT`) dan interpolasi parameter sisi-Python.
  Diblokir oleh bug Rust P52.56 & P51.31 — lihat
  [`docs/audits/audit-python-bindings-kairos.md`](../../docs/audits/audit-python-bindings-kairos.md).

## Build & test

Crate ini **bukan member** workspace `akar-core` (standalone `[workspace]`),
sehingga gate `test [akar-core]` dan CI Rust tidak tersentuh.

```bash
# di akar-core/akar-python/
python -m venv .venv
.venv/Scripts/python -m pip install maturin pytest
$env:PYO3_PYTHON = ".venv\Scripts\python.exe"
.venv/Scripts/maturin develop      # build + install ke venv
.venv/Scripts/python -c "import akar; print(akar.__name__)"
```

## Struktur

```
akar-core/akar-python/
├── Cargo.toml      # pyo3 0.24 (abi3-py39), standalone workspace
├── pyproject.toml  # maturin build
└── src/lib.rs      # modul akar: Database, Connection, QueryResult
```
