// kuzu_main::test_httpfs.rs

#[test]
#[cfg(feature = "httpfs-extension")]
fn test_httpfs_extension() -> Result<(), String> {
    let dir = tempdir().map_err(|e| e.to_string())?;
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).map_err(|e| e.to_string())?);
    let conn = Connection::new(&db);

    // Test http_get
    let res = conn.query("RETURN http_get('https://example.com/') AS body")?;
    let chunk = res.chunks.first().unwrap();
    let body = match chunk.fields[0].get_value(0).unwrap() {
        kuzu_common::types::Value::String(s) => s,
        _ => panic!("Expected string"),
    };
    assert!(body.contains("Example Domain"), "http_get should fetch the URL successfully");

    // Test http_scan
    let res2 = conn.query("CALL http_scan('https://example.com/')").unwrap();
    println!("RES2: {:?}", res2);
    // Verify results
    let path = res2.message.expect("http_scan should return a message with the path");
    assert!(!path.is_empty(), "http_scan should return a temp file path");

    Ok(())
}
