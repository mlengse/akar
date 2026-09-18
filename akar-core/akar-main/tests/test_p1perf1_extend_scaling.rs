//! P1-PERF-1 (F3/F6) — Extend over a large relationship table.
//!
//! Two regressions are guarded here, both seen live on the Sulur `Connected`
//! table (≈25k edges, 737 `Memory` rows with a 384-d embedding):
//!
//! * **F6 — memory blow-up.** `PhysicalExtend::execute` used to wholesale-clone
//!   the rel fwd/rev adjacency maps, every rel property column AND the entire
//!   destination node table (`to_column_major_data`, including large embedding
//!   Lists) on *every* execution. It now borrows the catalog tables, resolves
//!   neighbours through the adjacency index (`scan_adj_list`/`scan_rev_adj_list`),
//!   reads destination columns lazily per emitted row, and honours a
//!   pushed-down `LIMIT` row budget.
//!
//! * **F3 — anchor doesn't help.** `MATCH (a:Memory {id: N})-[r:Connected]->(b)`
//!   was planned as `ScanNode(a) -> Extend -> Filter(a.id = N)`, so the anchor
//!   predicate sat *after* the hop and every node was expanded through the whole
//!   relationship table (`cost ∝ rel size`). `ExtendFilterPushDown` now hoists
//!   source-only predicates above the `Extend`, `FilterPushDown` folds them into
//!   the scan, and the hop only expands the matching source row. The
//!   `extend_counters` instrumentation makes this deterministic: the anchored
//!   hop must receive a handful of input rows, never the whole node table.

mod common;

use akar_processor::{extend_counters, reset_extend_counters};
use common::*;
use std::time::Instant;

const NODES: i64 = 200;

/// Expected edge count for the `b.id > a.id` cross-product seed.
fn expected_edges() -> usize {
    ((NODES - 1) * NODES / 2) as usize
}

fn setup_large(conn: &Connection) {
    exec(
        conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY(id))",
    );
    exec(conn, "CREATE REL TABLE Connected(FROM Memory TO Memory, weight DOUBLE)");

    let payload = "x".repeat(128);
    for id in 0..NODES {
        exec(conn, &format!("CREATE (a:Memory {{id: {id}, content: '{payload}'}})"));
    }

    // ~20k edges in a single statement (cross product with a range filter;
    // 200x200 candidate rows stays under the AKAR_MAX_CROSS_ROWS cap).
    exec(
        conn,
        "MATCH (a:Memory), (b:Memory) \
         WHERE b.id > a.id \
         CREATE (a)-[:Connected {weight: 0.5}]->(b)",
    );
}

#[test]
fn test_extend_large_rel_table_scales_and_prunes() {
    let (_db, conn) = setup_db();
    let t_setup = Instant::now();
    setup_large(&conn);
    eprintln!(
        "[P1PERF1] seed ({} edges) took {:?}",
        expected_edges(),
        t_setup.elapsed()
    );

    // 1. Unanchored count over the whole rel table: correctness + budget.
    let t0 = Instant::now();
    let counted = query_rows(&conn, "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN count(r)");
    let count_elapsed = t0.elapsed();
    assert_eq!(
        counted,
        vec![vec![format!("Int64({})", expected_edges())]],
        "count(r) over the whole rel table"
    );
    eprintln!(
        "[P1PERF1] count(r) over {} edges: {:?}",
        expected_edges(),
        count_elapsed
    );
    assert!(
        count_elapsed.as_millis() < 3_000,
        "count(r) took {:?}, expected well under 3s (no O(rel+node) clone)",
        count_elapsed
    );

    // 2. Anchored 1-hop WITHOUT limit: adjacency lookup, must return exactly
    //    the out-degree of the anchor (id 125 → ids 126..=199).
    let anchor = 125i64;
    let out_degree = (NODES - 1 - anchor) as usize;
    reset_extend_counters();
    let t0 = Instant::now();
    let rows = query_rows(
        &conn,
        &format!("MATCH (a:Memory {{id: {anchor}}})-[r:Connected]->(b:Memory) RETURN b.id"),
    );
    let anchored_elapsed = t0.elapsed();
    let (executions, input_rows) = extend_counters();
    assert_eq!(rows.len(), out_degree, "out-degree of anchor {anchor}");
    eprintln!(
        "[P1PERF1] anchored 1-hop (no limit): {:?}, extend executions={executions} input_rows={input_rows}",
        anchored_elapsed
    );
    // The anchor must filter the scan: the hop sees the single matching source
    // row, not all `NODES`. Before the push-down this was `NODES`.
    assert!(
        input_rows <= 4,
        "anchored hop received {input_rows} input rows — the anchor predicate is not \
         pushed into the scan (expected ≤4, full table = {NODES})"
    );
    assert!(
        anchored_elapsed.as_millis() < 500,
        "anchored 1-hop took {:?}, expected < 500ms",
        anchored_elapsed
    );

    // 3. Anchored 1-hop WITH limit: the pushed-down row budget must cap the
    //    work and return exactly `LIMIT` rows.
    reset_extend_counters();
    let t0 = Instant::now();
    let limited = query_rows(
        &conn,
        &format!("MATCH (a:Memory {{id: {anchor}}})-[r:Connected]->(b:Memory) RETURN b.id LIMIT 5"),
    );
    let limited_elapsed = t0.elapsed();
    let (_, limited_input_rows) = extend_counters();
    assert_eq!(limited.len(), 5, "LIMIT 5 rows");
    eprintln!(
        "[P1PERF1] anchored 1-hop LIMIT 5: {:?}, input_rows={limited_input_rows}",
        limited_elapsed
    );
    assert!(
        limited_input_rows <= 4,
        "anchored + LIMIT hop received {limited_input_rows} input rows (expected ≤4)"
    );
    assert!(
        limited_elapsed.as_millis() < 500,
        "anchored 1-hop + LIMIT took {:?}, expected < 500ms",
        limited_elapsed
    );

    // 4. Reverse direction anchored: uses the reverse adjacency index and must
    //    equally receive only the anchored source row.
    let in_degree = anchor as usize;
    reset_extend_counters();
    let t0 = Instant::now();
    let rev_rows = query_rows(
        &conn,
        &format!("MATCH (a:Memory {{id: {anchor}}})<-[r:Connected]-(b:Memory) RETURN b.id"),
    );
    let rev_elapsed = t0.elapsed();
    let (_, rev_input_rows) = extend_counters();
    assert_eq!(rev_rows.len(), in_degree, "in-degree of anchor {anchor}");
    eprintln!(
        "[P1PERF1] anchored reverse 1-hop: {:?}, input_rows={rev_input_rows}",
        rev_elapsed
    );
    assert!(
        rev_input_rows <= 4,
        "reverse anchored hop received {rev_input_rows} input rows (expected ≤4)"
    );
    assert!(
        rev_elapsed.as_millis() < 500,
        "anchored reverse 1-hop took {:?}, expected < 500ms",
        rev_elapsed
    );
}
