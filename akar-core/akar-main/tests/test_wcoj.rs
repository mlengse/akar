//! P46 WCOJ integration tests — planner-side enumeration of star/cycle patterns
//! via `LogicalIntersect` (fan-out and triangle queries).

mod common;

use common::*;

/// Sort rows of three i64 values and compare with the expected set.
fn assert_rows(conn: &Connection, sql: &str, expected: &[(i64, i64, i64)]) {
    let rows = query_rows(conn, sql);
    let fmt = |x: i64| format!("{:?}", Value::Int64(x));
    let parse = |s: &str| {
        s.trim_start_matches("Int64(")
            .trim_end_matches(')')
            .parse::<i64>()
            .unwrap_or_else(|_| panic!("row value not an Int64: {s:?}"))
    };
    let mut got: Vec<Vec<String>> = rows
        .iter()
        .map(|r| vec![fmt(parse(&r[0])), fmt(parse(&r[1])), fmt(parse(&r[2]))])
        .collect();
    got.sort_unstable();
    let mut exp: Vec<Vec<String>> = expected
        .iter()
        .map(|(a, b, c)| vec![fmt(*a), fmt(*b), fmt(*c)])
        .collect();
    exp.sort_unstable();
    assert_eq!(got, exp, "query: {sql}");
}

/// Create a small graph:
///   Person nodes 0..3, r1: 0->1, 0->2 ; r2: 0->2 ; r3: 1->2
/// So the only triangle through `a` is (0, 1, 2).
fn setup(conn: &Connection) {
    exec(conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(conn, "CREATE REL TABLE r1(FROM Person TO Person)");
    exec(conn, "CREATE REL TABLE r2(FROM Person TO Person)");
    exec(conn, "CREATE REL TABLE r3(FROM Person TO Person)");
    for i in 0..4 {
        exec(conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    exec(
        conn,
        "MATCH (a:Person {id: 0}), (b:Person {id: 1}) CREATE (a)-[:r1]->(b)",
    );
    exec(
        conn,
        "MATCH (a:Person {id: 0}), (b:Person {id: 2}) CREATE (a)-[:r1]->(b)",
    );
    exec(
        conn,
        "MATCH (a:Person {id: 0}), (b:Person {id: 2}) CREATE (a)-[:r2]->(b)",
    );
    exec(
        conn,
        "MATCH (a:Person {id: 1}), (b:Person {id: 2}) CREATE (a)-[:r3]->(b)",
    );
}

#[test]
fn test_wcoj_fan_out_equals_single_hop() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Fan-out: a->b via r1, a->c via r2.
    let wcoj = "MATCH (a:Person)-[:r1]->(b:Person), (a:Person)-[:r2]->(c:Person) RETURN a.id, b.id, c.id";
    // Reference: enumerate via two separate single-pattern queries.
    let mut expected: Vec<Vec<String>> = Vec::new();
    let r1_rows = query_rows(&conn, "MATCH (a:Person)-[:r1]->(b:Person) RETURN a.id, b.id");
    let r2_rows = query_rows(&conn, "MATCH (a:Person)-[:r2]->(c:Person) RETURN a.id, c.id");
    for r1 in &r1_rows {
        for r2 in &r2_rows {
            if r1[0] == r2[0] {
                expected.push(vec![r1[0].clone(), r1[1].clone(), r2[1].clone()]);
            }
        }
    }
    expected.sort_unstable();
    let mut got = query_rows(&conn, wcoj);
    got.sort_unstable();
    assert_eq!(got, expected, "query: {wcoj}");
}

#[test]
fn test_wcoj_triangle() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let triangle = "MATCH (a:Person)-[:r1]->(b:Person), (a:Person)-[:r2]->(c:Person), (b:Person)-[:r3]->(c:Person) RETURN a.id, b.id, c.id";
    // Only (0, 1, 2) forms a triangle: 0->1 (r1), 0->2 (r2), 1->2 (r3).
    assert_rows(&conn, triangle, &[(0, 1, 2)]);
}

#[test]
fn test_wcoj_explain_shows_intersect() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let result = conn
        .query("EXPLAIN MATCH (a:Person)-[:r1]->(b:Person), (a:Person)-[:r2]->(c:Person) RETURN a.id, b.id, c.id")
        .unwrap();
    assert!(result.is_success(), "EXPLAIN failed: {:?}", result.error_message);

    // The plan text is the single value of the single result row.
    let mut plan = String::new();
    for chunk in &result.chunks {
        if chunk.size > 0 {
            if let Some(Value::String(s)) = chunk.get_value(0, 0) {
                plan = s;
            }
        }
    }
    assert!(plan.contains("Intersect"), "expected Intersect in plan, got:\n{plan}");
}

#[test]
fn test_wcoj_fan_out_semantics_single_shared_node() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Node 3 is connected to nobody: fan-out through a=3 must be empty.
    let sql = "MATCH (a:Person {id: 3})-[:r1]->(b:Person), (a:Person)-[:r2]->(c:Person) RETURN a.id, b.id, c.id";
    let rows = query_rows(&conn, sql);
    assert!(rows.is_empty(), "expected no rows for isolated node, got {rows:?}");
}

#[test]
fn test_comma_pattern_chain_shared_var() {
    // P48.1: comma-separated patterns sharing a bound variable `b` must not
    // re-scan `b` (which previously produced a CrossProduct of two Person scans).
    let (_db, conn) = setup_db();
    setup(&conn);
    // r1: 0->1, 0->2 ; r3: 1->2. Path through a=0: only (0, 1, 2).
    let sql = "MATCH (a:Person {id: 0})-[:r1]->(b:Person), (b:Person)-[:r3]->(c:Person) RETURN a.id, b.id, c.id";
    assert_rows(&conn, sql, &[(0, 1, 2)]);
}

#[test]
fn test_comma_pattern_chain_no_cross_product() {
    // P48.1 plan-level guard: the shared `b` must not be scanned twice.
    let (_db, conn) = setup_db();
    setup(&conn);
    let result = conn
        .query(
            "EXPLAIN MATCH (a:Person {id: 0})-[:r1]->(b:Person), (b:Person)-[:r3]->(c:Person) RETURN a.id, b.id, c.id",
        )
        .unwrap();
    assert!(result.is_success(), "EXPLAIN failed: {:?}", result.error_message);
    let mut plan = String::new();
    for chunk in &result.chunks {
        if chunk.size > 0 {
            if let Some(Value::String(s)) = chunk.get_value(0, 0) {
                plan = s;
            }
        }
    }
    assert!(
        !plan.contains("CrossProduct"),
        "expected no CrossProduct (shared variable re-scanned), got:\n{plan}"
    );
    assert_eq!(
        plan.matches("ScanNode").count(),
        1,
        "expected a single Person scan, got:\n{plan}"
    );
}

#[test]
fn test_wcoj_cross_product_fan_out() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE N(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE s(FROM N TO N)");
    exec(&conn, "CREATE REL TABLE t(FROM N TO N)");
    for i in 0..3 {
        exec(&conn, &format!("CREATE (n:N {{id: {i}}})"));
    }
    // 0 has two s-neighbors (1, 2) and two t-neighbors (2, 1) → 2x2 = 4 rows.
    exec(&conn, "MATCH (a:N {id: 0}), (b:N {id: 1}) CREATE (a)-[:s]->(b)");
    exec(&conn, "MATCH (a:N {id: 0}), (b:N {id: 2}) CREATE (a)-[:s]->(b)");
    exec(&conn, "MATCH (a:N {id: 0}), (b:N {id: 2}) CREATE (a)-[:t]->(b)");
    exec(&conn, "MATCH (a:N {id: 0}), (b:N {id: 1}) CREATE (a)-[:t]->(b)");

    let sql = "MATCH (a:N)-[:s]->(b:N), (a:N)-[:t]->(c:N) RETURN a.id, b.id, c.id";
    let mut rows = query_rows(&conn, sql);
    rows.sort_unstable();
    let fmt = |x: i64| format!("{:?}", Value::Int64(x));
    let mut expected: Vec<Vec<String>> = Vec::new();
    for b in [1, 2] {
        for c in [2, 1] {
            expected.push(vec![fmt(0), fmt(b), fmt(c)]);
        }
    }
    expected.sort_unstable();
    assert_eq!(rows, expected, "cross product of fan-out neighbors expected");
}

/// BUG-A regression: a node-predicate `(a:Person {id: 0})` in MATCH combined
/// with an explicit `WHERE` clause must BOTH apply. The binder turns the
/// node-predicate into an implicit `WHERE` (a.id = 0); the planner previously
/// overwrote `filter_expr` instead of AND-combining, so the node-predicate was
/// dropped and CREATE built edges from every node.
#[test]
fn test_node_predicate_with_where_clause() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE r(FROM Person TO Person)");
    for i in 0..4 {
        exec(&conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    exec(
        &conn,
        "MATCH (a:Person {id: 0}), (b:Person) WHERE b.id > 0 AND b.id <= 2 CREATE (a)-[:r]->(b)",
    );

    let rows = query_rows(&conn, "MATCH (a:Person)-[:r]->(b:Person) RETURN a.id, b.id");
    let mut edges: Vec<(i64, i64)> = rows
        .iter()
        .map(|r| {
            let parse = |s: &str| {
                s.trim_start_matches("Int64(")
                    .trim_end_matches(')')
                    .parse::<i64>()
                    .expect("Int64")
            };
            (parse(&r[0]), parse(&r[1]))
        })
        .collect();
    edges.sort_unstable();
    assert_eq!(
        edges,
        vec![(0, 1), (0, 2)],
        "node-predicate + WHERE must create edges only from node 0 to nodes 1,2"
    );
}

/// BUG-A regression: node-predicate must also survive an explicit WHERE that
/// references the SAME variable (both filters applied together).
#[test]
fn test_node_predicate_with_same_var_where() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE r(FROM Person TO Person)");
    for i in 0..4 {
        exec(&conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    exec(
        &conn,
        "MATCH (a:Person {id: 0}), (b:Person {id: 2}) WHERE a.id >= 0 AND b.id <= 3 CREATE (a)-[:r]->(b)",
    );

    let rows = query_rows(&conn, "MATCH (a:Person)-[:r]->(b:Person) RETURN a.id, b.id");
    let parse = |s: &str| {
        s.trim_start_matches("Int64(")
            .trim_end_matches(')')
            .parse::<i64>()
            .expect("Int64")
    };
    assert_eq!(
        rows.len(),
        1,
        "expected exactly one edge 0->2, got: {:?}",
        rows.iter().map(|r| (parse(&r[0]), parse(&r[1]))).collect::<Vec<_>>()
    );
}

/// BUG-A regression: multiple node-predicates across patterns must all apply
/// even without an explicit WHERE clause (implicit WHEREs combine).
#[test]
fn test_multiple_node_predicates() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE r(FROM Person TO Person)");
    for i in 0..4 {
        exec(&conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    // Two implicit WHEREs (a.id=0, b.id=2) must both apply → only edge 0->2.
    exec(
        &conn,
        "MATCH (a:Person {id: 0}), (b:Person {id: 2}) CREATE (a)-[:r]->(b)",
    );

    let rows = query_rows(&conn, "MATCH (a:Person)-[:r]->(b:Person) RETURN a.id, b.id");
    let parse = |s: &str| {
        s.trim_start_matches("Int64(")
            .trim_end_matches(')')
            .parse::<i64>()
            .expect("Int64")
    };
    assert_eq!(
        rows.len(),
        1,
        "expected exactly one edge 0->2, got: {:?}",
        rows.iter().map(|r| (parse(&r[0]), parse(&r[1]))).collect::<Vec<_>>()
    );
}

/// BUG-B regression: `WHERE a.id = b.id` must filter, not silently pass every
/// row. The optimizer previously dropped equality join-condition filters even
/// when no join existed to consume them.
#[test]
fn test_equality_filter_two_columns() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE r3(FROM Person TO Person)");
    for i in 0..6 {
        exec(&conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)] {
        exec(
            &conn,
            &format!("MATCH (x:Person {{id: {a}}}), (y:Person {{id: {b}}}) CREATE (x)-[:r3]->(y)"),
        );
    }
    let eq = "MATCH (a:Person)-[:r3]->(b:Person) WHERE a.id = b.id RETURN a.id, b.id";
    assert!(
        query_rows(&conn, eq).is_empty(),
        "a.id = b.id must yield 0 rows (no self-edges)"
    );

    let lt = "MATCH (a:Person)-[:r3]->(b:Person) WHERE a.id < b.id RETURN a.id, b.id";
    assert_eq!(query_rows(&conn, lt).len(), 5, "a.id < b.id must yield all 5 edges");

    let gt = "MATCH (a:Person)-[:r3]->(b:Person) WHERE a.id > b.id RETURN a.id, b.id";
    assert!(query_rows(&conn, gt).is_empty(), "a.id > b.id must yield 0 rows");

    let eq0 = "MATCH (a:Person)-[:r3]->(b:Person) WHERE a.id = 0 RETURN a.id, b.id";
    assert_eq!(
        query_rows(&conn, eq0).len(),
        1,
        "a.id = 0 must yield exactly the edge 0->1"
    );
}

/// BUG-B regression: the WCOJ closure filter (`__wcoj_closure_*.id = c.id`)
/// closes the triangle. If the optimizer drops it, the query returns
/// Σ C(outdegree(c),2)-style overcount with invalid (a,b,c) rows.
#[test]
fn test_wcoj_triangle_closure_n6() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE r1(FROM Person TO Person)");
    exec(&conn, "CREATE REL TABLE r2(FROM Person TO Person)");
    exec(&conn, "CREATE REL TABLE r3(FROM Person TO Person)");
    for i in 0..6 {
        exec(&conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    // Forward-complete graph on 6 nodes → every triple a<b<c is a triangle.
    for (rel, lo, hi) in [
        ("r1", 0, 2),
        ("r1", 3, 5),
        ("r2", 0, 2),
        ("r2", 3, 5),
        ("r3", 0, 2),
        ("r3", 3, 5),
    ] {
        exec(
            &conn,
            &format!(
                "MATCH (a:Person), (b:Person) WHERE a.id >= {lo} AND a.id <= {hi} AND b.id > a.id CREATE (a)-[:{rel}]->(b)"
            ),
        );
    }
    let triangle = "MATCH (a:Person)-[:r1]->(b:Person), (a:Person)-[:r2]->(c:Person), (b:Person)-[:r3]->(c:Person) RETURN a.id, b.id, c.id";
    let rows = query_rows(&conn, triangle);
    assert_eq!(rows.len(), 20, "expected C(6,3)=20 triangles, got {rows:?}");
    let distinct: std::collections::HashSet<Vec<String>> = rows.iter().cloned().collect();
    assert_eq!(distinct.len(), 20, "duplicate or invalid triangle rows");
    for r in &rows {
        let parse = |s: &str| {
            s.trim_start_matches("Int64(")
                .trim_end_matches(')')
                .parse::<i64>()
                .expect("Int64")
        };
        let (a, b, c) = (parse(&r[0]), parse(&r[1]), parse(&r[2]));
        assert!(a < b && b < c, "invalid triangle row: {r:?}");
    }
}

/// Regression: relationship properties must be addressable through the rel
/// VARIABLE (`r.relation`), both in RETURN projections and in predicates
/// (WHERE and the pattern property map). The physical extend previously
/// prefixed rel fields with the rel TABLE name, so `r.relation` could not
/// resolve and queries silently returned the scan chunk or zero rows.
#[test]
fn test_rel_property_access_through_rel_var() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE knows(FROM Person TO Person, since INT64)");
    for i in 0..3 {
        exec(&conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    exec(
        &conn,
        "MATCH (a:Person {id: 0}), (b:Person {id: 1}) CREATE (a)-[:knows {since: 2010}]->(b)",
    );
    exec(
        &conn,
        "MATCH (a:Person {id: 1}), (b:Person {id: 2}) CREATE (a)-[:knows {since: 2020}]->(b)",
    );

    // Rel property in RETURN.
    let rows = query_rows(&conn, "MATCH (a:Person)-[r:knows]->(b:Person) RETURN a.id, b.id, r.since");
    let mut got: Vec<(i64, i64, i64)> = rows
        .iter()
        .map(|r| {
            let parse = |s: &str| {
                s.trim_start_matches("Int64(")
                    .trim_end_matches(')')
                    .parse::<i64>()
                    .expect("Int64")
            };
            (parse(&r[0]), parse(&r[1]), parse(&r[2]))
        })
        .collect();
    got.sort_unstable();
    assert_eq!(got, vec![(0, 1, 2010), (1, 2, 2020)], "RETURN r.since must resolve");

    // Rel property in WHERE predicate.
    let rows = query_rows(
        &conn,
        "MATCH (a:Person)-[r:knows]->(b:Person) WHERE r.since = 2020 RETURN a.id, b.id",
    );
    assert_eq!(rows.len(), 1, "WHERE r.since must filter");
    assert!(rows[0][0].contains("Int64(1)") && rows[0][1].contains("Int64(2)"));

    // Rel property in the pattern property map.
    let rows = query_rows(
        &conn,
        "MATCH (a:Person)-[r:knows {since: 2010}]->(b:Person) RETURN a.id, b.id",
    );
    assert_eq!(rows.len(), 1, "pattern property map must filter on r.since");
    assert!(rows[0][0].contains("Int64(0)") && rows[0][1].contains("Int64(1)"));
}
