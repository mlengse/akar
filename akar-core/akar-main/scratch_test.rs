use akar_main::{Connection, Database};
use akar_parser::ast::*;

fn main() {
    let db = Database::new(":memory:");
    let conn = Connection::new(&db);
    
    conn.query("CREATE NODE TABLE User(id INT64, name STRING, PRIMARY KEY(id))").unwrap();
    conn.query("CREATE NODE TABLE Post(id INT64, content STRING, PRIMARY KEY(id))").unwrap();
    conn.query("CREATE REL TABLE Likes(FROM User TO Post)").unwrap();
    
    conn.query("CREATE (u:User {id: 1, name: 'Alice'})").unwrap();
    conn.query("CREATE (u:User {id: 2, name: 'Bob'})").unwrap();
    conn.query("CREATE (p:Post {id: 10, content: 'Hello'})").unwrap();
    conn.query("CREATE (p:Post {id: 20, content: 'World'})").unwrap();
    
    let msg = conn.query("MATCH (u:User {id: 1}), (p:Post {id: 10}) CREATE (u)-[:Likes]->(p)").unwrap();
    println!("CREATE 1: {}", msg);
    let msg = conn.query("MATCH (u:User {id: 2}), (p:Post {id: 20}) CREATE (u)-[:Likes]->(p)").unwrap();
    println!("CREATE 2: {}", msg);
    
    // Query
    let query_str = "MATCH (u:User)-[:Likes]->(p:Post) WHERE u.id = 1 RETURN p.content";
    
    // Plan
    let statements = akar_parser::parse(query_str).unwrap();
    let binder = akar_binder::Binder::new(db.catalog());
    let bound = binder.bind(statements.clone()).unwrap();
    let planner = akar_planner::QueryPlanner::new();
    let plan = planner.plan(bound).unwrap();
    println!("LOGICAL PLAN:\n{:#?}", plan);
    
    let optimizer = akar_optimizer::Optimizer::with_stats(db.stats_store());
    let optimized_plan = optimizer.optimize(plan);
    println!("OPTIMIZED PLAN:\n{:#?}", optimized_plan);
    
    // Execute physical
    let r = conn.query(query_str).unwrap();
    println!("RESULT ROWS: {}", r.num_rows());
    if r.num_rows() > 0 {
        let chunk = &r.chunks[0];
        println!("CHUNK NAMES: {:?}", chunk.field_names);
        let field = &chunk.fields[0];
        println!("FIELD NULL? {}", field.is_null(0));
    }
}
