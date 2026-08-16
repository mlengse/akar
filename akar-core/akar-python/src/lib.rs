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
//! Translation layer dialek Kuzu→Akar (grammar `FLOAT[n]`/`IF NOT EXISTS`/
//! CALL vector index, multi-statement `INSTALL; LOAD`, `ALTER ... DEFAULT`)
//! dan interpolasi parameter sisi-Python (P53.x) — lihat `translation.rs`,
//! `param_interp.rs`, dan `docs/audits/audit-python-bindings-kairos.md`.

mod param_interp;
mod translation;

use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString};
use pyo3::{BoundObject, IntoPyObject};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use akar_common::types::Value;

use translation::{Translated, Translator, ERR_NOT_FOUND};

/// Kolom internal Akar yang tak boleh muncul sebagai properti node.
const INTERNAL_COLS: &[&str] = &["_id", "_label", "_src", "_dst", "_rel_id"];

/// `akar.Database` — membungkus `Arc<akar_main::Database>` + state translator.
#[pyclass(module = "akar")]
pub struct Database {
    db: Option<Arc<akar_main::Database>>,
    translator: Arc<Mutex<Translator>>,
    /// Connections created against this database. Kept alive here and
    /// force-closed by `close()` so the underlying file lock is released even
    /// if the Python `Connection` wrappers are still referenced (P53.18).
    connections: Vec<Py<Connection>>,
}

#[pymethods]
impl Database {
    /// `Database(path: str)` — buka (atau buat) database embedded di `path`.
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        let db = akar_main::Database::new(path, Default::default())
            .map_err(|e| PyValueError::new_err(format!("Cannot open Akar database at {path:?}: {e}")))?;
        Ok(Self {
            db: Some(Arc::new(db)),
            translator: Arc::new(Mutex::new(Translator::new())),
            connections: Vec::new(),
        })
    }

    /// Tutup database: lepas file lock & resource. Menutup paksa semua
    /// connection yang masih hidup (melepas `Arc<Database>` mereka) agar
    /// `Database(path sama)` bisa dibuka lagi — pola kairos
    /// close→checkpoint→reopen (P53.18, G7).
    fn close(&mut self, py: Python<'_>) {
        let conns = std::mem::take(&mut self.connections);
        for c in conns {
            if let Ok(mut conn) = c.try_borrow_mut(py) {
                conn.conn = None;
            }
        }
        self.db = None;
    }

    fn __repr__(&self) -> String {
        "<akar.Database>".to_string()
    }
}

/// `akar.Connection` — eksekusi Cypher terhadap sebuah `Database`.
#[pyclass(module = "akar")]
pub struct Connection {
    conn: Option<akar_main::Connection>,
    translator: Arc<Mutex<Translator>>,
}

#[pymethods]
impl Connection {
    /// `Connection(db: Database)`.
    #[new]
    fn new(database: &Bound<'_, Database>) -> PyResult<Py<Self>> {
        let py = database.py();
        let db = database
            .borrow()
            .db
            .clone()
            .ok_or_else(|| PyValueError::new_err("Database is closed"))?;
        let translator = database.borrow().translator.clone();
        let this = Py::new(
            py,
            Self {
                conn: Some(akar_main::Connection::new(&db)),
                translator,
            },
        )?;
        database.borrow_mut().connections.push(this.clone_ref(py));
        Ok(this)
    }

    /// `query(cypher: str)` — eksekusi tanpa parameter.
    fn query(&self, cypher: &str) -> PyResult<QueryResult> {
        self.run(cypher, None)
    }

    /// `execute(cypher: str, params: dict = None)` — eksekusi berparameter.
    ///
    /// Parameter diinterpolasi menjadi literal Cypher (P53.7) karena native
    /// prepared-statement tak dapat mensubstitusi `LIMIT $n`/`ORDER BY`/
    /// pola-properti (P51.31).
    #[pyo3(signature = (cypher, params=None))]
    fn execute(&self, cypher: &str, params: Option<&Bound<'_, PyDict>>) -> PyResult<QueryResult> {
        let mut map: HashMap<String, Value> = HashMap::new();
        if let Some(dict) = params {
            for (k, v) in dict.iter() {
                map.insert(k.extract()?, py_to_value(&v)?);
            }
        }
        self.run(cypher, if map.is_empty() { None } else { Some(map) })
    }

    /// Tutup connection: lepas `Arc<Database>` (file lock). Query berikutnya
    /// pada wrapper yang sama akan error "Connection is closed" (P53.18).
    fn close(&mut self) {
        self.conn = None;
    }

    fn __repr__(&self) -> String {
        "<akar.Connection>".to_string()
    }
}

impl Connection {
    fn conn(&self) -> PyResult<&akar_main::Connection> {
        self.conn
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Connection is closed"))
    }

    /// Terjemahkan tiap statement, lalu eksekusi berurutan. Mengembalikan
    /// hasil statement terakhir yang memproduksi data.
    fn run(&self, cypher: &str, params: Option<HashMap<String, Value>>) -> PyResult<QueryResult> {
        let stmts = translation::split_statements(cypher);
        if stmts.is_empty() {
            return Ok(QueryResult::from_native(akar_main::QueryResult::success_message(
                "(empty statement)".into(),
            )));
        }

        // Fase 1: terjemahkan semuanya dulu agar registri (dims, index, kolom)
        // terisi sebelum eksekusi (CREATE TABLE; CREATE VECTOR INDEX).
        let mut actions = Vec::with_capacity(stmts.len());
        {
            let mut tr = self
                .translator
                .lock()
                .map_err(|_| PyRuntimeError::new_err("translator lock poisoned"))?;
            for stmt in &stmts {
                actions.push(translation::translate(stmt, &mut tr).map_err(PyRuntimeError::new_err)?);
            }
        }

        // Fase 2: eksekusi.
        let conn = self.conn()?;
        let mut last: Option<QueryResult> = None;
        for action in actions {
            if let Some(result) = self.execute_action(conn, action, params.as_ref())? {
                last = Some(result);
            }
        }
        Ok(last.unwrap_or_else(|| QueryResult::from_native(akar_main::QueryResult::success_message("(ok)".into()))))
    }

    fn execute_action(
        &self,
        conn: &akar_main::Connection,
        action: Translated,
        params: Option<&HashMap<String, Value>>,
    ) -> PyResult<Option<QueryResult>> {
        match action {
            Translated::NoOp => Ok(None),
            Translated::Query(sql) => {
                let sql = self.interpolate(sql, params)?;
                let result = conn.query(&sql).map_err(PyRuntimeError::new_err)?;
                Ok(Some(QueryResult::from_native(result)))
            }
            Translated::Swallow(sql, needles) => {
                let sql = self.interpolate(sql, params)?;
                match conn.query(&sql) {
                    Ok(result) => Ok(Some(QueryResult::from_native(result))),
                    Err(err) if self.swallow(&err, needles) => Ok(None),
                    Err(err) => Err(PyRuntimeError::new_err(err)),
                }
            }
            Translated::CreateTableIfNotExists { table, sql } => {
                if self.table_exists(conn, &table)? {
                    Ok(None)
                } else {
                    let sql = self.interpolate(sql, params)?;
                    let result = conn.query(&sql).map_err(PyRuntimeError::new_err)?;
                    Ok(Some(QueryResult::from_native(result)))
                }
            }
            Translated::DropTableIfExists { table, sql } => match conn.query(&sql) {
                Ok(result) => {
                    if let Ok(mut tr) = self.translator.lock() {
                        tr.remove_table(&table);
                    }
                    Ok(Some(QueryResult::from_native(result)))
                }
                Err(err) if self.swallow(&err, ERR_NOT_FOUND) => Ok(None),
                Err(err) => Err(PyRuntimeError::new_err(err)),
            },
            Translated::VectorQuery {
                table,
                index_name,
                vec_expr,
                limit_expr,
                vec_col,
                where_sql,
            } => {
                let sql = self.build_vector_query(
                    conn,
                    &table,
                    &index_name,
                    &vec_expr,
                    &limit_expr,
                    &vec_col,
                    where_sql.as_deref(),
                )?;
                let sql = self.interpolate(sql, params)?;
                let result = conn.query(&sql).map_err(PyRuntimeError::new_err)?;
                Ok(Some(QueryResult::from_native(result)))
            }
        }
    }

    fn swallow(&self, err: &str, needles: &'static [&'static str]) -> bool {
        let lower = err.to_lowercase();
        needles.iter().any(|n| lower.contains(n))
    }

    fn interpolate(&self, sql: String, params: Option<&HashMap<String, Value>>) -> PyResult<String> {
        match params {
            Some(p) => param_interp::interpolate(&sql, p).map_err(PyRuntimeError::new_err),
            None => Ok(sql),
        }
    }

    /// `CALL show_tables()` → apakah tabel sudah ada (case-insensitive).
    fn table_exists(&self, conn: &akar_main::Connection, table: &str) -> PyResult<bool> {
        let result = conn.query("CALL show_tables()").map_err(PyRuntimeError::new_err)?;
        for chunk in &result.chunks {
            for row in 0..chunk.size {
                if let Some(Value::String(name)) = chunk.get_value(0, row) {
                    if name.eq_ignore_ascii_case(table) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    /// Nama kolom tabel untuk ekspansi `RETURN node` (P53.8). Sumber utama:
    /// registri translator (DDL yang sudah ditranslasi); fallback `CALL
    /// table_info('T')` untuk DB yang sudah ada dari sesi sebelumnya.
    fn ensure_table_schema(&self, conn: &akar_main::Connection, table: &str) -> PyResult<Vec<String>> {
        if let Ok(tr) = self.translator.lock() {
            if let Some(schema) = tr.table(table) {
                let cols = schema.column_names();
                if !cols.is_empty() {
                    return Ok(cols);
                }
            }
        }
        let query = format!("CALL table_info('{}')", table.replace('\'', "\\'"));
        let result = conn.query(&query).map_err(PyRuntimeError::new_err)?;
        let mut cols = Vec::new();
        for chunk in &result.chunks {
            for row in 0..chunk.size {
                if let Some(Value::String(name)) = chunk.get_value(1, row) {
                    if !INTERNAL_COLS.contains(&name.as_str()) {
                        cols.push(name);
                    }
                }
            }
        }
        if cols.is_empty() {
            return Err(PyRuntimeError::new_err(format!("Table '{table}' not found")));
        }
        Ok(cols)
    }

    /// Bangun brute-force `MATCH` yang meniru `CALL QUERY_VECTOR_INDEX(...)
    /// RETURN node, distance` (read-path HNSW write-only, P52.5).
    fn build_vector_query(
        &self,
        conn: &akar_main::Connection,
        table: &str,
        _index_name: &str,
        vec_expr: &str,
        limit_expr: &str,
        vec_col: &str,
        where_sql: Option<&str>,
    ) -> PyResult<String> {
        let cols = self.ensure_table_schema(conn, table)?;
        let props: Vec<String> = cols
            .iter()
            .map(|c| format!("{}: node.{}", qident(c), qident(c)))
            .collect();
        let mut q = format!("MATCH (node:{})", qident(table));
        if let Some(w) = where_sql {
            q.push(' ');
            q.push_str(w);
        }
        // ORDER BY memakai alias `distance` (bukan ulang ekspresi cosine):
        // ORDER BY pada ekspresi computed tak di-evaluasi sebagai sort key —
        // fallback posisional di `resolve_sort_keys` memetakan ke kolom 0
        // (P53.19, G9). Alias resolve via P53.16.
        q.push_str(&format!(
            " RETURN {{{}}} AS node, array_cosine_similarity(node.{}, {vec_expr}) AS distance \
             ORDER BY distance DESC LIMIT {limit_expr}",
            props.join(", "),
            qident(vec_col),
        ));
        Ok(q)
    }
}

/// Backtick-quote identifier bila tak sesuai pola `[A-Za-z_][A-Za-z0-9_]*`.
fn qident(s: &str) -> String {
    let plain = !s.is_empty()
        && (s.starts_with('_') || s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if plain {
        s.to_string()
    } else {
        format!("`{}`", s.replace('`', "``"))
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
            let row_end = if idx == slf.chunk_idx { slf.row_idx } else { 0 };
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
fn value_to_py(py: Python<'_>, v: Option<Value>) -> PyResult<PyObject> {
    let Some(v) = v else {
        return Ok(py.None());
    };
    let obj = match v {
        Value::Null => py.None(),
        Value::Bool(b) => PyBool::new(py, b).unbind().into_any(),
        Value::Int64(i) => PyInt::new(py, i).unbind().into_any(),
        Value::Int32(i) => PyInt::new(py, i).unbind().into_any(),
        Value::Int16(i) => PyInt::new(py, i).unbind().into_any(),
        Value::Int8(i) => PyInt::new(py, i).unbind().into_any(),
        Value::UInt64(u) => PyInt::new(py, u).unbind().into_any(),
        Value::UInt32(u) => PyInt::new(py, u).unbind().into_any(),
        Value::UInt16(u) => PyInt::new(py, u).unbind().into_any(),
        Value::UInt8(u) => PyInt::new(py, u).unbind().into_any(),
        Value::Int128(i) => PyInt::new(py, i).unbind().into_any(),
        Value::UInt128(u) => PyInt::new(py, u).unbind().into_any(),
        Value::Double(d) => PyFloat::new(py, d).unbind().into_any(),
        Value::Float(f) => PyFloat::new(py, f as f64).unbind().into_any(),
        Value::String(s) => PyString::new(py, &s).unbind().into_any(),
        Value::Blob(b) => PyBytes::new(py, &b).unbind().into_any(),
        Value::Date(d) => PyInt::new(py, d.0 as i64).unbind().into_any(),
        Value::Timestamp(t) => PyInt::new(py, t.0).unbind().into_any(),
        Value::TimestampTz(t) => PyInt::new(py, t.0).unbind().into_any(),
        Value::TimestampNs(t)
        | Value::TimestampMs(t)
        | Value::TimestampSec(t) => PyInt::new(py, t.0).unbind().into_any(),
        Value::Interval(_) => py.None(),
        Value::InternalID(id) => {
            let tuple = (id.table_id, id.offset);
            tuple.into_pyobject(py)?.unbind().into_any()
        }
        Value::Json(j) => serde_json_to_py(py, &j)?,
        Value::DTime(t) => PyInt::new(py, t).unbind().into_any(),
        Value::Union(_, inner) => value_to_py(py, Some(*inner))?,
        Value::List(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(value_to_py(py, Some(item))?)?;
            }
            list.unbind().into_any()
        }
        Value::Map(kvs) => {
            let d = PyDict::new(py);
            for (k, v) in kvs {
                d.set_item(value_to_py(py, Some(k))?, value_to_py(py, Some(v))?)?;
            }
            d.unbind().into_any()
        }
        Value::Struct(fields) => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static DB_SEQ: AtomicU64 = AtomicU64::new(0);

    fn fresh_conn() -> (std::path::PathBuf, Connection) {
        let n = DB_SEQ.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("akar_p53_1_{}_{}.db", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&path);
        let db = Arc::new(akar_main::Database::new(path.to_str().unwrap(), Default::default()).expect("open temp db"));
        let conn = Connection {
            conn: Some(akar_main::Connection::new(&db)),
            translator: Arc::new(Mutex::new(Translator::new())),
        };
        (path, conn)
    }

    /// Bootstrap `_ensure_schema()` persis dari `kairos/kuzudb_store.py:267-290`.
    const KAIROS_BOOTSTRAP: &str = "
INSTALL vector;
LOAD EXTENSION vector;
CREATE NODE TABLE IF NOT EXISTS Memory (
    id INT64, label STRING, content STRING,
    embedding FLOAT[384], salience DOUBLE, content_hash STRING,
    session_id STRING, prof STRING, scope STRING,
    colbert_tokens STRING,
    dae_embedding FLOAT[384],
    created_at DOUBLE, last_accessed DOUBLE, access_count INT64,
    protected BOOLEAN,
    PRIMARY KEY (id)
);
CREATE REL TABLE IF NOT EXISTS Connected (
    FROM Memory TO Memory,
    weight DOUBLE, type STRING, created_at DOUBLE,
    event_time DOUBLE, ingestion_time DOUBLE,
    valid_from DOUBLE, valid_to DOUBLE
);
CREATE NODE TABLE IF NOT EXISTS Revision (
    id INT64, memory_id INT64, old_content STRING,
    new_content STRING, reason STRING, created_at DOUBLE,
    PRIMARY KEY (id)
);
CREATE NODE TABLE IF NOT EXISTS Meta (
    key STRING, value STRING, PRIMARY KEY (key)
);
CREATE NODE TABLE IF NOT EXISTS Counter (
    key STRING, value INT64, PRIMARY KEY (key)
);
";

    fn show_table_names(conn: &Connection) -> Vec<String> {
        let result = conn
            .conn()
            .expect("show_tables")
            .query("CALL show_tables()")
            .expect("show_tables");
        let mut names = Vec::new();
        for chunk in &result.chunks {
            for row in 0..chunk.size {
                if let Some(Value::String(name)) = chunk.get_value(0, row) {
                    names.push(name);
                }
            }
        }
        names
    }

    #[test]
    fn kairos_ensure_schema_bootstrap_is_idempotent() {
        let (path, conn) = fresh_conn();

        conn.run(KAIROS_BOOTSTRAP, None).expect("bootstrap harus sukses");
        conn.run(KAIROS_BOOTSTRAP, None)
            .expect("bootstrap kedua harus idempoten");

        let names = show_table_names(&conn);
        for t in ["Memory", "Connected", "Revision", "Meta", "Counter"] {
            assert!(
                names.iter().any(|n| n.eq_ignore_ascii_case(t)),
                "tabel hilang {t}: {names:?}"
            );
        }

        conn.run("DROP TABLE IF EXISTS DoesNotExist", None)
            .expect("DROP IF EXISTS harus di-swallow");
        conn.run("DROP TABLE IF EXISTS Counter", None)
            .expect("DROP IF EXISTS tabel nyata");
        let names = show_table_names(&conn);
        assert!(
            !names.iter().any(|n| n.eq_ignore_ascii_case("Counter")),
            "Counter harus hilang: {names:?}"
        );

        conn.run("ALTER TABLE Memory ADD protected BOOLEAN DEFAULT false", None)
            .expect("ALTER ADD DEFAULT");
        conn.run("ALTER TABLE Memory ADD protected BOOLEAN DEFAULT false", None)
            .expect("ALTER ADD DEFAULT kedua (already exists) harus di-swallow");

        let _ = std::fs::remove_dir_all(&path);
    }

    fn fresh_db_path(tag: &str) -> std::path::PathBuf {
        let n = DB_SEQ.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("akar_{tag}_{}_{}.db", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn embed(n: f64) -> Value {
        let mut v = vec![0.0f64; 384];
        v[0] = n;
        v[1] = 1.0 - n * 0.1;
        Value::List(v.into_iter().map(Value::Double).collect())
    }

    /// G9 (P53.19): hasil translasi `CALL QUERY_VECTOR_INDEX(...) RETURN
    /// node, distance` (brute-force `MATCH ... RETURN {..} AS node,
    /// array_cosine_similarity(...) ...`) harus parse & eksekusi — dulu
    /// gagal parse 1:141 pada probe terisolasi (pra G2).
    #[test]
    fn p5319_g9_query_vector_index_translation() {
        let (path, conn) = fresh_conn();

        conn.run(KAIROS_BOOTSTRAP, None).expect("bootstrap");
        conn.run(
            "CALL CREATE_VECTOR_INDEX('Memory', 'mem_vec', 'embedding', metric := 'cosine')",
            None,
        )
        .expect("create index");

        for i in 0..4 {
            let params = HashMap::from([
                ("id".to_string(), Value::Int64(i)),
                ("content".to_string(), Value::String(format!("memory {i}"))),
                ("vec".to_string(), embed(i as f64)),
                ("sal".to_string(), Value::Double(0.5 + i as f64 * 0.1)),
            ]);
            conn.run(
                "CREATE (:Memory {id: $id, content: $content, embedding: $vec, salience: $sal})",
                Some(params),
            )
            .expect("insert");
        }

        let params = HashMap::from([
            ("query_vec".to_string(), embed(0.0)),
            ("limit".to_string(), Value::Int64(2)),
        ]);
        let q = conn
            .run(
                "CALL QUERY_VECTOR_INDEX('Memory', 'mem_vec', $query_vec, $limit) RETURN node, distance",
                Some(params),
            )
            .expect("vector query");

        let native = &q.result;
        let mut rows = 0usize;
        let mut prev_dist = f64::INFINITY;
        for chunk in &native.chunks {
            for row in 0..chunk.size {
                rows += 1;
                let node = chunk.get_value(0, row).expect("node column");
                let dist = chunk.get_value(1, row).expect("distance column");
                assert!(matches!(node, Value::Struct(_)), "node harus dict/struct, got {node:?}");
                let Value::Double(d) = dist else {
                    panic!("distance harus Double, got {dist:?}");
                };
                assert!(
                    (d - 1.0).abs() < 1e-6 || rows > 1,
                    "baris pertama (nearest) harus ~1.0, got {d}"
                );
                assert!(
                    d <= prev_dist + 1e-9,
                    "distance harus DESC, got {d} setelah {prev_dist}"
                );
                prev_dist = d;
            }
        }
        assert_eq!(rows, 2, "LIMIT 2 harus 2 baris");

        let _ = std::fs::remove_dir_all(&path);
    }

    /// G7 (P53.18): pola kairos close→checkpoint→reopen. `Database.close()`
    /// harus menutup paksa connection yang masih hidup sehingga file lock
    /// dilepas meskipun wrapper `Connection` Python masih direferensikan.
    #[test]
    fn db_close_releases_lock_even_with_live_connection() {
        let path = fresh_db_path("p53_18_reopen");

        Python::with_gil(|py| {
            let db = Bound::new(py, Database::new(path.to_str().unwrap()).expect("open temp db")).expect("wrap db");
            let _conn = Connection::new(&db).expect("create connection");
            db.borrow_mut().close(py);
            drop(db);

            let reopened =
                akar_main::Database::new(path.clone(), Default::default()).expect("reopen same path after close");
            drop(reopened);
        });

        let _ = std::fs::remove_dir_all(&path);
    }

    /// E3 (P53.35): replikasi flow harness `test_close_and_reopen` — dua
    /// `Database` Python pada path yang sama hidup bersamaan (fixture store +
    /// store milik test). Lock lintas-proses kini reentrant dalam satu proses:
    /// open kedua berbagi lock yang sama alih-alih gagal, dan setelah semua
    /// instance ditutup path bisa dibuka lagi.
    #[test]
    fn two_databases_on_same_path_coexist_then_reopen() {
        let path = fresh_db_path("p53_35_reopen");

        Python::with_gil(|py| {
            // fixture store
            let fixture = Bound::new(py, Database::new(path.to_str().unwrap()).expect("open temp db")).expect("wrap db");
            // store milik test — open kedua pada path sama, harus sukses (P53.35)
            let s = Bound::new(py, Database::new(path.to_str().unwrap()).expect("second open shares lock")).expect("wrap db");
            let _conn_s = Connection::new(&s).expect("connection on second db");
            s.borrow_mut().close(py);
            drop(s);

            // fixture masih hidup, path dibuka ulang — harus sukses (share)
            let s2 = Bound::new(py, Database::new(path.to_str().unwrap()).expect("reopen while fixture alive")).expect("wrap db");
            s2.borrow_mut().close(py);
            drop(s2);

            // fixture ditutup → semua refcount habis → lock dilepas
            fixture.borrow_mut().close(py);
            drop(fixture);

            let reopened = akar_main::Database::new(path.clone(), Default::default()).expect("reopen after all closed");
            drop(reopened);
        });

        let _ = std::fs::remove_dir_all(&path);
    }

    /// `Connection.close()` melepas `Arc<Database>`-nya; menggabungkan dengan
    /// `Database.close()` memungkinkan path yang sama dibuka ulang.
    #[test]
    fn connection_close_releases_lock_before_db_close() {
        let path = fresh_db_path("p53_18_conn_close");

        Python::with_gil(|py| {
            let db = Bound::new(py, Database::new(path.to_str().unwrap()).expect("open temp db")).expect("wrap db");
            let conn = Connection::new(&db).expect("create connection");
            conn.borrow_mut(py).close();
            db.borrow_mut().close(py);
            drop(conn);
            drop(db);

            let reopened =
                akar_main::Database::new(path.clone(), Default::default()).expect("reopen same path after close");
            drop(reopened);
        });

        let _ = std::fs::remove_dir_all(&path);
    }

    /// Connection tidak bisa dibuat setelah Database ditutup, dan query pada
    /// connection yang sudah ditutup harus error (bukan panic).
    #[test]
    fn closed_database_and_connection_reject_use() {
        let path = fresh_db_path("p53_18_reject");

        Python::with_gil(|py| {
            let db = Bound::new(py, Database::new(path.to_str().unwrap()).expect("open temp db")).expect("wrap db");
            let conn = Connection::new(&db).expect("create connection");
            db.borrow_mut().close(py);
            assert!(
                Connection::new(&db).is_err(),
                "Connection::new after Database.close() harus error"
            );
            conn.borrow_mut(py).close();
            assert!(
                conn.borrow(py).query("RETURN 1").is_err(),
                "query setelah Connection.close() harus error"
            );
        });

        let _ = std::fs::remove_dir_all(&path);
    }
}
