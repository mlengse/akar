use kuzu_main::{Connection, Database, QueryResult as NativeQueryResult};
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
