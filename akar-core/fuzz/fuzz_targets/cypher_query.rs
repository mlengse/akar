#![no_main]
use libfuzzer_sys::fuzz_target;
use akar_main::{Database, Connection, SystemConfig};

fuzz_target!(|data: &[u8]| {
    if let Ok(query) = std::str::from_utf8(data) {
        if query.trim().is_empty() {
            return;
        }
        let db = Database::new(":memory:", SystemConfig::default()).unwrap();
        let conn = Connection::new(&std::sync::Arc::new(db));
        // We don't care if the query fails to parse or execute,
        // we just want to make sure it doesn't panic.
        let _ = conn.query(query);
    }
});
