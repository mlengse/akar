#![no_main]
use libfuzzer_sys::fuzz_target;
use akar_main::{Database, Connection, SystemConfig};
use std::io::Write;
use tempfile::NamedTempFile;

fuzz_target!(|data: &[u8]| {
    if let Ok(csv_content) = std::str::from_utf8(data) {
        let db = Database::new(":memory:", SystemConfig::default()).unwrap();
        let conn = Connection::new(&std::sync::Arc::new(db));
        
        // Setup table
        let _ = conn.query("CREATE NODE TABLE FuzzNode(id INT64, val STRING, PRIMARY KEY(id))");
        
        if let Ok(mut temp_file) = NamedTempFile::new() {
            if temp_file.write_all(csv_content.as_bytes()).is_ok() {
                let path = temp_file.path().to_str().unwrap().replace("\\", "/");
                let query = format!("COPY FuzzNode FROM '{}'", path);
                let _ = conn.query(&query);
            }
        }
    }
});
