mod common;
use common::{exec, exec_err, query_column, setup_db};

/// P84 — `CREATE TYPE <name> AS <type>` creates a persistent, usable type alias.
#[test]
fn test_create_type_alias_basic() {
    let (db, conn) = setup_db();

    let msg = exec(&conn, "CREATE TYPE Age AS INT64");
    assert!(msg.to_lowercase().contains("created"), "msg: {msg}");

    // The alias must be usable as a column type in CREATE NODE TABLE.
    exec(
        &conn,
        "CREATE NODE TABLE Person(id INT64, age Age, name STRING, PRIMARY KEY (id))",
    );
    // Data round-trips through the aliased column (INT64 storage underneath).
    exec(&conn, "CREATE (:Person {id: 1, age: 25, name: 'alice'})");
    let rows = query_column(&conn, "MATCH (p:Person) RETURN p.age");
    assert_eq!(rows, vec![common::Value::Int64(25)]);

    // The alias is persisted in the catalog backing store.
    let catalog = db.catalog();
    let catalog = catalog.lock().unwrap();
    let alias = catalog.get_type_alias("age").expect("alias stored");
    assert_eq!(alias.name, "Age");
    assert_eq!(alias.type_name, "INT64");
    assert!(catalog.type_aliases().len() == 1, "one alias stored");
}

/// P84 — creating an alias over an existing alias resolves transitively.
#[test]
fn test_create_type_alias_chain() {
    let (_db, conn) = setup_db();

    exec(&conn, "CREATE TYPE Age AS INT64");
    exec(&conn, "CREATE TYPE Years AS Age");
    exec(&conn, "CREATE NODE TABLE T(id INT64, years Years, PRIMARY KEY (id))");
    exec(&conn, "CREATE (:T {id: 1, years: 42})");
    let rows = query_column(&conn, "MATCH (t:T) RETURN t.years");
    assert_eq!(rows, vec![common::Value::Int64(42)]);
}

/// P84 — duplicate alias and unknown base type both error (binder validation).
#[test]
fn test_create_type_alias_errors() {
    let (_db, conn) = setup_db();

    exec(&conn, "CREATE TYPE Age AS INT64");
    let err = exec_err(&conn, "CREATE TYPE Age AS STRING");
    assert!(
        err.to_lowercase().contains("exist") || err.to_lowercase().contains("already"),
        "duplicate alias error, got: {err}"
    );

    let err = exec_err(&conn, "CREATE TYPE NotAType AS DOES_NOT_EXIST");
    assert!(
        err.to_lowercase().contains("unknown type") || err.to_lowercase().contains("does_not_exist"),
        "unknown base type error, got: {err}"
    );
}

/// P84 — `ALTER TABLE ADD COLUMN` accepts an aliased type.
#[test]
fn test_create_type_alias_alter_add_column() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE TYPE Score AS DOUBLE");
    exec(
        &conn,
        "CREATE NODE TABLE Player(id INT64, name STRING, PRIMARY KEY (id))",
    );
    exec(&conn, "ALTER TABLE Player ADD COLUMN score Score");
    exec(&conn, "CREATE (:Player {id: 1, name: 'bob', score: 9.5})");
    let rows = query_column(&conn, "MATCH (p:Player) RETURN p.score");
    assert_eq!(rows, vec![common::Value::Double(9.5)]);
}

/// P84 — `COMMENT ON TABLE` persists the comment in the catalog backing store.
#[test]
fn test_comment_on_table_basic() {
    let (db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY (id))",
    );

    let msg = exec(&conn, "COMMENT ON TABLE Person IS 'People graph node'");
    assert!(msg.to_lowercase().contains("comment"), "msg: {msg}");

    let catalog = db.catalog();
    let catalog = catalog.lock().unwrap();
    let comment = catalog.get_table_comment("Person").cloned();
    assert_eq!(comment.as_deref(), Some("People graph node"));

    // Overwriting a comment replaces it.
    drop(catalog);
    exec(&conn, "COMMENT ON TABLE Person IS 'Updated comment'");
    let catalog = db.catalog();
    let catalog = catalog.lock().unwrap();
    assert_eq!(
        catalog.get_table_comment("person").cloned().as_deref(),
        Some("Updated comment")
    );
}

/// P84 — commenting a non-existent table errors (binder validation).
#[test]
fn test_comment_on_table_not_found() {
    let (_db, conn) = setup_db();
    let err = exec_err(&conn, "COMMENT ON TABLE NonExistent IS 'nope'");
    assert!(
        err.to_lowercase().contains("not found"),
        "comment on missing table, got: {err}"
    );
}

/// P84 — the stored comment is surfaced by `CALL show_tables()`.
#[test]
fn test_comment_surfaces_in_show_tables() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE City(id INT64, name STRING, PRIMARY KEY (id))");
    exec(&conn, "COMMENT ON TABLE City IS 'Cities catalog'");

    let rows = common::query_rows(&conn, "CALL show_tables()");
    let city = rows
        .iter()
        .find(|r| r.first().map(|s| s.to_lowercase().contains("city")).unwrap_or(false))
        .unwrap_or_else(|| panic!("City row missing: {rows:?}"));
    let joined = city.join(",").to_lowercase();
    assert!(
        joined.contains("cities catalog"),
        "comment surfaced in show_tables, row: {joined}"
    );
}
