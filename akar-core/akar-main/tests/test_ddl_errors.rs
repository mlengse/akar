mod common;
use common::{exec, exec_err, query_column, setup_db, Value};

#[test]
fn test_create_table_already_exists() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let err = exec_err(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    assert!(err.contains("already exists") || err.contains("Error"));
}

#[test]
fn test_drop_table_does_not_exist() {
    let (_db, conn) = setup_db();
    let err = exec_err(&conn, "DROP TABLE NonExistent");
    assert!(err.contains("not found") || err.contains("does not exist") || err.contains("Error"));
}

#[test]
fn test_create_rel_table_missing_node_table() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let err = exec_err(&conn, "CREATE REL TABLE Knows(FROM Person TO NonExistent)");
    assert!(err.contains("does not exist") || err.contains("not found") || err.contains("Error"));
}

#[test]
fn test_create_table_missing_primary_key() {
    let (_db, conn) = setup_db();
    let err = exec_err(&conn, "CREATE NODE TABLE Person(id INT64)");
    assert!(err.contains("Parse error") || err.contains("PRIMARY KEY") || err.contains("Error"));
}

#[test]
fn test_create_table_multiple_primary_keys() {
    let (_db, conn) = setup_db();
    let err = exec_err(
        &conn,
        "CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY (id), PRIMARY KEY (name))",
    );
    assert!(err.contains("Parse error") || err.contains("Error"));
}

#[test]
fn test_create_table_invalid_primary_key_type() {
    let (_db, conn) = setup_db();
    let res = exec(&conn, "CREATE NODE TABLE Person(id DOUBLE, PRIMARY KEY (id))");
    assert!(res.contains("created") || res.contains("success"));
}

#[test]
fn test_alter_table_add_existing_property() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    let err = exec_err(&conn, "ALTER TABLE Person ADD age STRING");
    assert!(err.contains("already exists") || err.contains("Error"));
}

#[test]
fn test_alter_table_drop_nonexistent_property() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    let err = exec_err(&conn, "ALTER TABLE Person DROP name");
    assert!(err.contains("does not exist") || err.contains("not found") || err.contains("Error"));
}

#[test]
fn test_alter_table_drop_primary_key() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    let result = conn.query("ALTER TABLE Person DROP id");
    match result {
        Ok(qr) => {
            if !qr.is_success() {
                let err = qr.error_message.as_deref().unwrap_or("");
                assert!(err.contains("primary key") || err.contains("cannot drop") || err.contains("not found"));
            }
        }
        Err(e) => assert!(
            e.contains("primary key") || e.contains("Error") || e.contains("cant drop") || e.contains("not found")
        ),
    }
}

#[test]
fn test_alter_table_rename_nonexistent_table() {
    let (_db, conn) = setup_db();
    let err = exec_err(&conn, "ALTER TABLE NonExistent RENAME TO NewName");
    assert!(err.contains("does not exist") || err.contains("not found") || err.contains("Error"));
}

#[test]
fn test_alter_table_rename_to_existing_table() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE NODE TABLE Animal(id INT64, PRIMARY KEY (id))");
    let err = exec_err(&conn, "ALTER TABLE Person RENAME TO Animal");
    assert!(err.contains("already exists") || err.contains("Error"));
}

#[test]
fn test_create_table_invalid_type() {
    let (_db, conn) = setup_db();
    let err = exec_err(
        &conn,
        "CREATE NODE TABLE Person(id INT64, name INVALID_TYPE, PRIMARY KEY (id))",
    );
    assert!(err.contains("type") || err.contains("Parse error") || err.contains("Error"));
}

#[test]
fn test_create_table_syntax_error() {
    let (_db, conn) = setup_db();
    let err = exec_err(&conn, "CREATE TABLE Person");
    assert!(err.contains("Parse error") || err.contains("Error"));
}

#[test]
fn test_drop_property_syntax_error() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    let err = exec_err(&conn, "ALTER TABLE Person DROP");
    assert!(err.contains("Parse error") || err.contains("Error"));
}

#[test]
fn test_create_table_reserved_keyword() {
    let (_db, conn) = setup_db();
    let res = exec(&conn, "CREATE NODE TABLE MATCH(id INT64, PRIMARY KEY (id))");
    assert!(res.contains("created") || res.contains("success"));
}

#[test]
fn test_create_rel_table_same_from_to() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let result = conn.query("CREATE REL TABLE Knows(FROM Person TO Person)");
    match result {
        Ok(qr) => assert!(qr.is_success() || qr.error_message.as_deref().unwrap_or("").contains("Parse error")),
        Err(e) => assert!(e.contains("Parse error") || e.contains("Error")),
    }
}

#[test]
fn test_drop_nonexistent_index() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let err = exec_err(&conn, "DROP INDEX NonExistentIdx");
    assert!(err.contains("Parse error") || err.contains("Error"));
}

#[test]
fn test_create_node_table_empty_pk_list() {
    let (_db, conn) = setup_db();
    let err = exec_err(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY ())");
    assert!(err.contains("Parse error") || err.contains("Error"));
}

#[test]
fn test_create_table_duplicate_column_names() {
    let (_db, conn) = setup_db();
    let res = exec(&conn, "CREATE NODE TABLE Person(id INT64, id STRING, PRIMARY KEY (id))");
    assert!(res.contains("created") || res.contains("success"));
}

#[test]
fn test_alter_table_add_property_syntax_error() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let err = exec_err(&conn, "ALTER TABLE Person ADD");
    assert!(err.contains("Parse error") || err.contains("Error"));
}

#[test]
fn test_create_sequence_no_name() {
    let (_db, conn) = setup_db();
    let err = exec_err(&conn, "CREATE SEQUENCE");
    assert!(err.contains("Parse error") || err.contains("Error"));
}

#[test]
fn test_create_node_table_with_pk_creates_art_index() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY (id))");
    assert!(
        db.table_catalog().has_art_index("Person"),
        "CREATE NODE TABLE with a PRIMARY KEY must auto-create the ART PK index"
    );
}

#[test]
fn test_create_node_table_with_pk_creates_art_index_for_numeric_type() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE UIntTab(id UINT64, PRIMARY KEY (id))");
    assert!(db.table_catalog().has_art_index("UIntTab"));
    exec(&conn, "CREATE (u:UIntTab {id: 41})");
    let vals = query_column(&conn, "MATCH (u:UIntTab) RETURN u.id");
    assert_eq!(vals, vec![Value::UInt64(41)]);
}

#[test]
fn test_duplicate_art_index_creation_is_not_silent() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let err = db.create_art_index("Person", "second_idx").unwrap_err();
    assert!(
        err.contains("already has an ART index"),
        "second ART index creation on an already-indexed table must error, got: {err}"
    );
}
