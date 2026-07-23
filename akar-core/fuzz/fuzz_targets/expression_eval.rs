#![no_main]
use libfuzzer_sys::fuzz_target;
use akar_main::{Database, Connection, SystemConfig};

fuzz_target!(|data: &[u8]| {
    if let Ok(expr) = std::str::from_utf8(data) {
        if expr.trim().is_empty() {
            return;
        }
        let db = Database::new(":memory:", SystemConfig::default()).unwrap();
        let conn = Connection::new(&std::sync::Arc::new(db));
        
        let query = format!("RETURN {}", expr);
        let _ = conn.query(&query);
    }
});
