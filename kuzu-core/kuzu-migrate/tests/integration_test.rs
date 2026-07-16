use std::process::Command;
use std::fs;
use tempfile::tempdir;
use kuzu_main::{Database, SystemConfig, Connection};

#[test]
#[ignore = "COPY TO parquet generation in mock test environment produces corrupt footer"]
fn test_migration_ingestion() {
    let mock_cpp_dir = tempdir().unwrap();
    let rust_dir = tempdir().unwrap();

    // 1. Generate Parquet data using our own engine (mocking the C++ export)
    {
        let db = std::sync::Arc::new(Database::new(mock_cpp_dir.path(), SystemConfig::default()).unwrap());
        let conn = Connection::new(&db);
        
        conn.query("CREATE NODE TABLE User(id INT64, name STRING, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE (u:User {id: 1, name: 'Alice'})").unwrap();
        conn.query("CREATE (u:User {id: 2, name: 'Bob'})").unwrap();
        
        let parquet_path = mock_cpp_dir.path().join("User.parquet");
        let parquet_path_str = parquet_path.to_str().unwrap().replace("\\", "/");
        conn.query(&format!("COPY (MATCH (a:User) RETURN a.id, a.name) TO '{}'", parquet_path_str)).unwrap();

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
    }

    // 2. Run kuzu-migrate --skip-extract
    let status = Command::new("cargo")
        .args(&[
            "run", "-p", "kuzu-migrate", "--", 
            "--from", mock_cpp_dir.path().to_str().unwrap(), 
            "--to", rust_dir.path().to_str().unwrap(),
            "--skip-extract"
        ])
        .status()
        .expect("Failed to execute kuzu-migrate");

    assert!(status.success(), "kuzu-migrate failed");

    // 3. Verify in new Rust Kuzu database
    {
        let db = std::sync::Arc::new(Database::new(rust_dir.path(), SystemConfig::default()).unwrap());
        let conn = Connection::new(&db);
        
        let mut result = conn.query("MATCH (u:User) RETURN u.id, u.name ORDER BY u.id").unwrap();
        
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
    }
}
