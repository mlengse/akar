use kuzu_main::{Connection, Database, QueryResult as NativeQueryResult, PreparedStatement as NativePreparedStatement};
use kuzu_common::types::Value;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct KuzuDatabase {
    db: Arc<Database>,
}

#[wasm_bindgen]
impl KuzuDatabase {
    #[wasm_bindgen(constructor)]
    pub fn new(db_path: &str) -> Result<KuzuDatabase, JsValue> {
        let db = Database::new(db_path, Default::default()).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(KuzuDatabase { db: Arc::new(db) })
    }
}

#[wasm_bindgen]
pub struct KuzuPreparedStatement {
    stmt: NativePreparedStatement,
}

#[wasm_bindgen]
pub struct KuzuConnection {
    conn: Connection,
}

#[wasm_bindgen]
impl KuzuConnection {
    #[wasm_bindgen(constructor)]
    pub fn new(database: &KuzuDatabase) -> Result<KuzuConnection, JsValue> {
        let conn = Connection::new(&database.db);
        Ok(KuzuConnection { conn })
    }

    #[wasm_bindgen]
    pub fn query(&self, query: &str) -> Result<QueryResult, JsValue> {
        let result = self.conn.query(query).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(QueryResult {
            result,
            chunk_idx: 0,
            row_idx: 0,
        })
    }

    #[wasm_bindgen]
    pub fn prepare(&self, query: &str) -> Result<KuzuPreparedStatement, JsValue> {
        let stmt = self.conn.prepare(query).map_err(|e| JsValue::from_str(&e))?;
        Ok(KuzuPreparedStatement { stmt })
    }

    #[wasm_bindgen]
    pub fn execute(&self, prepared: &KuzuPreparedStatement, params: &js_sys::Object) -> Result<QueryResult, JsValue> {
        let keys = js_sys::Object::keys(params);
        let mut key_strings = Vec::new();
        for i in 0..keys.length() {
            let key_val = js_sys::Array::from(&keys).get(i);
            if let Some(k) = key_val.as_string() {
                key_strings.push(k);
            }
        }
        
        let mut param_vec = Vec::new();
        for k in &key_strings {
            let js_key = JsValue::from_str(k);
            let js_val = js_sys::Reflect::get(params, &js_key).map_err(|_| JsValue::from_str("Failed to get property"))?;
            
            let val = if js_val.is_null() || js_val.is_undefined() {
                Value::Null
            } else if let Some(b) = js_val.as_bool() {
                Value::Bool(b)
            } else if let Some(f) = js_val.as_f64() {
                if f.fract() == 0.0 {
                    Value::Int64(f as i64)
                } else {
                    Value::Double(f)
                }
            } else if let Some(s) = js_val.as_string() {
                Value::String(s)
            } else {
                return Err(JsValue::from_str(&format!("Unsupported parameter type for key {}", k)));
            };
            
            param_vec.push((k.as_str(), val));
        }

        let result = self.conn.execute(&prepared.stmt, param_vec).map_err(|e| JsValue::from_str(&e))?;
        Ok(QueryResult {
            result,
            chunk_idx: 0,
            row_idx: 0,
        })
    }
}

#[wasm_bindgen]
pub struct QueryResult {
    result: NativeQueryResult,
    chunk_idx: usize,
    row_idx: usize,
}

#[wasm_bindgen]
impl QueryResult {
    #[wasm_bindgen]
    pub fn is_success(&self) -> bool {
        self.result.is_success()
    }

    #[wasm_bindgen]
    pub fn get_error_message(&self) -> Option<String> {
        self.result.error_message.clone()
    }

    #[wasm_bindgen]
    pub fn get_num_rows(&self) -> usize {
        self.result.num_rows()
    }

    #[wasm_bindgen]
    pub fn has_next(&self) -> bool {
        if self.chunk_idx >= self.result.chunks.len() {
            return false;
        }
        let chunk = &self.result.chunks[self.chunk_idx];
        self.row_idx < chunk.size
    }

    #[wasm_bindgen]
    pub fn get_next(&mut self) -> Result<JsValue, JsValue> {
        if !self.has_next() {
            return Ok(JsValue::NULL);
        }

        let chunk = &self.result.chunks[self.chunk_idx];
        let obj = js_sys::Object::new();

        for (col_idx, field_vec) in chunk.fields.iter().enumerate() {
            let val_opt = field_vec.get_value(self.row_idx);
            let js_val = match val_opt {
                Some(v) => serde_wasm_bindgen::to_value(&v)
                    .map_err(|e| JsValue::from_str(&format!("Serialization error: {}", e)))?,
                None => JsValue::NULL,
            };

            // Use field name if available, else column index
            let key = if chunk.field_names.len() > col_idx {
                chunk.field_names[col_idx].clone()
            } else {
                format!("_{}", col_idx)
            };

            let js_key = JsValue::from_str(&key);
            js_sys::Reflect::set(&obj, &js_key, &js_val)
                .map_err(|_| JsValue::from_str("Failed to set object property"))?;
        }

        // Advance iterator
        self.row_idx += 1;
        if self.row_idx >= chunk.size {
            self.chunk_idx += 1;
            self.row_idx = 0;
        }

        Ok(obj.into())
    }

    #[wasm_bindgen]
    pub fn get_column_names(&self) -> Result<js_sys::Array, JsValue> {
        let arr = js_sys::Array::new();
        if !self.result.chunks.is_empty() {
            for name in &self.result.chunks[0].field_names {
                arr.push(&JsValue::from_str(name));
            }
        }
        Ok(arr)
    }

    #[wasm_bindgen]
    pub fn reset_iterator(&mut self) {
        self.chunk_idx = 0;
        self.row_idx = 0;
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}
