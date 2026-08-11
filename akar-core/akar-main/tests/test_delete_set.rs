use akar_common::types::Value;
use akar_main::{Connection, Database, SystemConfig};
use std::sync::Arc;
use tempfile::tempdir;

#[test]
fn test_delete_and_set() -> Result<(), String> {
    let dir = tempdir().map_err(|e| e.to_string())?;
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).map_err(|e| e.to_string())?);
    let conn = Connection::new(&db);

    conn.query("CREATE NODE TABLE User (id INT64, age INT64, PRIMARY KEY(id))")?;
    conn.query("CREATE REL TABLE Follows (FROM User TO User, dummy INT64)")?;

    conn.query("CREATE (u:User {id: 1, age: 20})")?;
    conn.query("CREATE (u:User {id: 2, age: 25})")?;
    conn.query("MATCH (a:User {id: 1}), (b:User {id: 2}) CREATE (a)-[:Follows]->(b)")?;

    // SET must target the row matched by the predicate (physical row index via
    // the `_id` column), and the column named by the SET target — not column 0.
    let res = conn.query("MATCH (u:User {id: 1}) SET u.age = 30")?;
    let chunk = res.chunks.first().unwrap();
    let val = chunk.get_i64(0, 0).unwrap();
    assert_eq!(val, 1, "SET should update 1 row");

    let after_set = conn.query("MATCH (u:User) RETURN u.id, u.age")?;
    let after_chunk = after_set.chunks.first().unwrap();
    assert_eq!(after_chunk.size, 2);
    let (id0, age0) = (after_chunk.get_value(0, 0), after_chunk.get_value(1, 0));
    let (_id1, age1) = (after_chunk.get_value(0, 1), after_chunk.get_value(1, 1));
    assert!(id0 == Some(Value::Int64(1)) || id0 == Some(Value::Int64(2)));
    if id0 == Some(Value::Int64(1)) {
        assert_eq!(age0, Some(Value::Int64(30)), "row id=1 must have age=30 after SET");
        assert_eq!(age1, Some(Value::Int64(25)), "row id=2 must be untouched by SET");
    } else {
        assert_eq!(age1, Some(Value::Int64(30)), "row id=1 must have age=30 after SET");
        assert_eq!(age0, Some(Value::Int64(25)), "row id=2 must be untouched by SET");
    }

    // DETACH DELETE must remove the matched node (id=1). The delete is a soft
    // delete (row slot nulled + PK removed from the index, P52.16), so:
    //   - the PK lookup must no longer resolve id=1,
    //   - the surviving row id=2 must be intact.
    let res2 = conn.query("MATCH (a:User {id: 1}) DETACH DELETE a")?;
    let chunk2 = res2.chunks.first().unwrap();
    let val2 = chunk2.get_i64(0, 0).unwrap();
    assert_eq!(val2, 1, "DELETE should remove 1 node");

    let gone = conn.query("MATCH (u:User {id: 1}) RETURN u.id")?;
    assert_eq!(
        gone.chunks.first().unwrap().size,
        0,
        "deleted node must not resolve via PK lookup"
    );

    let survivor = conn.query("MATCH (u:User {id: 2}) RETURN u.id, u.age")?;
    let sur_chunk = survivor.chunks.first().unwrap();
    assert_eq!(sur_chunk.size, 1, "id=2 must survive the delete");
    assert_eq!(sur_chunk.get_value(0, 0), Some(Value::Int64(2)));
    assert_eq!(sur_chunk.get_value(1, 0), Some(Value::Int64(25)));

    Ok(())
}
