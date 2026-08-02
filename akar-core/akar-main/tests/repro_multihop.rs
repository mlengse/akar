//! Temporary repro for P48.1 — same-table multi-hop join cross-product.
mod common;

use common::*;

fn setup(conn: &Connection) {
    exec(conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(conn, "CREATE REL TABLE r1(FROM Person TO Person)");
    exec(conn, "CREATE REL TABLE r3(FROM Person TO Person)");
    for i in 0..12 {
        exec(conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    // r1: 0->1..9 (10 b nodes)
    for i in 1..=9 {
        exec(conn, &format!("MATCH (a:Person {{id: 0}}), (b:Person {{id: {i}}}) CREATE (a)-[:r1]->(b)"));
    }
    // r3: each b -> distinct c
    exec(conn, "MATCH (a:Person {id: 1}), (b:Person {id: 2}) CREATE (a)-[:r3]->(b)");
    exec(conn, "MATCH (a:Person {id: 2}), (b:Person {id: 3}) CREATE (a)-[:r3]->(b)");
    exec(conn, "MATCH (a:Person {id: 3}), (b:Person {id: 4}) CREATE (a)-[:r3]->(b)");
}

#[test]
fn test_chain_single_path() {
    let (_db, conn) = setup_db();
    setup(&conn);
    // Single path chain: (a)-[:r1]->(b)-[:r3]->(c)
    let sql = "MATCH (a:Person {id: 0})-[:r1]->(b:Person)-[:r3]->(c:Person) RETURN a.id, b.id, c.id";
    let rows = query_rows(&conn, sql);
    println!("chain rows: {rows:?}");
    assert_eq!(rows.len(), 3, "expected 3 valid paths, got: {rows:?}");
}

#[test]
fn test_chain_comma_patterns() {
    let (_db, conn) = setup_db();
    setup(&conn);
    // Comma-separated patterns sharing b: (a)-[:r1]->(b), (b)-[:r3]->(c)
    let sql =
        "MATCH (a:Person {id: 0})-[:r1]->(b:Person), (b:Person)-[:r3]->(c:Person) RETURN a.id, b.id, c.id";
    let rows = query_rows(&conn, sql);
    println!("comma rows: {rows:?}");
    // BUG P48.1: cross-product — 27 rows (9 b × 3 c) instead of 3 valid paths.
    assert_eq!(rows.len(), 3, "expected 3 valid paths, got: {rows:?}");
}

#[test]
fn test_chain_explain_comma() {
    let (_db, conn) = setup_db();
    setup(&conn);
    let result = conn.query(
        "EXPLAIN MATCH (a:Person {id: 0})-[:r1]->(b:Person), (b:Person)-[:r3]->(c:Person) RETURN a.id, b.id, c.id",
    );
    println!("EXPLAIN result: {result:?}");
    let rows = query_rows(&conn, "EXPLAIN MATCH (a:Person {id: 0})-[:r1]->(b:Person), (b:Person)-[:r3]->(c:Person) RETURN a.id, b.id, c.id");
    println!("EXPLAIN rows: {rows:?}");
}

#[test]
fn test_same_rel_reused() {
    let (_db, conn) = setup_db();
    setup(&conn);
    // Two-hop using same rel table r1: (a)-[:r1]->(b)-[:r1]->(c)
    let sql = "MATCH (a:Person {id: 0})-[:r1]->(b:Person)-[:r1]->(c:Person) RETURN a.id, b.id, c.id";
    let rows = query_rows(&conn, sql);
    println!("same-rel rows: {rows:?}");
    // b in 1..9; c = b+1 via r1? No — r1 only from 0. So no c via r1 from b.
    assert_eq!(rows.len(), 0, "expected 0 rows (no r1 from b), got: {rows:?}");
}

#[test]
fn test_rel_copy_csv() {
    use std::fs::File;
    use std::io::Write;
    // P48.2 repro: COPY into a rel table (no user props) from a CSV with src,dst columns.
    let dir = tempfile::tempdir().unwrap();
    let csv_path = dir.path().join("rels.csv");
    {
        let mut f = File::create(&csv_path).unwrap();
        writeln!(f, "0,1").unwrap();
        writeln!(f, "1,2").unwrap();
        writeln!(f, "2,3").unwrap();
    }
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE r1(FROM Person TO Person)");
    for i in 0..4 {
        exec(&conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    let sql = format!(
        "COPY r1 FROM '{}' (HEADER false)",
        csv_path.display().to_string().replace('\\', "/")
    );
    let result = conn.query(&sql);
    match result {
        Ok(r) if r.is_success() => {
            println!("rel COPY succeeded: {}", r.result_summary());
        }
        Ok(r) => {
            panic!(
                "rel COPY returned error result: {:?}",
                r.error_message.unwrap_or_default()
            );
        }
        Err(e) => {
            println!("rel COPY failed with error: {e}");
            // Assert on the actual observed behavior (cross-product is the bug under audit).
            assert!(
                e.contains("Column count mismatch") || e.contains("expected 0 columns"),
                "unexpected error: {e}"
            );
        }
    }
    // Verify the 3 edges actually loaded (P48.2: rel COPY must insert edges).
    let rows = query_rows(&conn, "MATCH (a:Person)-[:r1]->(b:Person) RETURN COUNT(*)");
    println!("r1 edge count after COPY: {rows:?}");
    assert_eq!(rows[0][0], "Int64(3)", "expected 3 edges loaded, got: {rows:?}");
}

#[test]
fn test_rel_copy_csv_has_props() {
    use std::fs::File;
    use std::io::Write;
    // Same as above but with one user property column — mimics migrate's rel parquet layout.
    let dir = tempfile::tempdir().unwrap();
    let csv_path = dir.path().join("rels.csv");
    {
        let mut f = File::create(&csv_path).unwrap();
        writeln!(f, "0,1,5").unwrap();
        writeln!(f, "1,2,6").unwrap();
        writeln!(f, "2,3,7").unwrap();
    }
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE r1(FROM Person TO Person, since INT64)");
    for i in 0..4 {
        exec(&conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    let sql = format!(
        "COPY r1 FROM '{}' (HEADER false)",
        csv_path.display().to_string().replace('\\', "/")
    );
    let result = conn.query(&sql);
    match result {
        Ok(r) if r.is_success() => {
            println!("rel COPY (with props) succeeded: {}", r.result_summary());
        }
        Ok(r) => {
            panic!(
                "rel COPY returned error result: {:?}",
                r.error_message.unwrap_or_default()
            );
        }
        Err(e) => {
            println!("rel COPY (with props) failed with error: {e}");
            assert!(
                e.contains("Column count mismatch") || e.contains("expected 0 columns"),
                "unexpected error: {e}"
            );
        }
    }
    // Verify the 3 edges + property values actually loaded.
    let rows = query_rows(&conn, "MATCH (a:Person)-[:r1]->(b:Person) RETURN COUNT(*)");
    println!("r1 edge count after COPY: {rows:?}");
    assert_eq!(rows[0][0], "Int64(3)", "expected 3 edges loaded, got: {rows:?}");
}

#[test]
fn test_date_scan_null() {
    // H1 repro: DATE column should survive scan, not come back as NULL.
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Event(id INT64, when DATE, PRIMARY KEY(id))");
    exec(&conn, "CREATE (e:Event {id: 1, when: DATE('2024-01-15')})");
    let rows = query_rows(&conn, "MATCH (e:Event) RETURN e.when");
    println!("date rows: {rows:?}");
    // DATE('2024-01-15') should scan back as a non-null value.
    assert!(
        rows.iter().all(|r| r[0] != "null"),
        "expected non-null DATE value, got: {rows:?}"
    );
}

#[test]
fn test_pred_pushdown_multi_scan_timing() {
    // P48.3 empirical: multi-scan MATCH+WHERE should push predicate to scan.
    // Without pushdown the b-scan is fully materialized before the cross product.
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    let n = 1500;
    for i in 0..n {
        exec(&conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    // Warm up / verify counts.
    let rows = query_rows(&conn, "MATCH (a:Person) WHERE a.id >= 0 AND a.id <= 100 RETURN COUNT(*)");
    assert_eq!(rows[0][0], "Int64(101)", "unexpected single-scan count: {rows:?}");
    let rows = query_rows(
        &conn,
        "MATCH (a:Person), (b:Person) WHERE b.id >= 0 AND b.id <= 100 RETURN COUNT(*)",
    );
    // Correctness is preserved regardless of pushdown; this asserts the filter applies.
    assert_eq!(rows[0][0], "Int64(151500)", "unexpected multi-scan count: {rows:?}");

    // Timing: single-scan (filter adjacent to scan -> FoldedPushDown) vs
    // multi-scan (filter above cross product -> NOT pushed).
    let single_start = std::time::Instant::now();
    let _ = conn.query("MATCH (a:Person) WHERE a.id >= 0 AND a.id <= 100 RETURN a.id");
    let single_elapsed = single_start.elapsed();

    let multi_start = std::time::Instant::now();
    let _ = conn.query(
        "MATCH (a:Person), (b:Person) WHERE b.id >= 0 AND b.id <= 100 RETURN a.id, b.id",
    );
    let multi_elapsed = multi_start.elapsed();

    println!("P48.3 timing: single-scan+WHERE = {single_elapsed:?}, multi-scan+WHERE = {multi_elapsed:?}");
    println!(
        "ratio multi/single = {:.1}x",
        multi_elapsed.as_secs_f64() / single_elapsed.as_secs_f64().max(1e-9)
    );
    // No hard assertion on wall-clock (flaky); presence of pushdown is a plan-level concern.
}

#[test]
fn test_count_variable() {
    // COUNT(<variable>) should count non-null rows, like COUNT(*).
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    for i in 0..5 {
        exec(&conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    let rows = query_rows(&conn, "MATCH (a:Person) RETURN COUNT(*)");
    println!("COUNT(*) = {rows:?}");
    assert_eq!(rows[0][0], "Int64(5)", "unexpected COUNT(*): {rows:?}");
    let rows = query_rows(&conn, "MATCH (a:Person) RETURN COUNT(a)");
    println!("COUNT(a) = {rows:?}");
    // BUG? COUNT(variable) returns 0 instead of 5.
    assert_eq!(rows[0][0], "Int64(5)", "unexpected COUNT(a): {rows:?}");
}

#[test]
fn test_timestamp_scan_null() {
    // H1 variant: TIMESTAMP column.
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Event(id INT64, ts TIMESTAMP, PRIMARY KEY(id))");
    exec(&conn, "CREATE (e:Event {id: 1, ts: TIMESTAMP('2024-01-15 10:00:00')})");
    let rows = query_rows(&conn, "MATCH (e:Event) RETURN e.ts");
    println!("timestamp rows: {rows:?}");
    assert!(
        rows.iter().all(|r| r[0] != "null"),
        "expected non-null TIMESTAMP value, got: {rows:?}"
    );
}
