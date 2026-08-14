//! Parser tests

use crate::ast::*;
use crate::parser::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_node_table() {
        let sql = "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateNodeTable(t) => {
                assert_eq!(t.name, "Person");
                assert_eq!(t.columns.len(), 2);
                assert_eq!(t.primary_key, "name");
                assert_eq!(t.columns[0].name, "name");
                assert_eq!(t.columns[0].type_name, "STRING");
            }
            _ => panic!("Expected CreateNodeTable"),
        }
    }

    #[test]
    fn test_drop_table() {
        let sql = "DROP TABLE Person";
        assert!(matches!(parse(sql).unwrap(), Statement::DropTable(t) if t.name == "Person"));
    }

    #[test]
    fn test_create_node_table_boolean_types() {
        // `BOOLEAN` must parse even though `BOOL` shares its prefix (P53.1
        // blocker: Kairos `_ensure_schema` uses `protected BOOLEAN`).
        let sql = "CREATE NODE TABLE T(id INT64, flag BOOLEAN, b BOOL, PRIMARY KEY (id))";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateNodeTable(t) => {
                assert_eq!(t.columns[1].type_name, "BOOLEAN");
                assert_eq!(t.columns[2].type_name, "BOOL");
            }
            _ => panic!("Expected CreateNodeTable"),
        }
    }

    #[test]
    fn test_match_return() {
        let sql = "MATCH (a:Person) RETURN a.name, a.age";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Query(q) => {
                assert_eq!(q.clauses.len(), 2);
                assert!(matches!(q.clauses[0], Clause::Match(_)));
                assert!(matches!(q.clauses[1], Clause::Return(_)));
            }
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn test_match_where_return() {
        let sql = "MATCH (a:Person) WHERE a.age > 25 RETURN a.name";
        let stmt = parse(sql).unwrap();
        assert!(matches!(stmt, Statement::Query(q) if q.clauses.len() == 3));
    }

    #[test]
    fn test_match_with_string() {
        let sql = "MATCH (a:Person) WHERE a.name = 'Alice' RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_limit_negative_errors() {
        // P51.31: negative LIMIT must error instead of silently dropping the limit
        let err = parse("MATCH (a:Person) RETURN a.name LIMIT -1").unwrap_err();
        assert!(err.contains("LIMIT"), "Expected LIMIT error, got: {err}");
    }

    #[test]
    fn test_limit_overflow_errors() {
        // P51.31: u64 overflow must error instead of silently dropping the limit
        let err = parse("MATCH (a:Person) RETURN a.name LIMIT 99999999999999999999999").unwrap_err();
        assert!(err.contains("LIMIT"), "Expected LIMIT error, got: {err}");
    }

    #[test]
    fn test_skip_negative_errors() {
        let err = parse("MATCH (a:Person) RETURN a.name SKIP -1").unwrap_err();
        assert!(err.contains("SKIP"), "Expected SKIP error, got: {err}");
    }

    #[test]
    fn test_rel_pattern() {
        let sql = "MATCH (a:Person)-[r:Knows]->(b:Person) RETURN a, b";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_lower_upper_parse() {
        let sql = "RETURN lower('HELLO'), upper('hello')";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_cast_aliases_parse() {
        let sql = "RETURN date('2024-01-01'), float('3.14'), bool('true')";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_function_call() {
        let sql = "MATCH (a:Person) RETURN COUNT(a)";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_function_call_ast_nextval_currval() {
        let sql = "RETURN nextval('my_seq'), currval('my_seq')";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Query(q) => {
                assert_eq!(q.clauses.len(), 1);
                match &q.clauses[0] {
                    Clause::Return(r) => {
                        assert_eq!(r.expressions.len(), 2);
                        assert!(matches!(
                            &r.expressions[0].expression,
                            Expression::FunctionCall(name, args)
                            if name == "nextval" && args.len() == 1
                        ));
                        assert!(matches!(
                            &r.expressions[1].expression,
                            Expression::FunctionCall(name, args)
                            if name == "currval" && args.len() == 1
                        ));
                    }
                    _ => panic!("Expected Return clause"),
                }
            }
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn test_count_star() {
        let sql = "MATCH (a:Person) RETURN COUNT(*)";
        let result = parse(sql);
        assert!(result.is_ok(), "COUNT(*) should parse: {:?}", result.err());
    }

    #[test]
    fn test_integer_expr() {
        let sql = "MATCH (a) WHERE a.age = 30 RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_boolean_expr() {
        let sql = "MATCH (a) WHERE a.active = TRUE RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_comparison_ops() {
        for sql in [
            "MATCH (a) WHERE a.age <= 30 RETURN a",
            "MATCH (a) WHERE a.age >= 30 RETURN a",
            "MATCH (a) WHERE a.age > 30 RETURN a",
            "MATCH (a) WHERE a.age < 30 RETURN a",
        ] {
            assert!(parse(sql).is_ok(), "should parse: {sql}");
        }
    }

    #[test]
    fn test_list_literal() {
        let sql = "MATCH (a) WHERE a.age IN [1, 2, 3] RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_foreach_basic() {
        let sql = "FOREACH (x IN [1,2,3] | CREATE (n:Num {val: x}))";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Query(q) => {
                assert_eq!(q.clauses.len(), 1);
                match &q.clauses[0] {
                    Clause::Foreach(f) => {
                        assert_eq!(f.variable, "x");
                        assert_eq!(f.clauses.len(), 1);
                    }
                    _ => panic!("Expected Foreach clause"),
                }
            }
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn test_foreach_in_match() {
        let sql = "MATCH (a:Person) FOREACH (x IN [1,2] | SET a.val = x)";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_var_length_path_simple() {
        let sql = "MATCH (a:Person)-[*]->(b:Person) RETURN a, b";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_var_length_path_with_bounds() {
        let sql = "MATCH (a:Person)-[*1..5]->(b:Person) RETURN a, b";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_var_length_path_with_rel_variable() {
        let sql = "MATCH (a:Person)-[r*1..3]->(b:Person) RETURN a, b";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_exists_subquery_basic() {
        let sql = "MATCH (a:Person) WHERE EXISTS { MATCH (b:City) WHERE b.name = a.name } RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_exists_subquery_in_return() {
        let sql = "MATCH (a:Person) RETURN EXISTS { MATCH (b:City) WHERE b.pop > 1000 }";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_complex_and_or() {
        let sql = "MATCH (a:Person) WHERE a.age > 25 AND a.name = 'Bob' OR a.age < 10 RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_create_rel_table() {
        let sql = "CREATE REL TABLE Knows(FROM Person TO Person, since INT64)";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateRelTable(t) => {
                assert_eq!(t.name, "Knows");
                assert_eq!(t.columns.len(), 1);
            }
            _ => panic!("Expected CreateRelTable"),
        }
    }

    #[test]
    fn test_parameter_syntax() {
        let sql = "MATCH (a:Person) WHERE a.age > $min RETURN a";
        let stmt = parse(sql).unwrap();
        assert!(matches!(stmt, Statement::Query(_)));
    }

    #[test]
    fn test_multiple_parameters() {
        let sql = "MATCH (a:Person) WHERE a.age > $min AND a.age < $max RETURN a";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_parameter_with_string() {
        let sql = "MATCH (a:Person) WHERE a.name = $name RETURN a";
        assert!(parse(sql).is_ok());
    }

    // ==================== COPY FROM tests ====================

    #[test]
    fn test_copy_to_parquet() {
        let sql = "COPY (MATCH (a:User) RETURN a.id, a.name) TO 'test.parquet' (FORMAT PARQUET)";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CopyTo(c) => {
                assert_eq!(c.file_path, "test.parquet");
                assert!(matches!(c.format, CopyToFormat::Parquet));
                assert!(!c.header);
            }
            _ => panic!("Expected CopyTo"),
        }
    }

    #[test]
    fn test_copy_to_csv() {
        let sql = "COPY (MATCH (a:User) RETURN a.id, a.name) TO 'test.csv' (FORMAT CSV, HEADER true)";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CopyTo(c) => {
                assert_eq!(c.file_path, "test.csv");
                assert!(matches!(c.format, CopyToFormat::Csv));
                assert!(c.header);
            }
            _ => panic!("Expected CopyTo"),
        }
    }

    #[test]
    fn test_copy_to_default_csv() {
        let sql = "COPY (MATCH (a:User) RETURN a.id, a.name) TO 'test.csv'";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CopyTo(c) => {
                assert_eq!(c.file_path, "test.csv");
                assert!(matches!(c.format, CopyToFormat::Csv));
                assert!(!c.header);
            }
            _ => panic!("Expected CopyTo"),
        }
    }

    #[test]
    fn test_copy_from_basic() {
        let sql = "COPY Person FROM 'data.csv'";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CopyFrom(c) => {
                assert_eq!(c.table_name, "Person");
                assert!(c.file_path.contains("data.csv"));
                assert!(c.options.is_empty());
            }
            _ => panic!("Expected CopyFrom, got {:?}", stmt),
        }
    }

    #[test]
    fn test_copy_from_with_options() {
        let sql = "COPY Person FROM 'data.csv' (HEADER TRUE, DELIM ',')";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CopyFrom(c) => {
                assert_eq!(c.table_name, "Person");
                assert_eq!(c.options.get("header").map(|s| s.as_str()), Some("TRUE"));
                assert_eq!(c.options.get("delim").map(|s| s.as_str()), Some(","));
            }
            _ => panic!("Expected CopyFrom"),
        }
    }

    #[test]
    fn test_copy_from_full_options() {
        let sql = "COPY Knows FROM 'rels.csv' (HEADER TRUE, DELIM '|', QUOTE '\"', ESCAPE '\\\\')";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CopyFrom(c) => {
                assert_eq!(c.table_name, "Knows");
                assert_eq!(c.options.get("header").map(|s| s.as_str()), Some("TRUE"));
                assert_eq!(c.options.get("delim").map(|s| s.as_str()), Some("|"));
            }
            _ => panic!("Expected CopyFrom"),
        }
    }

    #[test]
    fn test_copy_from_parse_error() {
        // Missing file path
        let sql = "COPY Person FROM";
        assert!(parse(sql).is_err());
    }

    #[test]
    fn test_copy_from_single_quoted_path() {
        let sql = "COPY Person FROM 'path/to/data.csv'";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CopyFrom(c) => {
                assert_eq!(c.table_name, "Person");
                assert!(c.file_path.contains("path/to/data.csv"));
            }
            _ => panic!("Expected CopyFrom"),
        }
    }

    // --- EXPLAIN tests ---
    #[test]
    fn test_explain_query() {
        let sql = "EXPLAIN MATCH (a:Person) RETURN a";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Explain(e) => {
                assert_eq!(e.explain_type, ExplainType::PhysicalPlan);
                assert!(matches!(*e.statement, Statement::Query(_)));
            }
            _ => panic!("Expected Explain, got {:?}", stmt),
        }
    }

    #[test]
    fn test_explain_logical() {
        let sql = "EXPLAIN LOGICAL MATCH (a:Person) RETURN a";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Explain(e) => {
                assert_eq!(e.explain_type, ExplainType::LogicalPlan);
            }
            _ => panic!("Expected Explain(Logical)"),
        }
    }

    #[test]
    fn test_explain_profile() {
        let sql = "EXPLAIN PROFILE MATCH (a:Person) RETURN a";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Explain(e) => {
                assert_eq!(e.explain_type, ExplainType::Profile);
            }
            _ => panic!("Expected Explain(Profile)"),
        }
    }

    #[test]
    fn test_explain_create_table() {
        let sql = "EXPLAIN CREATE NODE TABLE Test(id INT64, PRIMARY KEY (id))";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Explain(e) => {
                assert!(matches!(*e.statement, Statement::CreateNodeTable(_)));
            }
            _ => panic!("Expected Explain(CreateNodeTable)"),
        }
    }

    // --- Sequence tests ---

    #[test]
    fn test_create_sequence_basic() {
        let sql = "CREATE SEQUENCE my_seq";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert!(!s.if_not_exists);
                assert!(!s.or_replace);
                assert_eq!(s.start_with, None);
                assert_eq!(s.increment, None);
            }
            _ => panic!("Expected CreateSequence, got {:?}", stmt),
        }
    }

    #[test]
    fn test_create_sequence_if_not_exists() {
        let sql = "CREATE SEQUENCE IF NOT EXISTS my_seq";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert!(s.if_not_exists);
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_create_sequence_or_replace() {
        let sql = "CREATE OR REPLACE SEQUENCE my_seq";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert!(s.or_replace);
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_create_sequence_start_with() {
        let sql = "CREATE SEQUENCE my_seq START 100";
        let result = parse(sql);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());
        let stmt = result.unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert_eq!(s.start_with, Some(100));
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_create_sequence_start_with_with() {
        let sql = "CREATE SEQUENCE my_seq START WITH 100";
        let result = parse(sql);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());
        let stmt = result.unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert_eq!(s.start_with, Some(100));
            }
            _ => panic!("Expected CreateSequence, got {:?}", stmt),
        }
    }

    #[test]
    fn test_create_sequence_full_options() {
        let sql = "CREATE SEQUENCE my_seq START 100 INCREMENT 5 MINVALUE 1 MAXVALUE 1000 CYCLE";
        let result = parse(sql);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());
        let stmt = result.unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert_eq!(s.start_with, Some(100));
                assert_eq!(s.increment, Some(5));
                assert_eq!(s.min_value, Some(1));
                assert_eq!(s.max_value, Some(1000));
                assert_eq!(s.cycle, Some(true));
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_create_sequence_no_cycle() {
        let sql = "CREATE SEQUENCE my_seq NO CYCLE";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.cycle, Some(false));
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_create_sequence_no_minvalue() {
        let sql = "CREATE SEQUENCE my_seq NO MINVALUE";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.min_value, None);
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_create_sequence_negative_increment() {
        let sql = "CREATE SEQUENCE my_seq INCREMENT -1 START 100";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateSequence(s) => {
                assert_eq!(s.increment, Some(-1));
                assert_eq!(s.start_with, Some(100));
            }
            _ => panic!("Expected CreateSequence"),
        }
    }

    #[test]
    fn test_drop_sequence() {
        let sql = "DROP SEQUENCE my_seq";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::DropSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert!(!s.if_exists);
            }
            _ => panic!("Expected DropSequence"),
        }
    }

    #[test]
    fn test_drop_sequence_if_exists() {
        let sql = "DROP SEQUENCE IF EXISTS my_seq";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::DropSequence(s) => {
                assert_eq!(s.name, "my_seq");
                assert!(s.if_exists);
            }
            _ => panic!("Expected DropSequence"),
        }
    }

    #[test]
    fn test_export_database_basic() {
        let sql = "EXPORT DATABASE '/tmp/export'";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::ExportDatabase(e) => {
                assert_eq!(e.file_path, "/tmp/export");
                assert!(e.options.is_empty());
            }
            _ => panic!("Expected ExportDatabase"),
        }
    }

    #[test]
    fn test_import_database_basic() {
        let sql = "IMPORT DATABASE '/tmp/export'";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::ImportDatabase(i) => {
                assert_eq!(i.file_path, "/tmp/export");
            }
            _ => panic!("Expected ImportDatabase"),
        }
    }

    // ==================== CREATE MACRO tests ====================

    #[test]
    fn test_create_macro_no_args() {
        let sql = "CREATE MACRO my_macro() AS 42";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateMacro(m) => {
                assert_eq!(m.name, "my_macro");
                assert!(m.positional_args.is_empty());
                assert!(m.default_args.is_empty());
            }
            _ => panic!("Expected CreateMacro"),
        }
    }

    #[test]
    fn test_create_macro_positional_args() {
        let sql = "CREATE MACRO double(x) AS x * 2";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateMacro(m) => {
                assert_eq!(m.name, "double");
                assert_eq!(m.positional_args, vec!["x"]);
                assert!(m.default_args.is_empty());
            }
            _ => panic!("Expected CreateMacro"),
        }
    }

    #[test]
    fn test_create_macro_multiple_positional_args() {
        let sql = "CREATE MACRO add(x, y) AS x + y";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateMacro(m) => {
                assert_eq!(m.name, "add");
                assert_eq!(m.positional_args, vec!["x", "y"]);
            }
            _ => panic!("Expected CreateMacro"),
        }
    }

    #[test]
    fn test_create_macro_with_default_arg() {
        let sql = "CREATE MACRO inc(x, y = 1) AS x + y";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateMacro(m) => {
                assert_eq!(m.name, "inc");
                assert_eq!(m.positional_args, vec!["x"]);
                assert_eq!(m.default_args.len(), 1);
                assert_eq!(m.default_args[0].0, "y");
            }
            _ => panic!("Expected CreateMacro"),
        }
    }

    #[test]
    fn test_create_macro_expression_body() {
        let sql = "CREATE MACRO square(x) AS x * x";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateMacro(m) => {
                assert_eq!(m.name, "square");
                assert_eq!(m.positional_args, vec!["x"]);
                // Expression should be BinaryOp(Mul, Variable("x"), Variable("x"))
                match &*m.expression {
                    Expression::BinaryOp(op, _, _) => {
                        assert_eq!(*op, BinaryOp::Multiply);
                    }
                    _ => panic!("Expected BinaryOp in macro body"),
                }
            }
            _ => panic!("Expected CreateMacro"),
        }
    }

    // --- List predicate tests ---

    #[test]
    fn test_list_predicate_any() {
        let sql = "RETURN ANY(x IN [1,2,3] WHERE x > 2)";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Query(q) => match &q.clauses[0] {
                Clause::Return(r) => {
                    assert_eq!(r.expressions.len(), 1);
                    match &r.expressions[0].expression {
                        Expression::ListPredicate {
                            quantifier,
                            var_name,
                            list,
                            predicate,
                        } => {
                            assert_eq!(*quantifier, Quantifier::Any);
                            assert_eq!(var_name, "x");
                            assert!(matches!(&**list, Expression::List(_)));
                            assert!(matches!(&**predicate, Expression::BinaryOp(_, _, _)));
                        }
                        _ => panic!("Expected ListPredicate"),
                    }
                }
                _ => panic!("Expected Return clause"),
            },
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn test_list_predicate_all() {
        let sql = "RETURN ALL(x IN list WHERE x > 0)";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_list_predicate_none() {
        let sql = "MATCH (n) WHERE NONE(x IN n.list WHERE x = 0) RETURN n";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_list_predicate_single() {
        let sql = "MATCH (n) WHERE SINGLE(x IN n.list WHERE x < 0) RETURN n";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_list_predicate_in_return() {
        let sql = "RETURN ALL(x IN [true, false] WHERE x)";
        assert!(parse(sql).is_ok());
    }

    #[test]
    fn test_list_predicate_nested() {
        let sql = "RETURN ANY(x IN [1,2,3] WHERE x > 0 AND x < 5)";
        assert!(parse(sql).is_ok());
    }

    // --- ANALYZE tests ---

    #[test]
    fn test_analyze_star() {
        let sql = "ANALYZE *";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Analyze(a) => assert_eq!(a.table_name, None),
            _ => panic!("Expected Analyze, got {:?}", stmt),
        }
    }

    #[test]
    fn test_analyze_table() {
        let sql = "ANALYZE Person";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Analyze(a) => assert_eq!(a.table_name, Some("Person".to_string())),
            _ => panic!("Expected Analyze, got {:?}", stmt),
        }
    }

    #[test]
    fn test_analyze_table_with_keyword() {
        let sql = "ANALYZE TABLE Person";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::Analyze(a) => assert_eq!(a.table_name, Some("Person".to_string())),
            _ => panic!("Expected Analyze, got {:?}", stmt),
        }
    }

    #[test]
    fn test_create_fts_index_parse() {
        let sql = "CREATE FTS INDEX idx_name ON (Person.bio)";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateFtsIndex(c) => {
                assert_eq!(c.index_name, "idx_name");
                assert_eq!(c.table_name, "Person");
                assert_eq!(c.column_name, "bio");
                assert!(!c.if_not_exists);
            }
            _ => panic!("Expected CreateFtsIndex, got {:?}", stmt),
        }
    }

    #[test]
    fn test_create_vector_index_parse() {
        // P53: Kuzu-style `CREATE VECTOR INDEX ... WITH (metric=..., dims=N)`.
        // Regression: options are nested under `vector_index_option` and pest does
        // not expose string literals as inner pairs, so metric/dims were dropped.
        let sql = "CREATE VECTOR INDEX mem_vec ON (Memory.embedding) WITH (metric=cosine, dims=384)";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateVectorIndex(c) => {
                assert_eq!(c.index_name, "mem_vec");
                assert_eq!(c.table_name, "Memory");
                assert_eq!(c.column_name, "embedding");
                assert_eq!(c.metric, "cosine");
                assert_eq!(c.dimensions, 384);
            }
            _ => panic!("Expected CreateVectorIndex, got {:?}", stmt),
        }
    }

    #[test]
    fn test_create_vector_index_dims_first() {
        let sql = "CREATE VECTOR INDEX mem_vec ON (Memory.embedding) WITH (dims=384, metric=euclidean)";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateVectorIndex(c) => {
                assert_eq!(c.metric, "euclidean");
                assert_eq!(c.dimensions, 384);
            }
            _ => panic!("Expected CreateVectorIndex, got {:?}", stmt),
        }
    }
}
