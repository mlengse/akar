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
        })
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
    translator: Arc<Mutex<Translator>>,
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
            translator: database.translator.clone(),
        })
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

    fn close(&mut self) {}

    fn __repr__(&self) -> String {
        "<akar.Connection>".to_string()
    }
}

impl Connection {
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
        let mut last: Option<QueryResult> = None;
        for action in actions {
            if let Some(result) = self.execute_action(action, params.as_ref())? {
                last = Some(result);
            }
        }
        Ok(last.unwrap_or_else(|| QueryResult::from_native(akar_main::QueryResult::success_message("(ok)".into()))))
    }

    fn execute_action(
        &self,
        action: Translated,
        params: Option<&HashMap<String, Value>>,
    ) -> PyResult<Option<QueryResult>> {
        match action {
            Translated::NoOp => Ok(None),
            Translated::Query(sql) => {
                let sql = self.interpolate(sql, params)?;
                let result = self.conn.query(&sql).map_err(PyRuntimeError::new_err)?;
                Ok(Some(QueryResult::from_native(result)))
            }
            Translated::Swallow(sql, needles) => {
                let sql = self.interpolate(sql, params)?;
                match self.conn.query(&sql) {
                    Ok(result) => Ok(Some(QueryResult::from_native(result))),
                    Err(err) if self.swallow(&err, needles) => Ok(None),
                    Err(err) => Err(PyRuntimeError::new_err(err)),
                }
            }
            Translated::CreateTableIfNotExists { table, sql } => {
                if self.table_exists(&table)? {
                    Ok(None)
                } else {
                    let sql = self.interpolate(sql, params)?;
                    let result = self.conn.query(&sql).map_err(PyRuntimeError::new_err)?;
                    Ok(Some(QueryResult::from_native(result)))
                }
            }
            Translated::DropTableIfExists { table, sql } => match self.conn.query(&sql) {
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
                    &table,
                    &index_name,
                    &vec_expr,
                    &limit_expr,
                    &vec_col,
                    where_sql.as_deref(),
                )?;
                let sql = self.interpolate(sql, params)?;
                let result = self.conn.query(&sql).map_err(PyRuntimeError::new_err)?;
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
    fn table_exists(&self, table: &str) -> PyResult<bool> {
        let result = self.conn.query("CALL show_tables()").map_err(PyRuntimeError::new_err)?;
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
    fn ensure_table_schema(&self, table: &str) -> PyResult<Vec<String>> {
        if let Ok(tr) = self.translator.lock() {
            if let Some(schema) = tr.table(table) {
                let cols = schema.column_names();
                if !cols.is_empty() {
                    return Ok(cols);
                }
            }
        }
        let query = format!("CALL table_info('{}')", table.replace('\'', "\\'"));
        let result = self.conn.query(&query).map_err(PyRuntimeError::new_err)?;
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
        table: &str,
        _index_name: &str,
        vec_expr: &str,
        limit_expr: &str,
        vec_col: &str,
        where_sql: Option<&str>,
    ) -> PyResult<String> {
        let cols = self.ensure_table_schema(table)?;
        let props: Vec<String> = cols
            .iter()
            .map(|c| format!("{}: node.{}", qident(c), qident(c)))
            .collect();
        let mut q = format!("MATCH (node:{})", qident(table));
        if let Some(w) = where_sql {
            q.push(' ');
            q.push_str(w);
        }
        q.push_str(&format!(
            " RETURN {{{}}} AS node, array_cosine_similarity(node.{}, {vec_expr}) AS distance \
             ORDER BY array_cosine_similarity(node.{}, {vec_expr}) DESC LIMIT {limit_expr}",
            props.join(", "),
            qident(vec_col),
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
        akar_common::types::Value::TimestampNs(t)
        | akar_common::types::Value::TimestampMs(t)
        | akar_common::types::Value::TimestampSec(t) => PyInt::new(py, t.0).unbind().into_any(),
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
            conn: akar_main::Connection::new(&db),
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
        let result = conn.conn.query("CALL show_tables()").expect("show_tables");
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
}
