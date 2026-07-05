use wasm_bindgen::prelude::*;
use kuzu_main::{Database, Connection};
use std::sync::Arc;

#[wasm_bindgen]
pub struct KuzuDatabase {
    db: Arc<Database>,
}

#[wasm_bindgen]
impl KuzuDatabase {
    #[wasm_bindgen(constructor)]
    pub fn new(db_path: &str) -> Result<KuzuDatabase, JsValue> {
        let db = Database::new(db_path, Default::default())
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
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
    pub fn query(&self, query: &str) -> Result<JsValue, JsValue> {
        let result = self.conn.query(query)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        // Simple serialization for now (JSON-like or just success string)
        Ok(JsValue::from_str(&format!("Query executed, {} rows returned", result.num_rows())))
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}
