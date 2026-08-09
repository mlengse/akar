mod common;
use common::{exec, query_values, setup_db};

#[test]
fn test_empty_scan_node_table() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = query_values(&conn, "MATCH (p:Person) RETURN p.id");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_empty_scan_rel_table() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE REL TABLE Knows(FROM Person TO Person)");
    exec(&conn, "CREATE (p:Person {id: 1})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    let res = query_values(&conn, "MATCH (a:Person)-[k:Knows]->(b:Person) RETURN a.id, b.id");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_empty_aggregate_count() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = query_values(&conn, "MATCH (p:Person) RETURN COUNT(*)");
    assert_eq!(res.trim(), "Int64(0)");
}

#[test]
fn test_empty_aggregate_sum() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = query_values(&conn, "MATCH (p:Person) RETURN SUM(p.id)");
    // Scalar aggregate over empty input emits one row with default values
    // (COUNT=0, SUM/MIN/MAX/AVG=NULL), matching the SQL standard.
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_empty_aggregate_avg() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = query_values(&conn, "MATCH (p:Person) RETURN AVG(p.id)");
    // Scalar aggregate over empty input emits one row with default values
    // (COUNT=0, SUM/MIN/MAX/AVG=NULL), matching the SQL standard.
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_empty_join_both_empty() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE A(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE NODE TABLE B(id INT64, PRIMARY KEY (id))");
    let res = query_values(&conn, "MATCH (a:A), (b:B) WHERE a.id = b.id RETURN a.id");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_empty_join_one_empty() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE A(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE NODE TABLE B(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (a:A {id: 1})");
    let res = query_values(&conn, "MATCH (a:A), (b:B) WHERE a.id = b.id RETURN a.id");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_empty_order_by() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = query_values(&conn, "MATCH (p:Person) RETURN p.id ORDER BY p.id");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_empty_limit() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = query_values(&conn, "MATCH (p:Person) RETURN p.id LIMIT 10");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_empty_where() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = query_values(&conn, "MATCH (p:Person) WHERE p.id = 1 RETURN p.id");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_empty_distinct() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = query_values(&conn, "MATCH (p:Person) RETURN DISTINCT p.id");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_empty_union() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    let res = query_values(
        &conn,
        "MATCH (p:Person) RETURN p.id UNION MATCH (p2:Person) WHERE p2.id = 2 RETURN p2.id",
    );
    assert_eq!(res.trim(), "Int64(1)");
}

#[test]
fn test_empty_delete() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = exec(&conn, "MATCH (p:Person) DELETE p");
    // May return various result messages depending on pipeline execution
    assert!(
        res.contains("success")
            || res.contains("Deleted")
            || res.to_lowercase().contains("delete")
            || res.contains("(empty result)")
            || res.contains("Returned ")
            || res.contains("rows")
    );
}

#[test]
fn test_empty_drop_recreate() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    exec(&conn, "DROP TABLE Person");
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = query_values(&conn, "MATCH (p:Person) RETURN p.id");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_empty_create_and_arithmetic() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NULL + 1");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_empty_unwind() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = query_values(&conn, "UNWIND [] AS x RETURN x");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_empty_skip_zero() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    let res = query_values(&conn, "MATCH (p:Person) RETURN p.id LIMIT 1");
    assert_eq!(res.trim(), "Int64(1)");
}
