//! Integration tests for Connection — extracted from `connection.rs`.
//!
//! Test categories:
//! - integration_tests: COPY, DDL, DML, edge cases, auto-checkpoint
//! - merge_tests: MERGE behavior
//! - call_tests: CALL function behavior
//! - create_dml_tests: CREATE DML behavior
//! - foreach_tests: FOREACH behavior
//! - var_length_path_tests: Variable-length path parsing
//! - subquery_tests: Subquery behavior

#[cfg(test)]
mod integration_tests {
    use crate::connection::Connection;
    use crate::database::{Database, SystemConfig};
    use kuzu_common::types::Value;
    use std::sync::Arc;

    /// Create a temporary Database and Connection for testing.
    fn setup_db() -> (tempfile::TempDir, Arc<Database>, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_db");
        let config = SystemConfig::default();
        let database = Arc::new(Database::new(db_path, config).unwrap());
        let conn = Connection::new(&database);
        (dir, database, conn)
    }

    /// Helper: execute a query and return whether it succeeded.
    fn exec_ok(conn: &Connection, sql: &str) -> Result<String, String> {
        conn.query(sql).map(|r| r.to_string())
    }

    /// Helper: extract all values from the first column of a query result.
    fn query_column(conn: &Connection, sql: &str) -> Vec<Value> {
        let result = conn.query(sql).unwrap();
        result
            .chunks
            .iter()
            .flat_map(|c| (0..c.size).filter_map(|i| c.fields.first().and_then(|f| f.get_value(i))))
            .collect()
    }

    #[test]
    fn test_sequence_nextval_currval_query_e2e() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE SEQUENCE my_seq START 10 INCREMENT 2").unwrap();

        let r1 = query_column(&conn, "RETURN nextval('my_seq')");
        assert_eq!(r1, vec![Value::Int64(10)]);

        let r2 = query_column(&conn, "RETURN nextval('my_seq')");
        assert_eq!(r2, vec![Value::Int64(12)]);

        // currval should not advance.
        let c1 = query_column(&conn, "RETURN currval('my_seq')");
        assert_eq!(c1, vec![Value::Int64(14)]);

        let c2 = query_column(&conn, "RETURN currval('my_seq')");
        assert_eq!(c2, vec![Value::Int64(14)]);

        let r3 = query_column(&conn, "RETURN nextval('my_seq')");
        assert_eq!(r3, vec![Value::Int64(14)]);
    }

    #[test]
    fn test_sequence_missing_error_query_e2e() {
        let (_dir, _db, conn) = setup_db();
        let err = conn.query("RETURN nextval('does_not_exist')").unwrap_err();
        assert!(
            err.contains("Sequence 'does_not_exist' not found"),
            "Unexpected error: {err}"
        );
    }

    #[test]
    fn test_copy_csv_with_header() {
        let (dir, _db, conn) = setup_db();
        let db_path = dir.path().join("test_db");
        let _ = &db_path; // keep db path alive

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (name))",
        )
        .unwrap();

        let csv_path = dir.path().join("people.csv");
        std::fs::write(
            &csv_path,
            "name,age,score,active\nAlice,30,95.5,true\nBob,25,87.3,false\nCharlie,35,91.2,true\n",
        )
        .unwrap();

        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY Person FROM '{file_path}' (HEADER true)")).unwrap();

        let names = query_column(&conn, "MATCH (n:Person) RETURN n.name ORDER BY n.name");
        let extracted: Vec<String> = names
            .iter()
            .filter_map(|v| {
                if let Value::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(extracted, vec!["Alice", "Bob", "Charlie"]);
    }

    #[test]
    fn test_copy_csv_no_header() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (name))",
        )
        .unwrap();

        let csv_path = _dir.path().join("noheader.csv");
        std::fs::write(&csv_path, "Alice,30,95.5,true\nBob,25,87.3,false\n").unwrap();

        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY Person FROM '{file_path}' (HEADER false)")).unwrap();

        // Verify: MATCH should return 2 rows (even if column projection is raw)
        let names = query_column(&conn, "MATCH (n:Person) RETURN n.name");
        assert_eq!(names.len(), 2, "Expected 2 rows in MATCH result");
    }

    #[test]
    fn test_copy_csv_custom_delimiter() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Item(name STRING, price DOUBLE, PRIMARY KEY (name))",
        )
        .unwrap();

        let csv_path = _dir.path().join("items.csv");
        std::fs::write(&csv_path, "name|price\nWidget|19.99\nGadget|29.99\n").unwrap();

        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY Item FROM '{file_path}' (HEADER true, DELIM '|')")).unwrap();

        // Verify 2 rows were inserted
        let names = query_column(&conn, "MATCH (n:Item) RETURN n.name");
        assert_eq!(names.len(), 2, "Expected 2 rows after pipe-delimited COPY");
    }

    #[test]
    fn test_copy_csv_file_not_found() {
        let (dir, _db, conn) = setup_db();
        let _ = &dir;

        exec_ok(&conn, "CREATE NODE TABLE T(name STRING, PRIMARY KEY (name))").unwrap();

        let result = exec_ok(&conn, "COPY T FROM 'nonexistent.csv' (HEADER true)");
        assert!(result.is_err(), "Expected file not found error");
        let err = result.unwrap_err();
        assert!(err.contains("not found"), "Expected file not found error, got: {err}");
    }

    #[test]
    fn test_copy_csv_type_mismatch() {
        let (dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(name STRING, age INT64, PRIMARY KEY (name))").unwrap();

        let csv_path = dir.path().join("bad.csv");
        std::fs::write(&csv_path, "name,age\nAlice,not_a_number\n").unwrap();

        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        let result = exec_ok(&conn, &format!("COPY T FROM '{file_path}' (HEADER true)"));
        assert!(result.is_err(), "Expected type coercion error");
        let err = result.unwrap_err();
        assert!(
            err.contains("INT64") || err.contains("parse"),
            "Expected type error, got: {err}"
        );
    }

    #[test]
    fn test_copy_csv_column_count_mismatch() {
        let (dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(name STRING, age INT64, PRIMARY KEY (name))").unwrap();

        let csv_path = dir.path().join("bad_cols.csv");
        std::fs::write(&csv_path, "name,age,extra\nAlice,30,oops\n").unwrap();

        // Without DELIM option, the binder skips header validation.
        // The CSV reader will detect the mismatch and error at read time.
        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        let result = exec_ok(&conn, &format!("COPY T FROM '{file_path}' (HEADER true)"));
        assert!(result.is_err(), "Expected column count error");
        let err = result.unwrap_err();
        assert!(
            err.contains("Column count mismatch") || err.contains("match"),
            "Expected column count error, got: {err}"
        );
    }

    #[test]
    fn test_copy_parquet_roundtrip() {
        use arrow::array::*;
        use arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;

        let (dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (name))",
        )
        .unwrap();

        let pq_path = dir.path().join("data.parquet");
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("age", DataType::Int64, false),
            Field::new("score", DataType::Float64, false),
            Field::new("active", DataType::Boolean, false),
        ]));

        let batch = arrow::record_batch::RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["Alice", "Bob"])),
                Arc::new(Int64Array::from(vec![30i64, 25])),
                Arc::new(Float64Array::from(vec![95.5, 87.3])),
                Arc::new(BooleanArray::from(vec![true, false])),
            ],
        )
        .unwrap();

        let file = std::fs::File::create(&pq_path).unwrap();
        let mut writer = parquet::arrow::ArrowWriter::try_new(file, batch.schema(), None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let file_path = pq_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY Person FROM '{file_path}'")).unwrap();

        // Verify 2 rows were inserted via Parquet
        let names = query_column(&conn, "MATCH (n:Person) RETURN n.name");
        assert_eq!(names.len(), 2, "Expected 2 rows from Parquet COPY");
    }

    #[test]
    fn test_copy_csv_tab_delimiter() {
        let (dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(a STRING, b INT64, PRIMARY KEY (a))").unwrap();

        let csv_path = dir.path().join("data.tsv");
        std::fs::write(&csv_path, "a\tb\nx\t10\ny\t20\n").unwrap();

        // .tsv extension triggers tab delimiter in PhysicalCopyFrom.
        // Without a DELIM option, the binder skips header validation.
        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{file_path}' (HEADER true)")).unwrap();

        // Verify 2 rows were inserted via TSV
        let vals = query_column(&conn, "MATCH (n:T) RETURN n.b");
        assert_eq!(vals.len(), 2, "Expected 2 rows from TSV COPY");
    }

    #[test]
    fn test_copy_empty_csv() {
        let (dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(name STRING, PRIMARY KEY (name))").unwrap();

        let csv_path = dir.path().join("empty.csv");
        std::fs::write(&csv_path, "name\n").unwrap(); // header only

        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{file_path}' (HEADER true)")).unwrap();

        let vals = query_column(&conn, "MATCH (n:T) RETURN n.name");
        assert!(vals.is_empty(), "Expected no rows in empty CSV");
    }

    // ==================== Edge Case Tests ====================

    #[test]
    fn test_empty_table_scan() {
        // Scan an empty table (no data) — should return empty result, not error
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Empty(id INT64, label STRING, PRIMARY KEY (id))",
        )
        .unwrap();

        let result = conn.query("MATCH (n:Empty) RETURN n.id, n.label").unwrap();
        // Should produce a valid empty result
        assert_eq!(result.num_rows(), 0);
    }

    #[test]
    fn test_empty_match_return() {
        // Query with a WHERE clause that matches nothing
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(id INT64, PRIMARY KEY (id))").unwrap();

        // Insert some data then query with impossible filter
        let csv_path = _dir.path().join("data.csv");
        std::fs::write(&csv_path, "id\n1\n2\n3\n").unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{fp}' (HEADER true)")).unwrap();

        let result = conn.query("MATCH (n:T) WHERE n.id > 100 RETURN n.id").unwrap();
        assert_eq!(result.num_rows(), 0, "Expected 0 rows for impossible filter");
    }

    #[test]
    fn test_return_star_basic() {
        // RETURN * with MATCH should expand to all variables in scope
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(id INT64, label STRING, PRIMARY KEY (id))").unwrap();

        let csv_path = _dir.path().join("data.csv");
        std::fs::write(&csv_path, "id,label\n1,alice\n2,bob\n").unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{fp}' (HEADER true)")).unwrap();

        let result = conn.query("MATCH (n:T) RETURN *");
        assert!(result.is_ok(), "RETURN * should succeed: {:?}", result);
        if let Ok(r) = result {
            assert!(r.num_rows() > 0, "RETURN * should return rows");
        }
    }

    #[test]
    fn test_return_star_no_variables() {
        // RETURN * with no MATCH should fail with clear error
        let (_dir, _db, conn) = setup_db();
        let result = conn.query("RETURN *");
        assert!(result.is_err(), "RETURN * without variables should error");
    }

    #[test]
    fn test_lower_function_alias() {
        let (_dir, _db, conn) = setup_db();
        let result = conn.query("RETURN lower('HELLO') AS v");
        assert!(result.is_ok(), "lower() should work: {:?}", result);
    }

    #[test]
    fn test_upper_function_alias() {
        let (_dir, _db, conn) = setup_db();
        let result = conn.query("RETURN upper('hello') AS v");
        assert!(result.is_ok(), "upper() should work: {:?}", result);
    }

    #[test]
    fn test_ceiling_function_alias() {
        let (_dir, _db, conn) = setup_db();
        // Test standard function calls
        let str_result = conn.query("RETURN lower('HELLO') AS v");
        assert!(str_result.is_ok(), "lower('HELLO') should work: {:?}", str_result);
        let ceil_result = conn.query("RETURN ceil(3) AS v");
        assert!(ceil_result.is_ok(), "ceil() should work: {:?}", ceil_result);
        // Test ceiling alias
        let ceiling_result = conn.query("RETURN ceiling(3) AS v");
        assert!(
            ceiling_result.is_ok(),
            "ceiling() alias should work: {:?}",
            ceiling_result
        );
        // Test upper/lower aliases
        let upper_result = conn.query("RETURN upper('hello') AS v");
        assert!(upper_result.is_ok(), "upper() should work: {:?}", upper_result);
        let ucase_result = conn.query("RETURN ucase('hello') AS v");
        assert!(ucase_result.is_ok(), "ucase() should work: {:?}", ucase_result);
        let lcase_result = conn.query("RETURN lcase('HELLO') AS v");
        assert!(lcase_result.is_ok(), "lcase() should work: {:?}", lcase_result);
    }

    #[test]
    fn test_alter_add_column_with_data() {
        // ALTER TABLE ADD on a table that already has data
        // NOTE: grammar uses ADD <name> <type> (no COLUMN keyword)
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
        )
        .unwrap();

        // Insert some data
        let csv_path = _dir.path().join("people.csv");
        std::fs::write(&csv_path, "name,age\nAlice,30\nBob,25\n").unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY Person FROM '{fp}' (HEADER true)")).unwrap();

        // Add a new column (grammar: ADD <name> <type>, no COLUMN keyword)
        let add_result = exec_ok(&conn, "ALTER TABLE Person ADD email STRING");
        assert!(add_result.is_ok(), "ALTER ADD should succeed: {:?}", add_result);

        // Verify original columns still work
        let names = query_column(&conn, "MATCH (n:Person) RETURN n.name ORDER BY n.name");
        assert_eq!(names.len(), 2, "Expected 2 rows after ALTER ADD");

        // Verify the new column can be accessed
        let result = conn.query("MATCH (n:Person) RETURN n.name").unwrap();
        assert_eq!(result.num_rows(), 2, "Expected 2 rows");
    }

    #[test]
    fn test_alter_rename_column_with_data() {
        // NOTE: grammar uses RENAME <name> TO <newname> (no COLUMN keyword)
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(id INT64, label STRING, PRIMARY KEY (id))").unwrap();

        let csv_path = _dir.path().join("data.csv");
        std::fs::write(&csv_path, "id,label\n1,foo\n2,bar\n").unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{fp}' (HEADER true)")).unwrap();

        // Rename column (grammar: RENAME <name> TO <newname>)
        let rename_result = exec_ok(&conn, "ALTER TABLE T RENAME label TO renamed");
        assert!(
            rename_result.is_ok(),
            "ALTER RENAME should succeed: {:?}",
            rename_result
        );
    }

    #[test]
    fn test_alter_drop_column_with_data() {
        // NOTE: grammar uses DROP <name> (no COLUMN keyword)
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE T(id INT64, label STRING, score DOUBLE, PRIMARY KEY (id))",
        )
        .unwrap();

        let csv_path = _dir.path().join("data.csv");
        std::fs::write(&csv_path, "id,label,score\n1,foo,10.5\n2,bar,20.0\n").unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{fp}' (HEADER true)")).unwrap();

        // Drop a non-PK column (grammar: DROP <name>)
        let drop_result = exec_ok(&conn, "ALTER TABLE T DROP score");
        assert!(drop_result.is_ok(), "ALTER DROP should succeed: {:?}", drop_result);
    }

    #[test]
    fn test_large_dataset_stability() {
        // Insert 100 rows to test dataset stability
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Large(id INT64, value INT64, label STRING, PRIMARY KEY (id))",
        )
        .unwrap();

        // Generate a large CSV with simple integer values
        let csv_path = _dir.path().join("large.csv");
        let mut content = String::from("id,value,label\n");
        for i in 0..100 {
            let val = i * 10;
            content.push_str(&format!("{i},{val},item_{i}\n"));
        }
        std::fs::write(&csv_path, &content).unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");

        let copy_result = exec_ok(&conn, &format!("COPY Large FROM '{fp}' (HEADER true)"));
        assert!(copy_result.is_ok(), "COPY should succeed: {:?}", copy_result);

        // Verify scan
        let scan_result = conn.query("MATCH (n:Large) RETURN n.id ORDER BY n.id").unwrap();
        assert_eq!(scan_result.num_rows(), 100, "Expected 100 rows from scan");
    }

    #[test]
    fn test_delete_with_empty_match() {
        // DELETE with no matching rows should succeed (not error)
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(id INT64, PRIMARY KEY (id))").unwrap();

        let csv_path = _dir.path().join("data.csv");
        std::fs::write(&csv_path, "id\n1\n2\n").unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{fp}' (HEADER true)")).unwrap();

        // DELETE with WHERE that matches nothing
        let result = exec_ok(&conn, "MATCH (n:T) WHERE n.id > 100 DELETE n");
        assert!(result.is_ok(), "DELETE with no matches should succeed");

        // Verify all rows still exist
        let remaining = query_column(&conn, "MATCH (n:T) RETURN n.id ORDER BY n.id");
        assert_eq!(remaining.len(), 2, "All rows should remain after no-op DELETE");
    }

    #[test]
    fn test_set_valid_value() {
        // SET property to a valid value should work
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(id INT64, label STRING, PRIMARY KEY (id))").unwrap();

        let csv_path = _dir.path().join("data.csv");
        std::fs::write(&csv_path, "id,label\n1,original\n").unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{fp}' (HEADER true)")).unwrap();

        // SET with a valid expression
        let set_result = exec_ok(&conn, "MATCH (n:T) WHERE n.id = 1 SET n.label = 'updated'");
        assert!(set_result.is_ok(), "SET should succeed: {:?}", set_result);
    }

    #[test]
    fn test_unwind_basic() {
        // UNWIND with a non-empty list works (grammar requires at least one element)
        let (_dir, _db, conn) = setup_db();
        let result = conn.query("UNWIND [1, 2, 3] AS x RETURN x");
        assert!(result.is_ok(), "UNWIND should succeed: {:?}", result);
        if let Ok(r) = result {
            assert_eq!(r.num_rows(), 3);
        }
    }

    #[test]
    fn test_optional_match_no_match() {
        // OPTIONAL MATCH that finds nothing should produce NULL row
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(id INT64, PRIMARY KEY (id))").unwrap();

        let csv_path = _dir.path().join("data.csv");
        std::fs::write(&csv_path, "id\n1\n2\n").unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{fp}' (HEADER true)")).unwrap();

        // OPTIONAL MATCH that finds nothing should produce a row with NULL fields
        let result = conn
            .query("MATCH (n:T) OPTIONAL MATCH (m:T {id: 999}) RETURN n.id, m.id ORDER BY n.id")
            .unwrap();
        assert_eq!(result.num_rows(), 2, "Expected 2 rows from left side");
    }

    // ==================== Auto-Checkpoint Tests ====================

    /// Create a Database with a specific checkpoint_threshold.
    fn setup_db_with_checkpoint(threshold: i64) -> (Arc<Database>, Connection, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_db");
        let config = SystemConfig {
            checkpoint_threshold: threshold,
            ..SystemConfig::default()
        };
        let database = Arc::new(Database::new(db_path, config).unwrap());
        let conn = Connection::new(&database);
        (database, conn, dir)
    }

    #[test]
    fn test_auto_checkpoint_default_triggers_after_write() {
        let (db, conn, _dir) = setup_db_with_checkpoint(-1);

        exec_ok(&conn, "CREATE NODE TABLE T(id INT64, PRIMARY KEY (id))").unwrap();

        {
            let wal = db.storage_manager.wal().lock().unwrap();
            assert_eq!(wal.len(), 1, "WAL should have Checkpoint marker after DDL+checkpoint");
            assert!(matches!(wal.records()[0], kuzu_storage::wal::WALRecord::Checkpoint));
        }

        let csv_path = _dir.path().join("data.csv");
        std::fs::write(&csv_path, "id\n1\n2\n3\n").unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{fp}' (HEADER true)")).unwrap();

        {
            let wal = db.storage_manager.wal().lock().unwrap();
            assert_eq!(wal.len(), 1, "WAL should have only Checkpoint marker after DML");
            assert!(matches!(wal.records()[0], kuzu_storage::wal::WALRecord::Checkpoint));
        }

        let vals = query_column(&conn, "MATCH (n:T) RETURN n.id");
        assert_eq!(vals.len(), 3, "Data should survive checkpoint");
    }

    #[test]
    fn test_auto_checkpoint_disabled_no_checkpoint() {
        let (db, conn, _dir) = setup_db_with_checkpoint(0);

        exec_ok(&conn, "CREATE NODE TABLE T(id INT64, PRIMARY KEY (id))").unwrap();
        exec_ok(&conn, "CREATE NODE TABLE U(val INT64, PRIMARY KEY (val))").unwrap();

        let csv_path = _dir.path().join("data.csv");
        std::fs::write(&csv_path, "id\n10\n20\n").unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{fp}' (HEADER true)")).unwrap();

        {
            let wal = db.storage_manager.wal().lock().unwrap();
            assert!(
                wal.records()
                    .iter()
                    .all(|r| matches!(r, kuzu_storage::wal::WALRecord::Commit { .. })),
                "WAL should have only Commit records (no Checkpoint)"
            );
            assert_eq!(wal.len(), 3, "WAL has 1 Commit record per write operation");
        }

        let vals = query_column(&conn, "MATCH (n:T) RETURN n.id");
        assert_eq!(vals.len(), 2, "Data should be present even without checkpoint");
    }

    #[test]
    fn test_auto_checkpoint_threshold_respected() {
        let (db, conn, _dir) = setup_db_with_checkpoint(1_000_000);

        exec_ok(&conn, "CREATE NODE TABLE T(id INT64, PRIMARY KEY (id))").unwrap();

        let csv_path = _dir.path().join("data.csv");
        std::fs::write(&csv_path, "id\n100\n200\n").unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{fp}' (HEADER true)")).unwrap();

        let vals = query_column(&conn, "MATCH (n:T) RETURN n.id");
        assert_eq!(vals.len(), 2, "Data should be present");

        {
            let wal = db.storage_manager.wal().lock().unwrap();
            assert_eq!(wal.len(), 2, "WAL has 1 Commit per write operation (no checkpoint)");
        }
    }
}

// =========================================================================
// MERGE Tests
// =========================================================================

#[cfg(test)]
mod merge_tests {
    use crate::connection::Connection;
    use crate::database::{Database, SystemConfig};
    use kuzu_common::types::Value;
    use std::sync::Arc;

    fn setup_db() -> (tempfile::TempDir, Arc<Database>, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_db");
        let config = SystemConfig::default();
        let database = Arc::new(Database::new(db_path, config).unwrap());
        let conn = Connection::new(&database);
        (dir, database, conn)
    }

    fn exec_ok(conn: &Connection, sql: &str) -> Result<String, String> {
        conn.query(sql).map(|r| r.to_string())
    }

    fn query_column(conn: &Connection, sql: &str) -> Vec<Value> {
        let result = conn.query(sql).unwrap();
        result
            .chunks
            .iter()
            .flat_map(|c| (0..c.size).filter_map(|i| c.fields.first().and_then(|f| f.get_value(i))))
            .collect()
    }

    #[test]
    fn test_merge_creates_new_node() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
        )
        .unwrap();

        let result = exec_ok(&conn, "MERGE (n:Person {name: 'Alice', age: 30})");
        assert!(result.is_ok(), "MERGE should succeed: {:?}", result);

        let names = query_column(&conn, "MATCH (n:Person) RETURN n.name");
        assert_eq!(names.len(), 1, "Should have 1 node");
        assert_eq!(names[0], Value::String("Alice".into()));
    }

    #[test]
    fn test_merge_matches_existing_node() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
        )
        .unwrap();

        let csv_path = _dir.path().join("people.csv");
        std::fs::write(&csv_path, "name,age\nAlice,30\n").unwrap();
        let fp = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY Person FROM '{fp}' (HEADER true)")).unwrap();

        let result = exec_ok(&conn, "MERGE (n:Person {name: 'Alice'})");
        assert!(result.is_ok(), "MERGE existing should succeed: {:?}", result);

        let names = query_column(&conn, "MATCH (n:Person) RETURN n.name");
        assert_eq!(names.len(), 1, "Should still have 1 node (no duplicate)");
    }

    #[test]
    fn test_merge_on_create_sets_properties() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
        )
        .unwrap();

        let result = exec_ok(
            &conn,
            "MERGE (n:Person {name: 'Bob', age: 25}) ON CREATE SET n.age = 26",
        );
        assert!(result.is_ok(), "MERGE ON CREATE should succeed: {:?}", result);

        let ages = query_column(&conn, "MATCH (n:Person) RETURN n.age");
        assert_eq!(ages.len(), 1);
    }

    #[test]
    fn test_merge_parse_error_on_bad_syntax() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
        )
        .unwrap();

        let result = exec_ok(&conn, "MERGE");
        assert!(result.is_err(), "MERGE without pattern should fail");
    }

    #[test]
    fn test_merge_into_nonexistent_table() {
        let (_dir, _db, conn) = setup_db();

        let result = exec_ok(&conn, "MERGE (n:NoSuchTable {name: 'x'})");
        assert!(result.is_err(), "MERGE into non-existent table should fail");
    }
}

// =========================================================================
// CALL Tests
// =========================================================================

#[cfg(test)]
mod call_tests {
    use crate::connection::Connection;
    use crate::database::{Database, SystemConfig};
    use std::sync::Arc;

    fn setup_db() -> (tempfile::TempDir, Arc<Database>, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_db");
        let config = SystemConfig::default();
        let database = Arc::new(Database::new(db_path, config).unwrap());
        let conn = Connection::new(&database);
        (dir, database, conn)
    }

    fn exec_ok(conn: &Connection, sql: &str) -> Result<String, String> {
        conn.query(sql).map(|r| r.to_string())
    }

    #[test]
    fn test_call_show_tables_empty() {
        let (_dir, _db, conn) = setup_db();
        let result = exec_ok(&conn, "CALL show_tables()");
        assert!(result.is_ok(), "CALL show_tables() should succeed: {:?}", result);
    }

    #[test]
    fn test_call_show_tables_with_tables() {
        let (_dir, _db, conn) = setup_db();
        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
        )
        .unwrap();
        exec_ok(&conn, "CREATE NODE TABLE City(name STRING, PRIMARY KEY (name))").unwrap();

        let result = exec_ok(&conn, "CALL show_tables()");
        assert!(result.is_ok(), "CALL show_tables() with tables: {:?}", result);
        let out = result.unwrap().to_lowercase();
        assert!(
            out.contains("person") || out.contains("city"),
            "Should list tables: {out}"
        );
    }

    #[test]
    fn test_call_nonexistent_function() {
        let (_dir, _db, conn) = setup_db();
        let result = exec_ok(&conn, "CALL nonexistent_function()");
        assert!(result.is_err(), "Calling non-existent function should fail");
    }

    #[test]
    fn test_call_syntax_no_args() {
        let (_dir, _db, conn) = setup_db();
        let result = exec_ok(&conn, "CALL tables()");
        assert!(result.is_ok(), "CALL tables() should succeed: {:?}", result);
    }
}

// =========================================================================
// CREATE DML Tests — `CREATE (n:Label {props})`
// =========================================================================

#[cfg(test)]
mod create_dml_tests {
    use crate::connection::Connection;
    use crate::database::{Database, SystemConfig};
    use kuzu_common::types::Value;
    use std::sync::Arc;

    fn setup_db() -> (tempfile::TempDir, Arc<Database>, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_db");
        let config = SystemConfig::default();
        let database = Arc::new(Database::new(db_path, config).unwrap());
        let conn = Connection::new(&database);
        (dir, database, conn)
    }

    fn exec_ok(conn: &Connection, sql: &str) -> Result<String, String> {
        conn.query(sql).map(|r| r.to_string())
    }

    fn query_column(conn: &Connection, sql: &str) -> Vec<Value> {
        let result = conn.query(sql).unwrap();
        result
            .chunks
            .iter()
            .flat_map(|c| (0..c.size).filter_map(|i| c.fields.first().and_then(|f| f.get_value(i))))
            .collect()
    }

    #[test]
    fn test_create_dml_basic() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
        )
        .unwrap();

        let result = exec_ok(&conn, "CREATE (n:Person {name: 'Alice', age: 30})");
        assert!(result.is_ok(), "CREATE DML should succeed: {:?}", result);

        let names = query_column(&conn, "MATCH (n:Person) RETURN n.name");
        assert_eq!(names.len(), 1, "Should have 1 node");
        assert_eq!(names[0], Value::String("Alice".into()));
    }

    #[test]
    fn test_create_dml_multiple_nodes() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
        )
        .unwrap();

        exec_ok(&conn, "CREATE (n:Person {name: 'Alice', age: 30})").unwrap();
        exec_ok(&conn, "CREATE (n:Person {name: 'Bob', age: 25})").unwrap();
        exec_ok(&conn, "CREATE (n:Person {name: 'Charlie', age: 35})").unwrap();

        let names = query_column(&conn, "MATCH (n:Person) RETURN n.name ORDER BY n.name");
        assert_eq!(names.len(), 3, "Should have 3 nodes");
        assert_eq!(names[0], Value::String("Alice".into()));
        assert_eq!(names[1], Value::String("Bob".into()));
        assert_eq!(names[2], Value::String("Charlie".into()));
    }

    #[test]
    fn test_create_dml_nonexistent_table() {
        let (_dir, _db, conn) = setup_db();

        let result = exec_ok(&conn, "CREATE (n:NoSuchTable {name: 'x'})");
        assert!(result.is_err(), "CREATE into non-existent table should fail");
    }

    #[test]
    fn test_create_dml_without_variable() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE City(name STRING, PRIMARY KEY (name))").unwrap();

        let result = exec_ok(&conn, "CREATE (:City {name: 'Jakarta'})");
        assert!(result.is_ok(), "CREATE without variable should succeed: {:?}", result);

        let names = query_column(&conn, "MATCH (n:City) RETURN n.name");
        assert_eq!(names.len(), 1, "Should have 1 city");
        assert_eq!(names[0], Value::String("Jakarta".into()));
    }

    #[test]
    fn test_create_dml_verify_via_match() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Product(name STRING, id INT64, price INT64, PRIMARY KEY (name))",
        )
        .unwrap();

        exec_ok(&conn, "CREATE (p:Product {name: 'Laptop', id: 1, price: 999})").unwrap();
        exec_ok(&conn, "CREATE (p:Product {name: 'Mouse', id: 2, price: 25})").unwrap();

        let names = query_column(&conn, "MATCH (p:Product) RETURN p.name");
        assert_eq!(names.len(), 2, "Should have 2 products");
        assert_eq!(names[0], Value::String("Laptop".into()));
        assert_eq!(names[1], Value::String("Mouse".into()));
    }

    #[test]
    fn test_create_dml_empty_properties() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
        )
        .unwrap();

        let result = exec_ok(&conn, "CREATE (n:Person {name: 'NullTest', age: 99})");
        assert!(result.is_ok(), "CREATE with properties should succeed: {:?}", result);

        let names = query_column(&conn, "MATCH (n:Person) RETURN n.name");
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn test_create_dml_duplicate_pk_fails() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
        )
        .unwrap();

        exec_ok(&conn, "CREATE (n:Person {name: 'Alice', age: 30})").unwrap();

        let result = exec_ok(&conn, "CREATE (n:Person {name: 'Alice', age: 40})");
        assert!(result.is_err(), "Duplicate PK should fail");
    }
}

// =========================================================================
// FOREACH Tests
// =========================================================================

#[cfg(test)]
mod foreach_tests {
    use crate::connection::Connection;
    use crate::database::{Database, SystemConfig};
    use std::sync::Arc;

    fn setup_db() -> (tempfile::TempDir, Arc<Database>, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_db");
        let config = SystemConfig::default();
        let database = Arc::new(Database::new(db_path, config).unwrap());
        let conn = Connection::new(&database);
        (dir, database, conn)
    }

    fn exec_ok(conn: &Connection, sql: &str) -> Result<String, String> {
        conn.query(sql).map(|r| r.to_string())
    }

    #[test]
    fn test_foreach_parse_only() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE Num(val INT64, PRIMARY KEY (val))").unwrap();

        let result = exec_ok(&conn, "FOREACH (x IN [1,2,3] | CREATE (n:Num {val: x}))");
        assert!(result.is_ok(), "FOREACH should execute: {:?}", result);

        let r = conn.query("MATCH (n:Num) RETURN n.val ORDER BY n.val").unwrap();
        assert_eq!(r.num_rows(), 3, "FOREACH should create 3 nodes, got {}", r.num_rows());
    }

    #[test]
    fn test_foreach_in_match_context() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
        )
        .unwrap();

        let result = exec_ok(&conn, "MATCH (n:Person) FOREACH (x IN [1] | SET n.age = 99)");
        assert!(result.is_ok(), "FOREACH in MATCH context: {:?}", result);
    }
}

// =========================================================================
// Variable-length Path Tests
// =========================================================================

#[cfg(test)]
mod var_length_path_tests {
    use crate::connection::Connection;
    use crate::database::{Database, SystemConfig};
    use std::sync::Arc;

    fn setup_db() -> (tempfile::TempDir, Arc<Database>, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_db");
        let config = SystemConfig::default();
        let database = Arc::new(Database::new(db_path, config).unwrap());
        let conn = Connection::new(&database);
        (dir, database, conn)
    }

    fn exec_ok(conn: &Connection, sql: &str) -> Result<String, String> {
        conn.query(sql).map(|r| r.to_string())
    }

    #[test]
    fn test_var_length_path_parse() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))").unwrap();
        exec_ok(&conn, "CREATE REL TABLE Knows(FROM Person TO Person, since INT64)").unwrap();

        let result = exec_ok(&conn, "MATCH (a:Person)-[*]->(b:Person) RETURN a.name, b.name");
        assert!(result.is_ok(), "Var-length path (*) should parse: {:?}", result);
    }

    #[test]
    fn test_var_length_path_with_bounds_parse() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))").unwrap();
        exec_ok(&conn, "CREATE REL TABLE Knows(FROM Person TO Person, since INT64)").unwrap();

        let result = exec_ok(&conn, "MATCH (a:Person)-[*1..5]->(b:Person) RETURN a.name");
        assert!(result.is_ok(), "Var-length path with bounds should parse: {:?}", result);
    }
}

// =========================================================================
// Subquery Tests
// =========================================================================

#[cfg(test)]
mod subquery_tests {
    use crate::connection::Connection;
    use crate::database::{Database, SystemConfig};
    use std::sync::Arc;

    #[allow(dead_code)]
    fn setup_db() -> (tempfile::TempDir, Arc<Database>, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_db");
        let config = SystemConfig::default();
        let database = Arc::new(Database::new(db_path, config).unwrap());
        let conn = Connection::new(&database);
        (dir, database, conn)
    }

    #[allow(dead_code)]
    fn exec_ok(conn: &Connection, sql: &str) -> Result<String, String> {
        conn.query(sql).map(|r| r.to_string())
    }

    // Additional subquery tests would go here
}
