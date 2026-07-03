use kuzu_parser::parse;

fn main() {
    let sql = "MATCH (u:User {id: 1}), (p:Post {id: 10}) CREATE (u)-[:Likes]->(p)";
    let stmt = parse(sql).unwrap();
    println!("{:#?}", stmt);
}
