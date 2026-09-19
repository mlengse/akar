//! F12 regression tests — a `CASE` expression in a projection list must be
//! *evaluated* per row, not resolved to an input column by position.
//!
//! Root cause (akar-processor `map_projection.rs`): `projection_needs_
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
//! The fix makes the predicate fail-safe (only `Variable`/`PropertyAccess`/
//! `Star` are column-resolvable), which closes the whole class of bug rather
//! than just the `Case` variant.

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

#[test]
fn computed_aggregate_arguments_are_not_yet_supported() {
    // F13 (known limitation, deliberately pinned -- this is NOT the F12 bug).
    //
    // Aggregate arguments are resolved to *column indices* by
    // `resolve_agg_col_indices`; anything that is not a plain `Variable` /
    // `PropertyAccess` resolves to `None`, the aggregate then contributes no
    // values, and the result is NULL. `SUM(s.bridges)` works, while
    // `SUM(s.bridges * 2)`, `SUM(abs(s.bridges))` and `SUM(CASE ...)` all
    // return NULL. F12 returned a *wrong non-null value*; this is a separate
    // pre-existing gap.
    //
    // Pinned so the limitation is visible in the suite. Rewrite this test (do
    // not delete it) when P126 implements per-row evaluation of computed
    // aggregate arguments.
    let (_db, conn) = setup_phase_table();

    let plain = query_rows(&conn, "MATCH (s:DT) RETURN SUM(s.bridges) AS t");
    assert_eq!(
        plain,
        vec![vec!["Int64(18)".to_string()]],
        "a plain-column aggregate argument must work: {plain:?}"
    );

    for (label, sql) in [
        ("arithmetic", "MATCH (s:DT) RETURN SUM(s.bridges * 2) AS t"),
        ("function call", "MATCH (s:DT) RETURN SUM(abs(s.bridges)) AS t"),
        (
            "case",
            "MATCH (s:DT) RETURN SUM(CASE WHEN s.bridges > 6 THEN s.bridges ELSE 0 END) AS t",
        ),
    ] {
        let rows = query_rows(&conn, sql);
        assert_eq!(
            rows,
            vec![vec!["null".to_string()]],
            "{label} aggregate argument is expected to be unsupported (F13) and yield NULL; \
             if it now returns a number then P126 landed and this test must be rewritten: {rows:?}"
        );
    }
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
