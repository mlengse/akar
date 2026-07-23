use akar_binder::Binder;
use akar_catalog::{Catalog, CatalogColumn};
use akar_common::types::LogicalTypeID;
use akar_parser::parse;
use akar_planner::QueryPlanner;
use std::sync::{Arc, Mutex};

fn main() {
    let mut catalog = Catalog::new();
    catalog.create_node_table(
        "User".into(),
        vec![
            CatalogColumn {
                name: "id".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: true,
                compression: akar_common::enums::CompressionType::Uncompressed,
                default_value: None,
            },
            CatalogColumn {
                name: "name".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: false,
                compression: akar_common::enums::CompressionType::Uncompressed,
                default_value: None,
            },
        ],
    );
    catalog.create_node_table(
        "Post".into(),
        vec![
            CatalogColumn {
                name: "id".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: true,
                compression: akar_common::enums::CompressionType::Uncompressed,
                default_value: None,
            },
            CatalogColumn {
                name: "content".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: false,
                compression: akar_common::enums::CompressionType::Uncompressed,
                default_value: None,
            },
        ],
    );
    catalog.create_rel_table(
        "Likes".into(),
        catalog.get_table_id("User").unwrap(),
        catalog.get_table_id("Post").unwrap(),
        vec![],
    );

    let query = "MATCH (u:User)-[:Likes]->(p:Post) WHERE u.id = 1 RETURN p.content";
    let statements = parse(query).unwrap();
    println!("AST:\n{:#?}", statements);
    let binder = Binder::new(Arc::new(Mutex::new(catalog)));
    let bound = binder.bind(statements.clone()).unwrap();
    let planner = QueryPlanner::new();
    let plan = planner.plan(bound).unwrap();
    println!("PLAN:\n{:#?}", plan);
}
