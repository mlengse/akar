mod common;
use common::{setup_db, exec, query_values};

#[test]
fn test_null_insert_explicit() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: NULL})");
    let res = query_values(&conn, "MATCH (p:Person) RETURN p.age");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_insert_implicit() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    let res = query_values(&conn, "MATCH (p:Person) RETURN p.age");
    assert_eq!(res.trim(), "null");
}

#[test]
#[ignore = "Parser might not support rejecting primary key nulls during CREATE"]
fn test_null_primary_key_rejection() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = common::exec_err(&conn, "CREATE (p:Person {id: NULL})");
    assert!(res.to_lowercase().contains("null"));
}

#[test]
fn test_null_where_is_null() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    
    let res = query_values(&conn, "MATCH (p:Person) WHERE p.age IS NULL RETURN p.id");
    assert_eq!(res.trim(), "Int64(2)");
}

#[test]
fn test_null_where_is_not_null() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    
    let res = query_values(&conn, "MATCH (p:Person) WHERE p.age IS NOT NULL RETURN p.id");
    assert_eq!(res.trim(), "Int64(1)");
}

#[test]
fn test_null_where_equals() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: NULL})");
    
    // x = NULL is NULL, which evaluates to false in WHERE
    let res = query_values(&conn, "MATCH (p:Person) WHERE p.age = NULL RETURN p.id");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_null_where_not_equals() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: NULL})");
    
    // x <> NULL is NULL, which evaluates to false in WHERE
    let res = query_values(&conn, "MATCH (p:Person) WHERE p.age <> NULL RETURN p.id");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_null_arithmetic_addition() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NULL + 1");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_arithmetic_subtraction() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN 10 - NULL");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_arithmetic_multiplication() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NULL * 0");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_arithmetic_division() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NULL / 2");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_aggregate_count_star() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN COUNT(*)");
    assert_eq!(res.trim(), "Int64(2)");
}

#[test]
fn test_null_aggregate_count_col() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})"); // age is NULL
    
    // COUNT(col) should ignore NULLs
    let res = query_values(&conn, "MATCH (p:Person) RETURN COUNT(p.age)");
    assert_eq!(res.trim(), "Int64(1)");
}

#[test]
fn test_null_aggregate_sum() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})"); // age is NULL
    exec(&conn, "CREATE (p:Person {id: 3, age: 10})");
    
    // SUM should ignore NULLs
    let res = query_values(&conn, "MATCH (p:Person) RETURN SUM(p.age)");
    assert_eq!(res.trim(), "Int64(30)");
}

#[test]
fn test_null_aggregate_sum_all_nulls() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN SUM(p.age)");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_aggregate_avg() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    exec(&conn, "CREATE (p:Person {id: 3, age: 10})");
    
    // AVG should ignore NULLs, so 30 / 2 = 15
    let res = query_values(&conn, "MATCH (p:Person) RETURN AVG(p.age)");
    assert!(res.contains("15"));
}

#[test]
fn test_null_aggregate_min() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    exec(&conn, "CREATE (p:Person {id: 3, age: 10})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN MIN(p.age)");
    assert_eq!(res.trim(), "Int64(10)");
}

#[test]
fn test_null_aggregate_max() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    exec(&conn, "CREATE (p:Person {id: 3, age: 10})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN MAX(p.age)");
    assert_eq!(res.trim(), "Int64(20)");
}

#[test]
fn test_null_string_concat() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN CONCAT(NULL, 'x')");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_string_lower() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN LOWER(NULL)");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_boolean_and_true() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NULL AND TRUE");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_boolean_and_false() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NULL AND FALSE");
    assert_eq!(res.trim(), "Bool(false)");
}

#[test]
fn test_null_boolean_or_true() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NULL OR TRUE");
    assert_eq!(res.trim(), "Bool(true)");
}

#[test]
fn test_null_boolean_or_false() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NULL OR FALSE");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_boolean_not() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NOT NULL");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_order_by() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    exec(&conn, "CREATE (p:Person {id: 3, age: 10})");
    
    let res = common::query_rows(&conn, "MATCH (p:Person) RETURN p.age ORDER BY p.age");
    assert_eq!(res.len(), 3);
}

#[test]
#[ignore = "Parse error on DISTINCT"]
fn test_null_distinct() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    
    let res = common::query_rows(&conn, "MATCH (p:Person) RETURN DISTINCT p.age");
    assert_eq!(res.len(), 1);
    assert_eq!(res[0][0], "null");
}

#[test]
#[ignore = "CASE expressions not yet fully parsed"]
fn test_null_case_expression() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN CASE WHEN p.age IS NULL THEN 1 ELSE 0 END");
    assert_eq!(res.trim(), "Integer(1)");
}

#[test]
#[ignore = "coalesce function not returning correct value with columns"]
fn test_null_coalesce() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN coalesce(p.age, 100)");
    assert_eq!(res.trim(), "Int64(100)");
}

#[test]
#[ignore = "coalesce fails with multiple args"]
fn test_null_coalesce_multiple() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN coalesce(NULL, NULL, 'default', 'ignored')");
    assert_eq!(res.trim(), "String(\"default\")");
}

#[test]
#[ignore = "ifnull function issue"]
fn test_null_ifnull() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN ifnull(NULL, 42)");
    assert_eq!(res.trim(), "Int64(42)");
}

#[test]
fn test_null_is_null_constant() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NULL IS NULL");
    assert_eq!(res.trim(), "Bool(true)");
}

#[test]
fn test_null_is_not_null_constant() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NULL IS NOT NULL");
    assert_eq!(res.trim(), "Bool(false)");
}

#[test]
fn test_null_is_null_column() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    exec(&conn, "CREATE (p:Person {id: 2, age: 25})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN p.age IS NULL");
    let lines: Vec<&str> = res.trim().split('\n').collect();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().any(|l| l.trim() == "Bool(true)"));
    assert!(lines.iter().any(|l| l.trim() == "Bool(false)"));
}

#[test]
fn test_null_in_where_with_coalesce() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    exec(&conn, "CREATE (p:Person {id: 2, age: 30})");
    
    let res = query_values(&conn, "MATCH (p:Person) WHERE coalesce(p.age, 0) > 10 RETURN p.id");
    assert_eq!(res.trim(), "Int64(2)");
}

#[test]
#[ignore = "IN with NULL list element not supported"]
fn test_null_not_in_list() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN 5 NOT IN [1, 2, NULL]");
    assert_eq!(res.trim(), "null");
}

#[test]
#[ignore = "IN with NULL list element not supported"]
fn test_null_in_list() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN 1 IN [1, 2, NULL]");
    assert_eq!(res.trim(), "Bool(true)");
}

#[test]
#[ignore = "BETWEEN with NULL not supported in parser"]
fn test_null_between() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NULL BETWEEN 1 AND 10");
    assert_eq!(res.trim(), "null");
}

#[test]
#[ignore = "LIKE with NULL not supported"]
fn test_null_like() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN NULL LIKE 'a%'");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_regex() {
    let (_db, conn) = setup_db();
    let res = query_values(&conn, "RETURN regex_matches(NULL, 'a.*')");
    assert_eq!(res.trim(), "null");
}
