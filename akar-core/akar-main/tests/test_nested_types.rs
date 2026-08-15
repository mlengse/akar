mod common;
use common::{exec, exec_err, query_values, setup_db};

// P53.12: List/Struct columns and literals now round-trip through the Arrow
// scan/projection path (`arrow_array_from_values`) instead of collapsing to
// NULL. These tests pin the observable `{v:?}` representations.

#[test]
fn test_nested_list_empty() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, lst INT64[], PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, lst: []})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.lst");
    assert_eq!(res.trim(), "List([])");
}

#[test]
fn test_nested_list_of_nulls() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, lst INT64[], PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, lst: [NULL, NULL]})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.lst");
    assert_eq!(res.trim(), "List([Null, Null])");
}

#[test]
fn test_nested_list_nested() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, lst INT64[][], PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, lst: [[1, 2], [3, 4]]})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.lst");
    assert_eq!(
        res.trim(),
        "List([List([Int64(1), Int64(2)]), List([Int64(3), Int64(4)])])"
    );
}

#[test]
fn test_nested_list_empty_nested() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, lst INT64[][], PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, lst: [[]]})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.lst");
    assert_eq!(res.trim(), "List([List([])])");
}

#[test]
fn test_nested_map_empty() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE T(id INT64, mp MAP(STRING, INT64), PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (t:T {id: 1, mp: map([], [])})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.mp");
    // `map` is not a registered scalar function, so the expression folds to null.
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_nested_struct_empty_like() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE T(id INT64, s STRUCT(a INT64, b STRING), PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (t:T {id: 1, s: {a: NULL, b: NULL}})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.s");
    // The struct literal round-trips as a StructArray; the fields are null.
    assert!(res.contains("Struct"), "expected a struct, got: {res}");
    assert!(res.contains("Null"), "expected null fields, got: {res}");
}

#[test]
fn test_nested_deeply_nested_struct() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE T(id INT64, s STRUCT(a STRUCT(b STRUCT(c INT64))), PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (t:T {id: 1, s: {a: {b: {c: 100}}}})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.s");
    // Deeply nested structs round-trip through recursive arrow_array_from_values.
    assert!(res.contains("100"), "expected innermost value, got: {res}");
}

#[test]
fn test_nested_union_type() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE T(id INT64, u UNION(a INT64, b STRING), PRIMARY KEY (id))",
    );
    // union_value with named args not yet supported in parser
    // Just verify table creation works
    let _res = exec(&conn, "CREATE (t:T {id: 1})");
    // No assertion needed; we just verify no panic
}

#[test]
fn test_nested_list_size() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, lst INT64[], PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, lst: [10, 20, 30]})");
    let res = query_values(&conn, "MATCH (t:T) RETURN size(t.lst)");
    assert_eq!(res.trim(), "Int64(3)");
}

#[test]
fn test_nested_list_size_empty() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, lst INT64[], PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, lst: []})");
    let res = query_values(&conn, "MATCH (t:T) RETURN size(t.lst)");
    assert_eq!(res.trim(), "Int64(0)");
}

#[test]
fn test_nested_list_extract_out_of_bounds() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, lst INT64[], PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, lst: [10, 20]})");
    // P53.24: `[10]` is rewritten to `list_extract(t.lst, 11)` (0-based
    // subscript → 1-based list_extract); the index is no longer dropped.
    let err = exec_err(&conn, "MATCH (t:T) RETURN t.lst[10]");
    assert!(err.contains("out of bounds"), "expected OOB error, got: {err}");
}

#[test]
fn test_nested_list_element_access() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, lst INT64[], PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, lst: [10, 20]})");
    // P53.24: `t.lst[1]` → `list_extract(t.lst, 2)` → second element.
    let res = query_values(&conn, "MATCH (t:T) RETURN t.lst[1]");
    assert_eq!(res.trim(), "Int64(20)");
}

#[test]
fn test_nested_list_element_zero_based() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, lst INT64[], PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, lst: [10, 20]})");
    // P53.24: Cypher subscript is 0-based → `[0]` is the first element.
    let res = query_values(&conn, "MATCH (t:T) RETURN t.lst[0]");
    assert_eq!(res.trim(), "Int64(10)");
}

#[test]
fn test_nested_list_concat() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE T(id INT64, lst1 INT64[], lst2 INT64[], PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (t:T {id: 1, lst1: [1, 2], lst2: [3, 4]})");
    let res = query_values(&conn, "MATCH (t:T) RETURN list_concat(t.lst1, t.lst2)");
    assert_eq!(res.trim(), "List([Int64(1), Int64(2), Int64(3), Int64(4)])");
}
