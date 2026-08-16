//! P53.33 regression tests — IEEE f64 arithmetic exactness & FLOAT column support.
//!
//! The kairos drop-in gap audit expected `0.9 + 0.05 == 0.95`. That expectation
//! is unsatisfiable: IEEE binary64 gives `0.9 + 0.05 == 0.9500000000000001`
//! (Python agrees). This suite pins the engine's arithmetic to IEEE-exact f64
//! (no f32 corruption anywhere in Double math) and covers the two genuine bugs
//! found while auditing:
//! - `DataChunk::get_value` had no `PhysicalTypeID::Float` arm, so FLOAT columns
//!   read back as `None`/null even though the underlying Arrow data was valid.
//! - binary arithmetic/comparison had no `Value::Float` promotion, so
//!   `SET f.x = f.x + 0.05` on a FLOAT column wrote `Value::Null`.

mod common;
use common::*;

fn query_doubles(conn: &Connection, sql: &str) -> Vec<f64> {
    let result = conn.query(sql).unwrap();
    let mut out = Vec::new();
    for chunk in &result.chunks {
        for row in 0..chunk.size {
            for col in 0..chunk.fields.len() {
                if let Some(Value::Double(x)) = chunk.get_value(col, row) {
                    out.push(x);
                }
            }
        }
    }
    out
}

fn prepare_double(conn: &Connection, sql: &str, param: Value) -> Vec<Value> {
    let stmt = conn.prepare(sql).unwrap();
    let r = conn.execute(&stmt, vec![("d", param)]).unwrap();
    let mut out = Vec::new();
    for chunk in &r.chunks {
        for row in 0..chunk.size {
            for col in 0..chunk.fields.len() {
                out.push(chunk.get_value(col, row).unwrap_or(Value::Null));
            }
        }
    }
    out
}

#[test]
fn test_p5333_double_arithmetic_ieee_exact() {
    // Binary64 math must match Rust/Python bit-for-bit — never f32-rounded.
    let (_db, conn) = setup_db();
    assert_eq!(query_doubles(&conn, "RETURN 0.9 + 0.05")[0], 0.9 + 0.05);
    assert_eq!(query_doubles(&conn, "RETURN 0.9 + 0.05 + 0.05")[0], 0.9 + 0.05 + 0.05);
    assert_eq!(query_doubles(&conn, "RETURN 0.9 + 0.05 + 0.05")[0], 1.0);
    assert_eq!(query_doubles(&conn, "RETURN 0.1 + 0.2")[0], 0.1 + 0.2);
    assert_eq!(query_doubles(&conn, "RETURN 0.1 * 0.2")[0], 0.1 * 0.2);
    assert_eq!(query_doubles(&conn, "RETURN 1.0 / 3.0")[0], 1.0 / 3.0);
    assert_eq!(query_doubles(&conn, "RETURN 1.0 - 0.9")[0], 1.0 - 0.9);
    assert_eq!(query_doubles(&conn, "RETURN 0.9 + 1")[0], 1.9);
    assert_eq!(query_doubles(&conn, "RETURN 0.9 + 1.0")[0], 1.9);
    assert_eq!(query_doubles(&conn, "RETURN 3.0 % 2.0")[0], 1.0);
}

#[test]
fn test_p5333_prepared_params_ieee_exact() {
    let (_db, conn) = setup_db();
    let vals = prepare_double(&conn, "RETURN 0.9 + $d", Value::Double(0.05));
    assert_eq!(vals, vec![Value::Double(0.9 + 0.05)]);
    // A Float parameter stays f32-precise: 0.9 + f32(0.05) in f64.
    let vals = prepare_double(&conn, "RETURN 0.9 + $d", Value::Float(0.05));
    let expected = 0.9 + 0.05_f32 as f64;
    assert_eq!(vals, vec![Value::Double(expected)]);
}

#[test]
fn test_p5333_set_accumulation_exact() {
    // COALESCE(weight,0) + $delta twice must accumulate exactly: 0.9 ->
    // 0.9500000000000001 -> 1.0 (the same walk Python f64 takes).
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Chain(id INT64, weight DOUBLE, PRIMARY KEY (id))");
    exec(&conn, "CREATE (:Chain {id: 1, weight: 0.9})");
    let stmt = conn
        .prepare("MATCH (c:Chain {id: 1}) SET c.weight = COALESCE(c.weight, 0.0) + $delta")
        .unwrap();
    conn.execute(&stmt, vec![("delta", Value::Double(0.05))]).unwrap();
    assert_eq!(query_doubles(&conn, "MATCH (c:Chain {id: 1}) RETURN c.weight")[0], 0.9 + 0.05);
    conn.execute(&stmt, vec![("delta", Value::Double(0.05))]).unwrap();
    assert_eq!(query_doubles(&conn, "MATCH (c:Chain {id: 1}) RETURN c.weight")[0], 1.0);
}

#[test]
fn test_p5333_float_column_readback() {
    // A FLOAT column must read back as Value::Float (f32), not null (P53.33:
    // DataChunk::get_value lacked a Float arm).
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE F(id INT64, x FLOAT, PRIMARY KEY (id))");
    exec(&conn, "CREATE (:F {id: 1, x: 0.9})");
    let result = conn.query("MATCH (f:F {id: 1}) RETURN f.x").unwrap();
    let mut vals = Vec::new();
    for chunk in &result.chunks {
        for row in 0..chunk.size {
            vals.push(chunk.get_value(0, row));
        }
    }
    assert_eq!(vals, vec![Some(Value::Float(0.9_f32))], "FLOAT column must not read back as null");
    // Arithmetic on the f32 column promotes to f64 and stays non-null.
    let r = query_doubles(&conn, "MATCH (f:F {id: 1}) RETURN f.x + 0.05");
    assert_eq!(r, vec![0.9_f32 as f64 + 0.05]);
}

#[test]
fn test_p5333_set_float_column_not_null() {
    // SET on a FLOAT column must store the computed value, not null (P53.33:
    // add_values had no Float arm, so the fallback wrote Value::Null).
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE F(id INT64, x FLOAT, PRIMARY KEY (id))");
    exec(&conn, "CREATE (:F {id: 1, x: 0.9})");
    conn.query("MATCH (f:F {id: 1}) SET f.x = f.x + 0.05").unwrap();
    let tc = db.table_catalog();
    let t = tc.get_node_table_by_name("F").unwrap();
    let e1 = 0.9_f32 as f64 + 0.05;
    assert_eq!(t.get_value(0, 1), Some(&Value::Double(e1)));
    // Drop the catalog read guard before the next write: a live dashmap `Ref`
    // on the table's shard self-deadlocks with SET's write lock (P53.14 class).
    drop(t);
    conn.query("MATCH (f:F {id: 1}) SET f.x = f.x + 0.05").unwrap();
    let t = tc.get_node_table_by_name("F").unwrap();
    // FLOAT columns round-trip through f32 on each read: the second SET reads
    // f32(0.9499999761581421) = 0.949999988079071 before adding 0.05.
    let e2 = e1 as f32 as f64 + 0.05;
    assert_eq!(t.get_value(0, 1), Some(&Value::Double(e2)));
}
