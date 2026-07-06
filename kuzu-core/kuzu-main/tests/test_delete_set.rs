use kuzu_main::{Connection, Database, SystemConfig};
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

    let res = conn.query("MATCH (u:User {id: 1}) SET u.age = 30")?;
    let chunk = res.chunks.first().unwrap();
    let val = chunk.fields[0].get_i64(0).unwrap();
    assert_eq!(val, 1, "SET should update 1 row");

    let res2 = conn.query("MATCH (a:User {id: 1}) DETACH DELETE a")?;
    let chunk2 = res2.chunks.first().unwrap();
    let val2 = chunk2.fields[0].get_i64(0).unwrap();
    assert_eq!(val2, 1, "DELETE should remove 1 node");

    Ok(())
}
