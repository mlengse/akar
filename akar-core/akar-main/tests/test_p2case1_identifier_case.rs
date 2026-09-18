//! P2-CASE-1 — identifier case-sensitivity pinned by negative tests.
//!
//! Node/rel table names are matched verbatim (case-sensitive) at bind time.
//! These tests pin the exact bind error messages emitted for a wrong-case
//! label, and assert the correctly-cased form keeps working (positive control).

mod common;
use common::{exec, exec_err, query_column, setup_db};

#[test]
fn test_node_table_label_is_case_sensitive() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Memory(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (:Memory {id: 1})");
    exec(&conn, "CREATE (:Memory {id: 2})");

    let rows = query_column(&conn, "MATCH (m:Memory) RETURN m.id");
    assert_eq!(rows.len(), 2, "correct-case label must match");

    let err = exec_err(&conn, "MATCH (m:MEMORY) RETURN m.id");
    assert!(err.contains("Bind error: Table 'MEMORY' not found"), "got: {err}");

    let err = exec_err(&conn, "MATCH (m:memory) RETURN m.id");
    assert!(err.contains("Bind error: Table 'memory' not found"), "got: {err}");
}

#[test]
fn test_rel_table_label_is_case_sensitive() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE A(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE NODE TABLE B(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE REL TABLE Connected(FROM A TO B, since INT64)");
    exec(&conn, "CREATE (:A {id: 1})");
    exec(&conn, "CREATE (:B {id: 1})");
    exec(&conn, "MATCH (a:A), (b:B) CREATE (a)-[:Connected {since: 2020}]->(b)");

    let rows = query_column(&conn, "MATCH (a:A)-[r:Connected]->(b:B) RETURN r.since");
    assert_eq!(rows.len(), 1, "correct-case rel label must match");

    let err = exec_err(&conn, "MATCH (a:A)-[r:CONNECTED]-(b:B) RETURN r.since");
    assert!(
        err.contains("Bind error: Rel table 'CONNECTED' not found"),
        "got: {err}"
    );

    let err = exec_err(&conn, "MATCH (a:A)-[r:connected]-(b:B) RETURN r.since");
    assert!(
        err.contains("Bind error: Rel table 'connected' not found"),
        "got: {err}"
    );
}

#[test]
fn test_node_and_rel_table_case_lookups_are_independent() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Memory(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE REL TABLE Connected(FROM Memory TO Person, since INT64)");
    exec(&conn, "CREATE (:Memory {id: 1})");
    exec(&conn, "CREATE (:Person {id: 1})");
    exec(
        &conn,
        "MATCH (m:Memory), (p:Person) CREATE (m)-[:Connected {since: 2020}]->(p)",
    );

    let err = exec_err(&conn, "MATCH (m:Memory)-[r:CONNECTED]->(p:Person) RETURN r");
    assert!(
        err.contains("Bind error: Rel table 'CONNECTED' not found"),
        "got: {err}"
    );

    let err = exec_err(&conn, "MATCH (m:MEMORY)-[r:Connected]->(p:Person) RETURN r");
    assert!(err.contains("Bind error: Table 'MEMORY' not found"), "got: {err}");
}
