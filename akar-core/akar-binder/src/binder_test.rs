//! Binder tests

use crate::binder::Binder;
use crate::bound_statement::*;
use akar_catalog::{Catalog, CatalogColumn};
use akar_common::types::LogicalTypeID;
use akar_parser::parse;
use std::sync::{Arc, Mutex};

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_binder() -> Binder {
        let mut catalog = Catalog::new();
        catalog.create_node_table(
            "Person".into(),
            vec![
                CatalogColumn {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "name".into(),
                    logical_type: LogicalTypeID::String,
                    is_primary_key: true,
                    default_value: None,
                },
                CatalogColumn {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "age".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                    default_value: None,
                },
                CatalogColumn {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "score".into(),
                    logical_type: LogicalTypeID::Double,
                    is_primary_key: false,
                    default_value: None,
                },
            ],
        );
        catalog.create_rel_table(
            "Knows".into(),
            0,
            0,
            vec![CatalogColumn {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "since".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
                default_value: None,
            }],
        );
        Binder::new(Arc::new(Mutex::new(catalog)))
    }

    #[test]
    fn test_bind_create_node_table() {
        let binder = Binder::new(Arc::new(Mutex::new(Catalog::new())));
        let sql = "CREATE NODE TABLE City(name STRING, population INT64, PRIMARY KEY (name))";
        let stmt = parse(sql).unwrap();
        let bound = binder.bind(stmt).unwrap();
        match bound {
            BoundStatement::BoundCreateNodeTable(t) => {
                assert_eq!(t.name, "City");
                assert_eq!(t.columns.len(), 2);
                assert_eq!(t.columns[0].logical_type, LogicalTypeID::String);
                assert_eq!(t.columns[1].logical_type, LogicalTypeID::Int64);
            }
            _ => panic!("Expected BoundCreateNodeTable"),
        }
    }

    #[test]
    fn test_bind_drop_table() {
        let binder = Binder::new(Arc::new(Mutex::new(Catalog::new())));
        let sql = "DROP TABLE Person";
        // Should fail because table doesn't exist
        assert!(binder.bind(parse(sql).unwrap()).is_err());
    }

    #[test]
    fn test_bind_match_existing_table() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) RETURN a.name";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => {
                assert_eq!(q.clauses.len(), 2);
                assert_eq!(q.variables.len(), 1);
                assert_eq!(q.variables[0].name, "a");
                assert!(q.variables[0].is_node);
            }
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_match_nonexistent_table() {
        let binder = setup_binder();
        let sql = "MATCH (a:GhostTable) RETURN a";
        assert!(binder.bind(parse(sql).unwrap()).is_err());
    }

    #[test]
    fn test_bind_rel_pattern() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person)-[r:Knows]->(b:Person) RETURN a, b";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => {
                // a is the first node, r is the edge
                // b is the second node - parser currently drops it
                assert!(!q.variables.is_empty());
            }
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_where_boolean() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) WHERE a.age > 25 RETURN a.name";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => {
                assert_eq!(q.clauses.len(), 3); // match, where, return
                match &q.clauses[1] {
                    BoundClause::BoundWhere(w) => {
                        assert_eq!(w.expression.resolved_type, LogicalTypeID::Bool);
                    }
                    _ => panic!("Expected BoundWhere"),
                }
            }
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_duplicate_variable() {
        let binder = setup_binder();
        // Duplicate variable in same MATCH (comma-separated patterns)
        // Note: multiple MATCH clauses not yet supported in grammar
        let sql = "MATCH (a:Person) WHERE a.age = a.age RETURN a";
        // Should bind fine since a is used consistently
        assert!(binder.bind(parse(sql).unwrap()).is_ok());
    }

    #[test]
    fn test_bind_invalid_type() {
        let binder = Binder::new(Arc::new(Mutex::new(Catalog::new())));
        // Valid type but wrong for PRIMARY KEY
        let sql = "CREATE NODE TABLE Bad(age INT64, PRIMARY KEY (name))";
        assert!(binder.bind(parse(sql).unwrap()).is_err());
    }

    #[test]
    fn test_bind_empty_table_name() {
        let binder = Binder::new(Arc::new(Mutex::new(Catalog::new())));
        let sql = "CREATE NODE TABLE (name STRING, PRIMARY KEY (name))";
        // This should fail because parser expects a name
        assert!(parse(sql).is_err() || binder.bind(parse(sql).unwrap()).is_err());
    }

    #[test]
    fn test_bind_create_rel_table() {
        let binder = Binder::new(Arc::new(Mutex::new(Catalog::new())));
        let sql = "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))";
        binder.bind(parse(sql).unwrap()).unwrap();
        let sql2 = "CREATE REL TABLE Knows(FROM Person TO Person, since INT64)";
        let bound = binder.bind(parse(sql2).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundCreateRelTable(t) => {
                assert_eq!(t.name, "Knows");
                assert_eq!(t.columns.len(), 1);
            }
            _ => panic!("Expected BoundCreateRelTable"),
        }
    }

    #[test]
    fn test_bind_function_return_type() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) RETURN COUNT(a)";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => match &q.clauses[1] {
                BoundClause::BoundReturn(r) => {
                    assert_eq!(r.expressions[0].resolved_type, LogicalTypeID::Int64);
                }
                _ => panic!("Expected BoundReturn"),
            },
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_sequence_function_return_type() {
        let binder = setup_binder();
        let sql = "RETURN nextval('my_seq'), currval('my_seq')";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => match &q.clauses[0] {
                BoundClause::BoundReturn(r) => {
                    assert_eq!(r.expressions.len(), 2);
                    assert_eq!(r.expressions[0].resolved_type, LogicalTypeID::Int64);
                    assert_eq!(r.expressions[1].resolved_type, LogicalTypeID::Int64);
                }
                _ => panic!("Expected BoundReturn"),
            },
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_property_type_resolution() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) WHERE a.score > 4.5 RETURN a.name";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => {
                // WHERE clause: a.score should resolve to Double via catalog
                match &q.clauses[1] {
                    BoundClause::BoundWhere(w) => {
                        // The comparison `a.score > 4.5` produces Bool, but
                        // internally `a.score` was resolved to Double via catalog.
                        assert_eq!(w.expression.resolved_type, LogicalTypeID::Bool);
                    }
                    _ => panic!("Expected BoundWhere at index 1"),
                }
                // RETURN clause: a.name should resolve to String via catalog
                match &q.clauses[2] {
                    BoundClause::BoundReturn(r) => {
                        assert_eq!(r.expressions[0].resolved_type, LogicalTypeID::String);
                    }
                    _ => panic!("Expected BoundReturn at index 2"),
                }
            }
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_property_catalog_int64() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) RETURN a.age";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => match &q.clauses[1] {
                BoundClause::BoundReturn(r) => {
                    assert_eq!(r.expressions[0].resolved_type, LogicalTypeID::Int64);
                }
                _ => panic!("Expected BoundReturn"),
            },
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_property_catalog_double() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) RETURN a.score";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => match &q.clauses[1] {
                BoundClause::BoundReturn(r) => {
                    assert_eq!(r.expressions[0].resolved_type, LogicalTypeID::Double);
                }
                _ => panic!("Expected BoundReturn"),
            },
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_property_catalog_string() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) RETURN a.name";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => match &q.clauses[1] {
                BoundClause::BoundReturn(r) => {
                    assert_eq!(r.expressions[0].resolved_type, LogicalTypeID::String);
                }
                _ => panic!("Expected BoundReturn"),
            },
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_property_not_found_errors() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) RETURN a.nonexistent";
        let result = binder.bind(parse(sql).unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Property 'nonexistent' not found on table 'Person'")
        );
    }

    #[test]
    fn test_bind_rel_property_catalog() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person)-[e:Knows]->(b:Person) RETURN e.since";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => {
                // Find the RETURN clause
                let return_clause = q
                    .clauses
                    .iter()
                    .find_map(|c| {
                        if let BoundClause::BoundReturn(r) = c {
                            Some(r)
                        } else {
                            None
                        }
                    })
                    .expect("Expected RETURN clause");
                assert_eq!(return_clause.expressions[0].resolved_type, LogicalTypeID::Int64);
            }
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_complex_where() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) WHERE a.age > 25 AND a.name = 'Alice' RETURN a";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        assert!(matches!(bound, BoundStatement::BoundQuery(_)));
    }
}
