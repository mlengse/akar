//! Akar Python bindings (PyO3).
//!
//! Scaffold ("rumah") untuk drop-in replacement KuzuDB di proyek Kairos.
//! Meniru API surface `kuzu` Python client agar `import kuzu` → `import akar`
//! cukup mengubah satu baris:
//!
//! - `akar.Database(path)`
//! - `akar.Connection(db)`
//! - `Connection.query(cypher)` / `Connection.execute(cypher, params)`
//! - `QueryResult`: `has_next()`, `get_next()`, `get_column_names()`,
//!   `rows_as_dict(True)`, `get_all()`, `close()`, truthiness `if r:`,
//!   `len(r)`, iterasi.
//!
//! Status: **scaffold compile-ready**. Translation layer dialek Kuzu→Akar
//! (grammar `FLOAT[n]`/`IF NOT EXISTS`/CALL vector index, multi-statement
//! `INSTALL; LOAD`, `ALTER ... DEFAULT`) dan interpolasi parameter sisi-Python
//! (workaround P51.31) menyusul — lihat `docs/audits/audit-python-bindings-kairos.md`.

use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString};
use pyo3::{BoundObject, IntoPyObject};
use std::sync::Arc;

use akar_common::types::Value;

/// `akar.Database` — membungkus `Arc<akar_main::Database>`.
#[pyclass(module = "akar")]
pub struct Database {
    db: Option<Arc<akar_main::Database>>,
}

#[pymethods]
impl Database {
    /// `Database(path: str)` — buka (atau buat) database embedded di `path`.
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        let db = akar_main::Database::new(path, Default::default())
            .map_err(|e| PyValueError::new_err(format!("Cannot open Akar database at {path:?}: {e}")))?;
        Ok(Self { db: Some(Arc::new(db)) })
    }

    /// Tutup database (lepas file lock & resource).
    fn close(&mut self) {
        self.db = None;
    }

    fn __repr__(&self) -> String {
        "<akar.Database>".to_string()
    }
}

/// `akar.Connection` — eksekusi Cypher terhadap sebuah `Database`.
#[pyclass(module = "akar")]
pub struct Connection {
    conn: akar_main::Connection,
}

#[pymethods]
impl Connection {
    /// `Connection(db: Database)`.
    #[new]
    fn new(database: &Database) -> PyResult<Self> {
        let db = database
            .db
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("Database is closed"))?
            .clone();
        Ok(Self {
            conn: akar_main::Connection::new(&db),
        })
    }

    /// `query(cypher: str)` — eksekusi tanpa parameter.
    fn query(&self, cypher: &str) -> PyResult<QueryResult> {
        let result = self
            .conn
            .query(cypher)
            .map_err(PyRuntimeError::new_err)?;
        Ok(QueryResult::from_native(result))
    }

    /// `execute(cypher: str, params: dict = None)` — eksekusi berparameter.
    ///
    /// TODO(pasca-blocker): parameter `LIMIT $n`/ORDER BY/pola-properti tak
    /// tersubstitusi oleh prepared-statement native (P51.31) → ganti ke
    /// interpolasi sisi-Python + translation layer dialek Kuzu.
    #[pyo3(signature = (cypher, params=None))]
    fn execute(&self, cypher: &str, params: Option<&Bound<'_, PyDict>>) -> PyResult<QueryResult> {
        let params = params.filter(|d| !d.is_empty());
        let result = match params {
            None => self.conn.query(cypher),
            Some(dict) => {
                let stmt = self
                    .conn
                    .prepare(cypher)
                    .map_err(PyRuntimeError::new_err)?;
                let mut keys: Vec<String> = Vec::new();
                let mut vals: Vec<Value> = Vec::new();
                for (k, v) in dict.iter() {
                    keys.push(k.extract::<String>()?);
                    vals.push(py_to_value(&v)?);
                }
                let params: Vec<(&str, Value)> = keys.iter().map(|k| k.as_str()).zip(vals).collect();
                self.conn.execute(&stmt, params)
            }
        };
        let result = result.map_err(PyRuntimeError::new_err)?;
        Ok(QueryResult::from_native(result))
    }

    fn close(&mut self) {}

    fn __repr__(&self) -> String {
        "<akar.Connection>".to_string()
    }
}

/// `akar.QueryResult` — hasil query dengan surface kompatibel kuzu.
#[pyclass(module = "akar")]
pub struct QueryResult {
    result: akar_main::QueryResult,
    chunk_idx: usize,
    row_idx: usize,
    as_dict: bool,
    columns: Vec<String>,
}

impl QueryResult {
    fn from_native(result: akar_main::QueryResult) -> Self {
        let columns = column_names(&result);
        Self {
            result,
            chunk_idx: 0,
            row_idx: 0,
            as_dict: false,
            columns,
        }
    }
}

#[pymethods]
impl QueryResult {
    fn get_column_names(&self) -> Vec<String> {
        self.columns.clone()
    }

    /// Aktifkan mode dict — `get_next()`/`get_all()` mengembalikan dict
    /// `{nama_kolom: nilai}`. Mengembalikan `None` (kompatibel kuzu).
    fn rows_as_dict(&mut self, state: bool) {
        self.as_dict = state;
    }

    fn has_next(&self) -> bool {
        if self.chunk_idx >= self.result.chunks.len() {
            return false;
        }
        let chunk = &self.result.chunks[self.chunk_idx];
        self.row_idx < chunk.size
    }

    fn get_next(&mut self, py: Python<'_>) -> PyResult<PyObject> {
        if !self.has_next() {
            return Ok(py.None());
        }
        let chunk = &self.result.chunks[self.chunk_idx];
        let mut values = Vec::with_capacity(chunk.fields.len());
        for col in 0..chunk.fields.len() {
            values.push(value_to_py(py, chunk.get_value(col, self.row_idx))?);
        }
        let row = if self.as_dict {
            let d = PyDict::new(py);
            for (name, val) in self.columns.iter().zip(values) {
                d.set_item(name, val)?;
            }
            d.unbind().into_any()
        } else {
            PyList::new(py, values)?.unbind().into_any()
        };

        self.row_idx += 1;
        if self.row_idx >= chunk.size {
            self.chunk_idx += 1;
            self.row_idx = 0;
        }
        Ok(row)
    }

    /// Ambil seluruh sisa baris sebagai `list` (dict bila `rows_as_dict(True)`).
    fn get_all(&mut self, py: Python<'_>) -> PyResult<PyObject> {
        let mut rows = Vec::new();
        while self.has_next() {
            rows.push(self.get_next(py)?);
        }
        Ok(PyList::new(py, rows)?.unbind().into_any())
    }

    /// Sisa baris sebagai iterator Python (kuzu shim compat).
    fn __iter__(slf: PyRef<'_, Self>) -> PyResult<PyObject> {
        let py = slf.py();
        let mut rows = Vec::new();
        let mut idx = 0usize;
        while idx < slf.result.chunks.len() {
            let chunk = &slf.result.chunks[idx];
            let row_end = if idx == slf.chunk_idx {
                slf.row_idx
            } else {
                0
            };
            for row in row_end..chunk.size {
                let mut values = Vec::with_capacity(chunk.fields.len());
                for col in 0..chunk.fields.len() {
                    values.push(value_to_py(py, chunk.get_value(col, row))?);
                }
                rows.push(if slf.as_dict {
                    let d = PyDict::new(py);
                    for (name, val) in slf.columns.iter().zip(values) {
                        d.set_item(name, val)?;
                    }
                    d.unbind().into_any()
                } else {
                    PyList::new(py, values)?.unbind().into_any()
                });
            }
            idx += 1;
        }
        let list = PyList::new(py, rows)?;
        Ok(list.call_method0("__iter__")?.unbind().into_any())
    }

    fn __len__(&self) -> usize {
        self.result.num_rows
    }

    fn __bool__(&self) -> bool {
        self.result.num_rows > 0
    }

    fn close(&mut self) {}
}

/// Nama kolom: `field_names` chunk pertama, fallback positional (agregat
/// `field_names` kosong — P52.56) → `col{i}`.
fn column_names(result: &akar_main::QueryResult) -> Vec<String> {
    if let Some(chunk) = result.chunks.first() {
        if !chunk.field_names.is_empty() {
            return chunk.field_names.clone();
        }
    }
    (0..result.num_columns).map(|i| format!("col{i}")).collect()
}

/// Konversi `Value` (Akar) → objek Python.
fn value_to_py(py: Python<'_>, v: Option<akar_common::types::Value>) -> PyResult<PyObject> {
    let Some(v) = v else {
        return Ok(py.None());
    };
    let obj = match v {
        akar_common::types::Value::Null => py.None(),
        akar_common::types::Value::Bool(b) => PyBool::new(py, b).unbind().into_any(),
        akar_common::types::Value::Int64(i) => PyInt::new(py, i).unbind().into_any(),
        akar_common::types::Value::Int32(i) => PyInt::new(py, i).unbind().into_any(),
        akar_common::types::Value::Int16(i) => PyInt::new(py, i).unbind().into_any(),
        akar_common::types::Value::Int8(i) => PyInt::new(py, i).unbind().into_any(),
        akar_common::types::Value::UInt64(u) => PyInt::new(py, u).unbind().into_any(),
        akar_common::types::Value::UInt32(u) => PyInt::new(py, u).unbind().into_any(),
        akar_common::types::Value::UInt16(u) => PyInt::new(py, u).unbind().into_any(),
        akar_common::types::Value::UInt8(u) => PyInt::new(py, u).unbind().into_any(),
        akar_common::types::Value::Int128(i) => PyInt::new(py, i).unbind().into_any(),
        akar_common::types::Value::UInt128(u) => PyInt::new(py, u).unbind().into_any(),
        akar_common::types::Value::Double(d) => PyFloat::new(py, d).unbind().into_any(),
        akar_common::types::Value::Float(f) => PyFloat::new(py, f as f64).unbind().into_any(),
        akar_common::types::Value::String(s) => PyString::new(py, &s).unbind().into_any(),
        akar_common::types::Value::Blob(b) => PyBytes::new(py, &b).unbind().into_any(),
        akar_common::types::Value::Date(d) => PyInt::new(py, d.0 as i64).unbind().into_any(),
        akar_common::types::Value::Timestamp(t) => PyInt::new(py, t.0).unbind().into_any(),
        akar_common::types::Value::TimestampTz(t) => PyInt::new(py, t.0).unbind().into_any(),
        akar_common::types::Value::TimestampNs(t) | akar_common::types::Value::TimestampMs(t) | akar_common::types::Value::TimestampSec(t) => {
            PyInt::new(py, t.0).unbind().into_any()
        }
        akar_common::types::Value::Interval(_) => py.None(),
        akar_common::types::Value::InternalID(id) => {
            let tuple = (id.table_id, id.offset);
            tuple.into_pyobject(py)?.unbind().into_any()
        }
        akar_common::types::Value::Json(j) => serde_json_to_py(py, &j)?,
        akar_common::types::Value::DTime(t) => PyInt::new(py, t).unbind().into_any(),
        akar_common::types::Value::Union(_, inner) => value_to_py(py, Some(*inner))?,
        akar_common::types::Value::List(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(value_to_py(py, Some(item))?)?;
            }
            list.unbind().into_any()
        }
        akar_common::types::Value::Map(kvs) => {
            let d = PyDict::new(py);
            for (k, v) in kvs {
                d.set_item(value_to_py(py, Some(k))?, value_to_py(py, Some(v))?)?;
            }
            d.unbind().into_any()
        }
        akar_common::types::Value::Struct(fields) => {
            let d = PyDict::new(py);
            for (name, val) in fields {
                d.set_item(&name, value_to_py(py, Some(val))?)?;
            }
            d.unbind().into_any()
        }
    };
    Ok(obj)
}

/// Konversi `serde_json::Value` → objek Python (untuk `Value::Json`).
fn serde_json_to_py(py: Python<'_>, j: &serde_json::Value) -> PyResult<PyObject> {
    let obj = match j {
        serde_json::Value::Null => py.None(),
        serde_json::Value::Bool(b) => PyBool::new(py, *b).unbind().into_any(),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(|i| PyInt::new(py, i).unbind().into_any())
            .or_else(|| n.as_f64().map(|f| PyFloat::new(py, f).unbind().into_any()))
            .unwrap_or_else(|| py.None()),
        serde_json::Value::String(s) => PyString::new(py, s).unbind().into_any(),
        serde_json::Value::Array(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(serde_json_to_py(py, item)?)?;
            }
            list.unbind().into_any()
        }
        serde_json::Value::Object(map) => {
            let d = PyDict::new(py);
            for (k, v) in map {
                d.set_item(k, serde_json_to_py(py, v)?)?;
            }
            d.unbind().into_any()
        }
    };
    Ok(obj)
}

/// Konversi objek Python → `Value` (Akar).
fn py_to_value(obj: &Bound<'_, PyAny>) -> PyResult<Value> {
    if obj.is_none() {
        return Ok(Value::Null);
    }
    if let Ok(b) = obj.downcast::<PyBytes>() {
        return Ok(Value::Blob(b.as_bytes().to_vec()));
    }
    if let Ok(b) = obj.extract::<bool>() {
        return Ok(Value::Bool(b));
    }
    if let Ok(i) = obj.extract::<i64>() {
        return Ok(Value::Int64(i));
    }
    if let Ok(f) = obj.extract::<f64>() {
        return Ok(Value::Double(f));
    }
    if let Ok(s) = obj.extract::<String>() {
        return Ok(Value::String(s));
    }
    if let Ok(list) = obj.downcast::<PyList>() {
        let mut items = Vec::with_capacity(list.len());
        for item in list.iter() {
            items.push(py_to_value(&item)?);
        }
        return Ok(Value::List(items));
    }
    if let Ok(dict) = obj.downcast::<PyDict>() {
        let mut fields = Vec::with_capacity(dict.len());
        for (k, v) in dict.iter() {
            let key: String = k.extract().map_err(|_| {
                let ty = k.get_type().repr().map(|r| r.to_string()).unwrap_or_default();
                PyTypeError::new_err(format!("Parameter dict key must be str, got {ty}"))
            })?;
            fields.push((key, py_to_value(&v)?));
        }
        return Ok(Value::Struct(fields));
    }
    let ty = obj.get_type().repr().map(|r| r.to_string()).unwrap_or_default();
    Err(PyTypeError::new_err(format!("Unsupported parameter type: {ty}")))
}

/// Modul `akar`.
#[pymodule]
fn akar(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Database>()?;
    m.add_class::<Connection>()?;
    m.add_class::<QueryResult>()?;
    Ok(())
}
