//! P73.0 — Reproduce & isolate root cause: `UNWIND $rows AS row CREATE (... row.field ...)`
//! writes 0 rows (P69 partial). This test dumps concrete values at each step so we
//! can isolate WHERE the pipeline drops rows — not guess.

mod common;
use common::*;

fn setup_table(conn: &Connection) {
    exec(conn, "CREATE NODE TABLE T(id INT64, name STRING, PRIMARY KEY (id))");
}

#[test]
fn literal_unwind_create_works() {
    // Control: literal map list UNWIND -> CREATE. Expected to insert 2 nodes.
    let (_db, conn) = setup_db();
    setup_table(&conn);
    exec(
        &conn,
        "UNWIND [{id: 1, name: 'a'}, {id: 2, name: 'b'}] AS row \
         CREATE (n:T {id: row.id, name: row.name})",
    );
    let rows = query_rows(&conn, "MATCH (n:T) RETURN n.id, n.name ORDER BY n.id");
    eprintln!("literal_create rows = {rows:?}");
    assert_eq!(
        rows.len(),
        2,
        "literal UNWIND->CREATE must insert 2 nodes, got: {rows:?}"
    );
}

fn structs_rows() -> Value {
    Value::List(vec![
        Value::Struct(vec![
            ("id".into(), Value::Int64(1)),
            ("name".into(), Value::String("a".into())),
        ]),
        Value::Struct(vec![
            ("id".into(), Value::Int64(2)),
            ("name".into(), Value::String("b".into())),
        ]),
    ])
}

#[test]
fn param_unwind_create_noreturn_roundtrip() {
    // P73: kairos shape — `UNWIND $rows AS row CREATE (... row.id ...)` with a
    // list of structs and NO trailing RETURN. The write persists AND the query
    // result summary reflects the true inserted row count (P73.1 output fix).
    let (_db, conn) = setup_db();
    setup_table(&conn);

    let rows = structs_rows();
    let prepared = conn
        .prepare("UNWIND $rows AS row CREATE (n:T {id: row.id, name: row.name})")
        .unwrap();
    let res = conn.execute(&prepared, vec![("rows", rows)]);
    match res {
        Ok(result) => {
            eprintln!("noreturn summary = {}", result.result_summary());
            assert_eq!(
                result.num_rows,
                2,
                "CREATE (no RETURN) must report 2 rows, summary: {}",
                result.result_summary()
            );
            assert_eq!(result.num_columns, 3, "expected _id + id + name columns");
        }
        Err(e) => panic!("param UNWIND->CREATE (no RETURN) errored: {e}"),
    }
    let readback = query_rows(&conn, "MATCH (n:T) RETURN n.id, n.name ORDER BY n.id");
    eprintln!("noreturn readback = {readback:?}");
    assert_eq!(
        readback.len(),
        2,
        "UNWIND->CREATE must persist 2 nodes, readback: {readback:?}"
    );
}

#[test]
fn param_unwind_create_return_projection() {
    // With a trailing RETURN — the projection must resolve the created node's
    // properties (P73.1): 2 rows with the real id+name, not the old garbage
    // [[Int64(2), Int64(0)]].
    let (_db, conn) = setup_db();
    setup_table(&conn);

    let rows = structs_rows();
    let prepared = conn
        .prepare("UNWIND $rows AS row CREATE (n:T {id: row.id, name: row.name}) RETURN n.id, n.name ORDER BY n.id")
        .unwrap();
    let res = conn.execute(&prepared, vec![("rows", rows)]);
    let out = match res {
        Ok(result) => {
            let out: Vec<Vec<String>> = result
                .chunks
                .iter()
                .flat_map(|c| {
                    c.iter_rows().map(|r| {
                        (0..c.fields.len())
                            .map(|ci| match c.get_value(ci, r) {
                                Some(v) => format!("{v:?}"),
                                None => "null".into(),
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            eprintln!("CREATE RETURN output = {out:?}");
            out
        }
        Err(e) => panic!("param UNWIND->CREATE RETURN errored: {e}"),
    };
    assert_eq!(
        out,
        vec![
            vec!["Int64(1)".to_string(), "String(\"a\")".to_string()],
            vec!["Int64(2)".to_string(), "String(\"b\")".to_string()],
        ],
        "RETURN after CREATE must project the real node id+name, got: {out:?}"
    );
    let readback = query_rows(&conn, "MATCH (n:T) RETURN n.id, n.name ORDER BY n.id");
    eprintln!("CREATE RETURN readback = {readback:?}");
    assert_eq!(
        readback.len(),
        2,
        "UNWIND->CREATE must persist 2 nodes, readback: {readback:?}"
    );
}

fn setup_nodes_and_rel(conn: &Connection) {
    exec(
        conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY (id))",
    );
    exec(
        conn,
        "CREATE REL TABLE R(FROM Memory TO Memory, weight DOUBLE, type STRING)",
    );
    exec(conn, "CREATE (:Memory {id: 1, content: 'a'})");
    exec(conn, "CREATE (:Memory {id: 2, content: 'b'})");
}

fn rel_rows() -> Value {
    Value::List(vec![
        Value::Struct(vec![
            ("src".into(), Value::Int64(1)),
            ("tgt".into(), Value::Int64(2)),
            ("weight".into(), Value::Double(0.5)),
            ("type".into(), Value::String("SUPPORTS".into())),
        ]),
        Value::Struct(vec![
            ("src".into(), Value::Int64(2)),
            ("tgt".into(), Value::Int64(1)),
            ("weight".into(), Value::Double(0.4)),
            ("type".into(), Value::String("BRIDGE".into())),
        ]),
    ])
}

#[test]
fn param_unwind_createrel_batch_persists() {
    // P73.2 candidate: kairos `debug.py`/`synthesis.py` edge-create shape —
    // UNWIND $batch -> MATCH both nodes -> CREATE rel. Does the edge write persist?
    let (_db, conn) = setup_db();
    setup_nodes_and_rel(&conn);

    let rows = rel_rows();
    let q = "UNWIND $rows AS row \
             MATCH (a:Memory {id: row.src}), (b:Memory {id: row.tgt}) \
             CREATE (a)-[r:R {weight: row.weight, type: row.type}]->(b)";
    let prepared = conn.prepare(q).unwrap();
    let res = conn.execute(&prepared, vec![("rows", rows.clone())]);
    match res {
        Ok(result) => eprintln!("createrel summary = {}", result.result_summary()),
        Err(e) => panic!("param UNWIND->CREATE REL errored: {e}"),
    }
    let readback = query_rows(
        &conn,
        "MATCH (a:Memory)-[r:R]->(b:Memory) RETURN r.weight, r.type ORDER BY r.type",
    );
    eprintln!("createrel readback = {readback:?}");
    assert_eq!(
        readback.len(),
        2,
        "UNWIND->CREATE REL must persist 2 edges, got: {readback:?}"
    );

    // With RETURN — capture projection output for the rel path.
    let q = "UNWIND $rows AS row \
             MATCH (a:Memory {id: row.src}), (b:Memory {id: row.tgt}) \
             CREATE (a)-[r:R {weight: row.weight, type: row.type}]->(b) \
             RETURN a.id, b.id, r.weight, r.type";
    let prepared = conn.prepare(q).unwrap();
    let res = conn.execute(&prepared, vec![("rows", rows)]);
    match res {
        Ok(result) => {
            let out: Vec<Vec<String>> = result
                .chunks
                .iter()
                .flat_map(|c| {
                    c.iter_rows().map(|r| {
                        (0..c.fields.len())
                            .map(|ci| match c.get_value(ci, r) {
                                Some(v) => format!("{v:?}"),
                                None => "null".into(),
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            eprintln!("createrel RETURN output = {out:?}");
        }
        Err(e) => panic!("param UNWIND->CREATE REL RETURN errored: {e}"),
    }
}
