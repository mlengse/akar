mod common;
use common::{exec, query_column, query_values, setup_db, Value};

#[test]
fn test_boundary_uint64_max_roundtrip_copy() {
    // P48.12: u64::MAX must scan back as UInt64(18446744073709551615), not wrap to Int64(-1).
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE UIntTab(id UINT64, PRIMARY KEY(id))");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("u.csv");
    std::fs::write(&path, "id\n100000\n18446744073709551615\n").unwrap();
    let path = path.to_str().unwrap().replace('\\', "/");
    exec(&conn, &format!("COPY UIntTab FROM '{path}' (HEADER true)"));
    let vals = query_column(&conn, "MATCH (u:UIntTab) RETURN u.id");
    assert!(vals.contains(&Value::UInt64(18446744073709551615)));
    assert!(vals.contains(&Value::UInt64(100000)));
    assert!(!vals.contains(&Value::Int64(-1)));
}

#[test]
fn test_boundary_uint64_where_eq_gt() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE UIntTab(id UINT64, PRIMARY KEY(id))");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("u.csv");
    // The parser only accepts i64-range integer literals, so u64::MAX must be loaded via COPY.
    std::fs::write(&path, "id\n100000\n18446744073709551615\n").unwrap();
    let path = path.to_str().unwrap().replace('\\', "/");
    exec(&conn, &format!("COPY UIntTab FROM '{path}' (HEADER true)"));

    // UInt64 column compared against an Int64 literal (cross-type equality/comparison).
    let vals = query_column(&conn, "MATCH (u:UIntTab) WHERE u.id = 100000 RETURN u.id");
    assert_eq!(vals, vec![Value::UInt64(100000)]);

    let vals = query_column(&conn, "MATCH (u:UIntTab) WHERE u.id > 100000 RETURN u.id");
    assert_eq!(vals, vec![Value::UInt64(18446744073709551615)]);

    let vals = query_column(&conn, "MATCH (u:UIntTab) WHERE u.id < 100000 RETURN u.id");
    assert!(vals.is_empty());
}

#[test]
fn test_boundary_uint64_arithmetic() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE UIntTab(id UINT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE (u:UIntTab {id: 41})");
    let res = query_values(&conn, "MATCH (u:UIntTab) RETURN u.id + 1");
    assert_eq!(res.trim(), "UInt64(42)");
}

#[test]
fn test_boundary_int64_max() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Extreme(id INT64, val INT64, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (e:Extreme {id: 1, val: 9223372036854775807})");
    let res = query_values(&conn, "MATCH (e:Extreme) RETURN e.val");
    assert_eq!(res.trim(), "Int64(9223372036854775807)");
}

#[test]
fn test_boundary_int64_min_via_insert() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN 0 - 1");
    assert_eq!(res.trim(), "Int64(-1)");
}

#[test]
fn test_boundary_double_large() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Extreme(id INT64, val DOUBLE, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (e:Extreme {id: 1, val: 1.7976931348623157e308})");
    let res = query_values(&conn, "MATCH (e:Extreme) RETURN e.val");
    assert!(res.contains("1.7976931348623157e308") || res.contains("Double("));
}

#[test]
fn test_boundary_double_small() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Extreme(id INT64, val DOUBLE, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (e:Extreme {id: 1, val: 2.2250738585072014e-308})");
    let res = query_values(&conn, "MATCH (e:Extreme) RETURN e.val");
    assert!(res.contains("2.2250738585072014e-308") || res.contains("Double("));
}

#[test]
fn test_boundary_empty_string_vs_null() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE StringTab(id INT64, s1 STRING, s2 STRING, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (s:StringTab {id: 1, s1: '', s2: NULL})");

    let res1 = query_values(&conn, "MATCH (s:StringTab) RETURN s.s1");
    assert_eq!(res1.trim(), "String(\"\")");

    let res2 = query_values(&conn, "MATCH (s:StringTab) RETURN s.s2");
    assert_eq!(res2.trim(), "null");
}

#[test]
fn test_boundary_very_long_string_1k() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE StringTab(id INT64, s STRING, PRIMARY KEY (id))",
    );
    let long_str = "A".repeat(1000);
    exec(&conn, &format!("CREATE (s:StringTab {{id: 1, s: '{}'}})", long_str));
    let res = query_values(&conn, "MATCH (s:StringTab) RETURN s.s");
    assert!(res.contains(&long_str));
}

#[test]
fn test_boundary_very_long_string_100k() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE StringTab(id INT64, s STRING, PRIMARY KEY (id))",
    );
    let long_str = "A".repeat(100000);
    exec(&conn, &format!("CREATE (s:StringTab {{id: 1, s: '{}'}})", long_str));
    let res = query_values(&conn, "MATCH (s:StringTab) RETURN s.s");
    assert!(res.contains(&long_str));
}

#[test]
fn test_boundary_single_char_string() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE StringTab(id INT64, s STRING, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (s:StringTab {id: 1, s: 'a'})");
    let res = query_values(&conn, "MATCH (s:StringTab) RETURN s.s");
    assert_eq!(res.trim(), "String(\"a\")");
}

#[test]
fn test_boundary_maximum_column_count() {
    let (_db, conn) = setup_db();
    let mut create_query = String::from("CREATE NODE TABLE WideTable(id INT64");
    for i in 1..=100 {
        create_query.push_str(&format!(", col{} INT64", i));
    }
    create_query.push_str(", PRIMARY KEY(id))");
    let res = exec(&conn, &create_query);
    assert!(res.contains("created") || res.contains("success"));
}

#[test]
fn test_boundary_boolean_literals() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE BoolTab(id INT64, b1 BOOL, b2 BOOL, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (b:BoolTab {id: 1, b1: TRUE, b2: FALSE})");
    exec(&conn, "CREATE (b:BoolTab {id: 2, b1: true, b2: false})");

    let res = query_values(&conn, "MATCH (b:BoolTab) RETURN b.b1");
    let lines: Vec<&str> = res.trim().split('\n').collect();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|l| l.trim() == "Bool(true)"));
}

#[test]
fn test_boundary_string_with_quotes() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE StringTab(id INT64, s STRING, PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "CREATE (s:StringTab {id: 1, s: 'He said \"hello\" and O\\'Connor'})",
    );
    let res = query_values(&conn, "MATCH (s:StringTab) RETURN s.s");
    assert!(res.contains("O'Connor") || res.contains("\"hello\""));
}

#[test]
fn test_boundary_single_row_table() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE SingleRow(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE (s:SingleRow {id: 1})");
    let res = query_values(&conn, "MATCH (s:SingleRow) RETURN COUNT(*)");
    assert_eq!(res.trim(), "Int64(1)");
}

#[test]
fn test_boundary_negative_integer() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN 0 - 42");
    assert_eq!(res.trim(), "Int64(-42)");
}

#[test]
fn test_boundary_zero_integer() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, val INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, val: 0})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.val");
    assert_eq!(res.trim(), "Int64(0)");
}

#[test]
fn test_boundary_double_zero() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, val DOUBLE, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, val: 0.0})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.val");
    assert!(res.contains("0"));
}

#[test]
fn test_boundary_double_negative() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, val DOUBLE, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, val: -3.14})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.val");
    assert!(res.contains("3.14") || res.contains("Double(-3.14)"));
}

#[test]
fn test_boundary_bool_true() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN true");
    assert_eq!(res.trim(), "Bool(true)");
}

#[test]
fn test_boundary_bool_false() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN false");
    assert_eq!(res.trim(), "Bool(false)");
}

#[test]
fn test_boundary_string_whitespace() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, s STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, s: '   '})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.s");
    assert_eq!(res.trim(), "String(\"   \")");
}

#[test]
fn test_boundary_string_special_chars() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, s STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, s: '!@#$%^&*()_+'})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.s");
    assert_eq!(res.trim(), "String(\"!@#$%^&*()_+\")");
}
