//! F12/F13 regression tests — a `CASE` expression in a projection list must be
//! *evaluated* per row, not resolved to an input column by position; and a
//! computed aggregate argument (P126) must be evaluated before aggregation
//! rather than silently aggregating nothing.
//!
//! F12 root cause (akar-processor `map_projection.rs`): `projection_needs_
//! expression_eval` enumerated the computed expression variants and omitted
//! `Expression::Case`. A CASE projection therefore took the plain-column path;
//! `resolve_projection_column_expand` returns `None` for CASE (it resolves only
//! `Variable`/`PropertyAccess`), so the caller fell through to its positional
//! fallback `column_indices = (0..expressions.len())` and silently projected an
//! unrelated input column:
//!
//! ```text
//! RETURN CASE WHEN s.phase = 'rem' THEN s.bridges ELSE 0 END AS a
//!   -> returned s.id      (input column 0)
//! RETURN s.id AS id, CASE ... AS a
//!   -> returned s.phase   (input column 1)
//! SUM(CASE ... ) -> NULL
//! ```
//!
//! The F12 fix makes that predicate fail-safe (only `Variable`/`PropertyAccess`/
//! `Star` are column-resolvable), which closed the whole class of bug rather
//! than just the `Case` variant.
//!
//! F13 root cause (akar-processor `map_aggregate.rs` + `aggregatehashtable.rs`):
//! `resolve_agg_col_indices` maps an aggregate argument to a column index and
//! only understands `Variable`/`PropertyAccess`/`Star`, so `SUM(s.bridges * 2)`,
//! `SUM(abs(x))` and `SUM(CASE ...)` resolved to "no column needed" and returned
//! NULL. The P126 fix pre-evaluates such arguments into synthetic trailing
//! columns and rewrites the argument to reference them.

mod common;
use common::*;

/// Three rows with distinct `id`/`phase`/`bridges` so a positional mix-up is
/// always observable (returning column 0/1 yields `id`/`phase`, never the
/// expected branch value).
fn setup_phase_table() -> (std::sync::Arc<Database>, Connection) {
    let (db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE DT(id INT64, phase STRING, bridges INT64, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (:DT {id: 10, phase: 'rem', bridges: 7})");
    exec(&conn, "CREATE (:DT {id: 20, phase: 'supersedes', bridges: 11})");
    (db, conn)
}

#[test]
fn searched_case_returns_branch_value_not_a_column() {
    // Single-item projection: the pre-fix bug returned `s.id` (input column 0).
    let (_db, conn) = setup_phase_table();
    let rows = query_rows(
        &conn,
        "MATCH (s:DT) RETURN CASE WHEN s.phase = 'rem' THEN s.bridges ELSE 0 END AS a \
         ORDER BY s.id",
    );
    assert_eq!(
        rows,
        vec![vec!["Int64(7)".to_string()], vec!["Int64(0)".to_string()]],
        "searched CASE must yield the THEN/ELSE branch, got: {rows:?}"
    );
}

#[test]
fn case_as_second_projection_item_is_not_positional() {
    // Two-item projection: the pre-fix bug returned `s.phase` (input column 1)
    // for the CASE. Branch values 100/200 are distinguishable from id and phase,
    // so any positional fallback fails this assertion.
    let (_db, conn) = setup_phase_table();
    let rows = query_rows(
        &conn,
        "MATCH (s:DT) RETURN s.id AS id, CASE WHEN s.phase = 'rem' THEN 100 ELSE 200 END AS a \
         ORDER BY s.id",
    );
    assert_eq!(
        rows,
        vec![
            vec!["Int64(10)".to_string(), "Int64(100)".to_string()],
            vec!["Int64(20)".to_string(), "Int64(200)".to_string()],
        ],
        "CASE in a later projection position must still be evaluated, got: {rows:?}"
    );
}

#[test]
fn simple_case_form_is_evaluated() {
    // `CASE <subject> WHEN <value> ...` (simple form) took the same broken path.
    let (_db, conn) = setup_phase_table();
    let rows = query_rows(
        &conn,
        "MATCH (s:DT) RETURN CASE s.phase WHEN 'rem' THEN 1 ELSE 2 END AS a ORDER BY s.id",
    );
    assert_eq!(
        rows,
        vec![vec!["Int64(1)".to_string()], vec!["Int64(2)".to_string()]],
        "simple-form CASE must be evaluated, got: {rows:?}"
    );
}

/// P126 (F13): a computed aggregate argument is evaluated per row and then
/// aggregated, instead of resolving to "no column needed" and yielding NULL.
///
/// Before the fix `SUM(s.bridges)` returned 18 while `SUM(s.bridges * 2)`,
/// `SUM(abs(s.bridges))` and `SUM(CASE ...)` all returned NULL, because
/// `resolve_agg_col_indices` maps arguments to column indices and only
/// understands `Variable`/`PropertyAccess`/`Star`.
///
/// `bridges` is 7 and 11 on the two fixture rows, so the doubled values are 14
/// and 22 and their sum is 36.
#[test]
fn computed_aggregate_arguments_are_evaluated() {
    let (_db, conn) = setup_phase_table();

    // Regression guard: plain-column aggregates must be untouched by the fix.
    let plain = query_rows(&conn, "MATCH (s:DT) RETURN SUM(s.bridges) AS t");
    assert_eq!(
        plain,
        vec![vec!["Int64(18)".to_string()]],
        "a plain-column aggregate argument must keep working: {plain:?}"
    );

    for (label, sql, expected) in [
        ("arithmetic", "MATCH (s:DT) RETURN SUM(s.bridges * 2) AS t", "Int64(36)"),
        (
            "function call",
            "MATCH (s:DT) RETURN SUM(abs(s.bridges)) AS t",
            "Int64(18)",
        ),
        (
            "case",
            "MATCH (s:DT) RETURN SUM(CASE WHEN s.bridges > 6 THEN s.bridges ELSE 0 END) AS t",
            "Int64(18)",
        ),
        ("min", "MATCH (s:DT) RETURN MIN(s.bridges * 2) AS t", "Int64(14)"),
        ("max", "MATCH (s:DT) RETURN MAX(s.bridges * 2) AS t", "Int64(22)"),
    ] {
        let rows = query_rows(&conn, sql);
        assert_eq!(
            rows,
            vec![vec![expected.to_string()]],
            "{label} aggregate argument must be evaluated, got: {rows:?}"
        );
    }
}

/// The aggregate functions that use the same argument-resolution path but are
/// absent from the Arrow scalar fast paths (they go through the per-row
/// `update_states_row` instead) must work too.
#[test]
fn computed_aggregate_arguments_avg_collect_and_distinct() {
    let (_db, conn) = setup_phase_table();

    let avg = query_rows(&conn, "MATCH (s:DT) RETURN AVG(s.bridges * 2) AS t");
    assert_eq!(
        avg,
        vec![vec!["Double(18.0)".to_string()]],
        "AVG over a computed argument, got: {avg:?}"
    );

    let collect = query_rows(&conn, "MATCH (s:DT) RETURN COLLECT(s.bridges * 2) AS t");
    assert_eq!(
        collect,
        vec![vec!["List([Int64(14), Int64(22)])".to_string()]],
        "COLLECT over a computed argument, got: {collect:?}"
    );

    // P88 DISTINCT takes the dedicated accumulator, which resolves the argument
    // through the same column indices.
    let distinct = query_rows(&conn, "MATCH (s:DT) RETURN COUNT(DISTINCT s.bridges * 2) AS t");
    assert_eq!(
        distinct,
        vec![vec!["Int64(2)".to_string()]],
        "COUNT(DISTINCT <computed>) got: {distinct:?}"
    );

    let distinct_collapsed = query_rows(&conn, "MATCH (s:DT) RETURN COUNT(DISTINCT s.bridges - s.bridges) AS t");
    assert_eq!(
        distinct_collapsed,
        vec![vec!["Int64(1)".to_string()]],
        "COUNT(DISTINCT) must still collapse equal values, got: {distinct_collapsed:?}"
    );
}

/// Computed arguments must also work under `GROUP BY`, where the partitioned
/// fast path resolves arguments by column index as well. Plain-column GROUP BY
/// output is asserted alongside it so a regression in the grouped path is
/// visible in the same test.
#[test]
fn computed_aggregate_arguments_under_group_by() {
    let (_db, conn) = setup_phase_table();

    let mut plain = query_rows(&conn, "MATCH (s:DT) RETURN s.phase AS p, SUM(s.bridges) AS t");
    plain.sort();
    assert_eq!(
        plain,
        vec![
            vec!["String(\"rem\")".to_string(), "Int64(7)".to_string()],
            vec!["String(\"supersedes\")".to_string(), "Int64(11)".to_string()],
        ],
        "plain-column GROUP BY changed: {plain:?}"
    );

    let mut computed = query_rows(&conn, "MATCH (s:DT) RETURN s.phase AS p, SUM(s.bridges * 2) AS t");
    computed.sort();
    assert_eq!(
        computed,
        vec![
            vec!["String(\"rem\")".to_string(), "Int64(14)".to_string()],
            vec!["String(\"supersedes\")".to_string(), "Int64(22)".to_string()],
        ],
        "GROUP BY over a computed aggregate argument, got: {computed:?}"
    );
}

#[test]
fn case_in_where_filter_is_evaluated() {
    let (_db, conn) = setup_phase_table();
    let rows = query_rows(
        &conn,
        "MATCH (s:DT) WHERE CASE WHEN s.phase = 'rem' THEN true ELSE false END \
         RETURN s.id AS id",
    );
    assert_eq!(
        rows,
        vec![vec!["Int64(10)".to_string()]],
        "CASE in WHERE must filter by the branch value, got: {rows:?}"
    );
}

#[test]
fn case_with_string_branches_and_alias_preserved() {
    let (_db, conn) = setup_phase_table();
    let rows = query_rows(
        &conn,
        "MATCH (s:DT) RETURN CASE WHEN s.bridges > 10 THEN 'heavy' ELSE 'light' END AS weight \
         ORDER BY s.id",
    );
    assert_eq!(
        rows,
        vec![
            vec!["String(\"light\")".to_string()],
            vec!["String(\"heavy\")".to_string()],
        ],
        "string branches must survive with their alias, got: {rows:?}"
    );
}

/// Guard against over-eager evaluation: expressions that were already correct
/// must keep their results (they exercise the same projection path).
#[test]
fn other_projection_expressions_unchanged() {
    let (_db, conn) = setup_phase_table();

    let arith = query_rows(&conn, "MATCH (s:DT) RETURN s.bridges * 2 AS d ORDER BY s.id");
    assert_eq!(
        arith,
        vec![vec!["Int64(14)".to_string()], vec!["Int64(22)".to_string()]],
        "arithmetic projection changed: {arith:?}"
    );

    let coalesced = query_rows(&conn, "MATCH (s:DT) RETURN COALESCE(s.bridges, 0) AS b ORDER BY s.id");
    assert_eq!(
        coalesced,
        vec![vec!["Int64(7)".to_string()], vec!["Int64(11)".to_string()]],
        "COALESCE projection changed: {coalesced:?}"
    );

    // Plain column projections still take the fast column path and keep names.
    let plain = query_rows(&conn, "MATCH (s:DT) RETURN s.phase AS p ORDER BY s.id");
    assert_eq!(
        plain,
        vec![
            vec!["String(\"rem\")".to_string()],
            vec!["String(\"supersedes\")".to_string()],
        ],
        "plain column projection changed: {plain:?}"
    );
}
