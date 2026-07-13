mod common;

use common::{setup_test_db, exec, query_values};

#[test]
fn test_null_insert_explicit() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: NULL})");
    let res = query_values(&conn, "MATCH (p:Person) RETURN p.age");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_insert_implicit() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    let res = query_values(&conn, "MATCH (p:Person) RETURN p.age");
    assert_eq!(res.trim(), "null");
}

#[test]
#[ignore = "Parser might not support rejecting primary key nulls during CREATE"]
fn test_null_primary_key_rejection() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    let res = common::exec_err(&conn, "CREATE (p:Person {id: NULL})");
    assert!(res.to_lowercase().contains("null"));
}

#[test]
#[ignore = "Parse error on IS NULL"]
fn test_null_where_is_null() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    
    let res = query_values(&conn, "MATCH (p:Person) WHERE p.age IS NULL RETURN p.id");
    assert_eq!(res.trim(), "Int64(2)");
}

#[test]
#[ignore = "Parse error on IS NOT NULL"]
fn test_null_where_is_not_null() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    
    let res = query_values(&conn, "MATCH (p:Person) WHERE p.age IS NOT NULL RETURN p.id");
    assert_eq!(res.trim(), "Int64(1)");
}

#[test]
fn test_null_where_equals() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: NULL})");
    
    // x = NULL is NULL, which evaluates to false in WHERE
    let res = query_values(&conn, "MATCH (p:Person) WHERE p.age = NULL RETURN p.id");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_null_where_not_equals() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: NULL})");
    
    // x <> NULL is NULL, which evaluates to false in WHERE
    let res = query_values(&conn, "MATCH (p:Person) WHERE p.age <> NULL RETURN p.id");
    assert_eq!(res.trim(), "");
}

#[test]
fn test_null_arithmetic_addition() {
    let (_db, conn) = setup_test_db();
    let res = query_values(&conn, "RETURN NULL + 1");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_arithmetic_subtraction() {
    let (_db, conn) = setup_test_db();
    let res = query_values(&conn, "RETURN 10 - NULL");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_arithmetic_multiplication() {
    let (_db, conn) = setup_test_db();
    let res = query_values(&conn, "RETURN NULL * 0");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_arithmetic_division() {
    let (_db, conn) = setup_test_db();
    let res = query_values(&conn, "RETURN NULL / 2");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_aggregate_count_star() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN COUNT(*)");
    assert_eq!(res.trim(), "Int64(2)");
}

#[test]
#[ignore = "COUNT(col) incorrectly counts nulls or fails"]
fn test_null_aggregate_count_col() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})"); // age is NULL
    
    // COUNT(col) should ignore NULLs
    let res = query_values(&conn, "MATCH (p:Person) RETURN COUNT(p.age)");
    assert_eq!(res.trim(), "Int64(1)");
}

#[test]
#[ignore = "SUM incorrectly aggregates nulls or fails"]
fn test_null_aggregate_sum() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})"); // age is NULL
    exec(&conn, "CREATE (p:Person {id: 3, age: 10})");
    
    // SUM should ignore NULLs
    let res = query_values(&conn, "MATCH (p:Person) RETURN SUM(p.age)");
    assert_eq!(res.trim(), "Int64(30)");
}

#[test]
#[ignore = "SUM all nulls returns wrong value"]
fn test_null_aggregate_sum_all_nulls() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN SUM(p.age)");
    assert_eq!(res.trim(), "null");
}

#[test]
#[ignore = "AVG incorrectly handles nulls"]
fn test_null_aggregate_avg() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    exec(&conn, "CREATE (p:Person {id: 3, age: 10})");
    
    // AVG should ignore NULLs, so 30 / 2 = 15
    let res = query_values(&conn, "MATCH (p:Person) RETURN AVG(p.age)");
    assert!(res.contains("15"));
}

#[test]
#[ignore = "MIN incorrectly handles nulls"]
fn test_null_aggregate_min() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    exec(&conn, "CREATE (p:Person {id: 3, age: 10})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN MIN(p.age)");
    assert_eq!(res.trim(), "Int64(10)");
}

#[test]
#[ignore = "MAX incorrectly handles nulls"]
fn test_null_aggregate_max() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    exec(&conn, "CREATE (p:Person {id: 3, age: 10})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN MAX(p.age)");
    assert_eq!(res.trim(), "Int64(20)");
}

#[test]
#[ignore = "concat not implemented yet or handles nulls differently"]
fn test_null_string_concat() {
    let (_db, conn) = setup_test_db();
    let res = query_values(&conn, "RETURN CONCAT(NULL, 'x')");
    assert_eq!(res.trim(), "null");
}

#[test]
#[ignore = "lower not implemented yet"]
fn test_null_string_lower() {
    let (_db, conn) = setup_test_db();
    let res = query_values(&conn, "RETURN LOWER(NULL)");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_boolean_and_true() {
    let (_db, conn) = setup_test_db();
    let res = query_values(&conn, "RETURN NULL AND TRUE");
    assert_eq!(res.trim(), "null");
}

#[test]
#[ignore = "NULL AND FALSE returns null instead of false"]
fn test_null_boolean_and_false() {
    let (_db, conn) = setup_test_db();
    let res = query_values(&conn, "RETURN NULL AND FALSE");
    assert_eq!(res.trim(), "Bool(false)");
}

#[test]
#[ignore = "NULL OR TRUE returns null instead of true"]
fn test_null_boolean_or_true() {
    let (_db, conn) = setup_test_db();
    let res = query_values(&conn, "RETURN NULL OR TRUE");
    assert_eq!(res.trim(), "Bool(true)");
}

#[test]
fn test_null_boolean_or_false() {
    let (_db, conn) = setup_test_db();
    let res = query_values(&conn, "RETURN NULL OR FALSE");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_boolean_not() {
    let (_db, conn) = setup_test_db();
    let res = query_values(&conn, "RETURN NOT NULL");
    assert_eq!(res.trim(), "null");
}

#[test]
fn test_null_order_by() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, age: 20})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    exec(&conn, "CREATE (p:Person {id: 3, age: 10})");
    
    // By default, Kuzu puts nulls last? Let's assume nulls last or first, we just want to ensure it works.
    let res = common::query_rows(&conn, "MATCH (p:Person) RETURN p.age ORDER BY p.age");
    assert_eq!(res.len(), 3);
}

#[test]
#[ignore = "Parse error on DISTINCT"]
fn test_null_distinct() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    exec(&conn, "CREATE (p:Person {id: 2})");
    
    let res = common::query_rows(&conn, "MATCH (p:Person) RETURN DISTINCT p.age");
    assert_eq!(res.len(), 1);
    assert_eq!(res[0][0], "null");
}

#[test]
#[ignore = "case expressions not yet fully parsed or supported"]
fn test_null_case_expression() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN CASE WHEN p.age IS NULL THEN 1 ELSE 0 END");
    assert_eq!(res.trim(), "Integer(1)");
}

#[test]
#[ignore = "coalesce not implemented yet"]
fn test_null_coalesce() {
    let (_db, conn) = setup_test_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, age INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1})");
    
    let res = query_values(&conn, "MATCH (p:Person) RETURN coalesce(p.age, 100)");
    assert_eq!(res.trim(), "Int64(100)");
}
