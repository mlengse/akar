use akar_main::{Connection, Database, SystemConfig};
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_migration_ingestion() {
    let mock_cpp_dir = tempdir().unwrap();

    // 1. Generate Parquet data using our own engine (mocking the C++ export)
    let db = std::sync::Arc::new(Database::new(mock_cpp_dir.path(), SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);

    conn.query("CREATE NODE TABLE User(id INT64, name STRING, PRIMARY KEY(id))")
        .unwrap();
    conn.query("CREATE (u:User {id: 1, name: 'Alice'})").unwrap();
    conn.query("CREATE (u:User {id: 2, name: 'Bob'})").unwrap();

    let parquet_path = mock_cpp_dir.path().join("User.parquet");
    let parquet_path_str = parquet_path.to_str().unwrap().replace("\\", "/");
    conn.query(&format!(
        "COPY (MATCH (a:User) RETURN a.id, a.name) TO '{}' (FORMAT PARQUET)",
        parquet_path_str
    ))
    .unwrap();

    // Validate the parquet file is readable by performing COPY FROM in the same connection
    conn.query("CREATE NODE TABLE ImportedUser(id INT64, name STRING, PRIMARY KEY(id))")
        .unwrap();
    let import_path_str = parquet_path_str.clone();
    conn.query(&format!("COPY ImportedUser FROM '{}'", import_path_str))
        .unwrap();
    let result = conn
        .query("MATCH (u:ImportedUser) RETURN u.id, u.name ORDER BY u.id")
        .unwrap();
    let mut rows = Vec::new();
    for chunk in &result.chunks {
        for row_idx in 0..chunk.size {
            let id = format!("{:?}", chunk.get_value(0, row_idx).unwrap());
            let name = format!("{:?}", chunk.get_value(1, row_idx).unwrap());
            rows.push((id, name));
        }
    }
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "Int64(1)");
    assert_eq!(rows[0].1, "String(\"Alice\")");
    assert_eq!(rows[1].0, "Int64(2)");
    assert_eq!(rows[1].1, "String(\"Bob\")");

    // Write a mock schema.json
    let schema_json = r#"{
        "tables": [
            {
                "name": "User",
                "type": "NODE",
                "properties": [
                    {"name": "id", "type": "INT64", "is_primary_key": true},
                    {"name": "name", "type": "STRING", "is_primary_key": false}
                ]
            }
        ],
        "connections": []
    }"#;
    fs::write(mock_cpp_dir.path().join("schema.json"), schema_json).unwrap();
    // Drop DB to release any file locks before running Akar-migrate
    drop(db);
    drop(conn);

    // 2. Run the akar-migrate CLI binary directly.
    //
    // Uses `CARGO_BIN_EXE_akar-migrate` (set by cargo when building this
    // integration test) instead of a nested `cargo run -p akar-migrate`.
    // A nested cargo invocation would trigger a separate (re)build and contend
    // for the workspace build lock from inside the test; the env var points at
    // the already-built binary, so no recompilation happens.
    let binary = env!("CARGO_BIN_EXE_akar-migrate");
    let from_path = mock_cpp_dir.path().to_str().unwrap().replace("\\", "/");
    let to_path = mock_cpp_dir.path().to_str().unwrap().replace("\\", "/");
    let status = Command::new(binary)
        .args(["--from", &from_path, "--to", &to_path, "--skip-extract"])
        .status()
        .expect("Failed to execute akar-migrate");

    assert!(status.success(), "akar-migrate failed");
}
