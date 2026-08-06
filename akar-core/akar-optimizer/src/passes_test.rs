//! Tests for optimizer passes — extracted from `passes.rs`.

#[cfg(test)]
mod tests {
    use crate::passes::OptimizationPass;
    use crate::passes::TreeOptimizationPass;
    use crate::passes::flat::constant_folding::fold_expression;
    use crate::passes::flat::join_optimization::{extract_root_variable, is_join_condition};
    use crate::passes::flat::scan_ops::is_tautology;
    use crate::passes::flat::*;
    use crate::passes::tree::*;
    use akar_binder::bound_statement::BoundExpression;
    use akar_common::types::LogicalTypeID;
    use akar_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};
    use akar_planner::logical_operator::*;

    fn make_scan(name: &str) -> LogicalOperator {
        LogicalOperator::ScanNode(LogicalScanNode {
            predicate: None,
            table_name: name.into(),
            table_id: 0,
            alias: None,
            columns: vec!["col1".into(), "col2".into()],
            cardinality: 0,
            fts_query: None,
        })
    }

    fn make_filter() -> LogicalOperator {
        LogicalOperator::Filter(LogicalFilter {
            expression: Expression::BinaryOp(
                BinaryOp::GreaterThan,
                Box::new(Expression::Variable("a".into())),
                Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(25))),
            ),
            children: Vec::new(),
            cardinality: 0,
        })
    }

    fn make_projection() -> LogicalOperator {
        LogicalOperator::Projection(LogicalProjection {
            expressions: vec![BoundExpression {
                expression: Expression::Variable("a".into()),
                resolved_type: LogicalTypeID::Any,
                is_constant: false,
            }],
            children: Vec::new(),
            cardinality: 0,
        })
    }

    fn make_order() -> LogicalOperator {
        LogicalOperator::OrderBy(LogicalOrderBy {
            sort_keys: vec![],
            children: Vec::new(),
            cardinality: 0,
        })
    }

    fn make_limit() -> LogicalOperator {
        LogicalOperator::Limit(LogicalLimit {
            limit: 10,
            offset: 0,
            children: Vec::new(),
            cardinality: 0,
        })
    }

    // ==================== Filter Push-Down Tests ====================

    #[test]
    fn test_filter_push_down() {
        let plan = vec![make_filter(), make_scan("Person"), make_projection()];
        let pass = FilterPushDown;
        let result = pass.apply(&plan);
        // Filter should be moved before Scan
        assert!(matches!(result[0], LogicalOperator::Filter(_)));
        assert!(matches!(result[1], LogicalOperator::ScanNode(_)));
    }

    #[test]
    fn test_combined_passes() {
        let plan = vec![
            make_filter(),
            make_filter(),
            make_scan("Person"),
            make_scan("City"),
            make_projection(),
        ];
        // Apply filter push-down
        let pass = FilterPushDown;
        let result = pass.apply(&plan);
        // Both filters should be before scans
        let filter_pos = result.iter().position(|op| matches!(op, LogicalOperator::Filter(_)));
        let scan_pos = result.iter().position(|op| matches!(op, LogicalOperator::ScanNode(_)));
        assert!(filter_pos.unwrap() < scan_pos.unwrap());
    }

    #[test]
    fn test_predicate_push_down() {
        let plan = vec![make_filter(), make_scan("Person"), make_projection()];
        let pass = PredicatePushDown;
        let result = pass.apply(&plan);
        // Filter should be merged into ScanNode
        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], LogicalOperator::ScanNode(s) if s.predicate.is_some()));
        assert!(matches!(&result[1], LogicalOperator::Projection(_)));
    }

    #[test]
    fn test_predicate_push_down_no_filter() {
        let plan = vec![make_scan("Person"), make_projection()];
        let pass = PredicatePushDown;
        let result = pass.apply(&plan);
        // No filter to push, plan unchanged
        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], LogicalOperator::ScanNode(s) if s.predicate.is_none()));
    }

    #[test]
    fn test_predicate_push_down_skip_existing_predicate() {
        let mut scan_with_pred = make_scan("Person");
        if let LogicalOperator::ScanNode(ref mut s) = scan_with_pred {
            s.predicate = Some(Expression::Constant(Constant::Bool(true)));
        }
        let plan = vec![make_filter(), scan_with_pred, make_projection()];
        let pass = PredicatePushDown;
        let result = pass.apply(&plan);
        // Scan already has predicate, so Filter should NOT be merged
        assert_eq!(result.len(), 3);
        assert!(matches!(result[0], LogicalOperator::Filter(_)));
        assert!(matches!(&result[1], LogicalOperator::ScanNode(s) if s.predicate.is_some()));
    }

    // ==================== Projection Push-Down Tests ====================

    #[test]
    fn test_projection_push_down() {
        let plan = vec![make_scan("Person"), make_filter(), make_projection()];
        let pass = ProjectionPushDown;
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 3);
    }

    // ==================== Join Optimization Tests ====================

    #[test]
    fn test_join_optimization() {
        let plan = vec![make_projection(), make_scan("Person"), make_scan("City"), make_filter()];
        let pass = JoinOptimization;
        let result = pass.apply(&plan);
        // JoinOptimization now converts equi-join filters to join conditions
        // The filter here is a.age > 25 (not equi-join), so it stays
        assert_eq!(result.len(), 3); // HashJoin/CrossProduct, Projection, Filter
    }

    #[test]
    fn test_is_join_condition() {
        // a.id = b.id is a join condition
        let expr = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("b".into())),
                "id".into(),
            )),
        );
        assert!(is_join_condition(&expr));
    }

    #[test]
    fn test_is_not_join_condition() {
        // a.age > 25 is NOT a join condition
        let expr = Expression::BinaryOp(
            BinaryOp::GreaterThan,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "age".into(),
            )),
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(25))),
        );
        assert!(!is_join_condition(&expr));
    }

    #[test]
    fn test_is_join_condition_same_var() {
        // a.id = a.id is NOT a join condition (same variable)
        let expr = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
        );
        assert!(!is_join_condition(&expr));
    }

    #[test]
    fn test_join_optimization_removes_equi_join_filter() {
        // Create filter with a.id = b.id (equi-join condition)
        let join_filter = LogicalOperator::Filter(LogicalFilter {
            expression: Expression::BinaryOp(
                BinaryOp::Equal,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("A".into())),
                    "id".into(),
                )),
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("B".into())),
                    "id".into(),
                )),
            ),
            children: Vec::new(),
            cardinality: 0,
        });
        let plan = vec![make_scan("A"), make_scan("B"), join_filter];
        let pass = JoinOptimization;
        let result = pass.apply(&plan);
        // Equi-join filter should be removed
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], LogicalOperator::HashJoin(_)));
    }

    // ==================== Top-K Tests ====================

    #[test]
    fn test_top_k_detection() {
        let plan = vec![make_order(), make_limit()];
        let pass = TopKOptimization;
        let result = pass.apply(&plan);
        // OrderBy + Limit fused into a single TopK
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], LogicalOperator::TopK(_)));
    }

    #[test]
    fn test_top_k_with_projection() {
        let plan = vec![make_order(), make_projection(), make_limit()];
        let pass = TopKOptimization;
        let result = pass.apply(&plan);
        // OrderBy + Projection + Limit → Projection + TopK (2 operators)
        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], LogicalOperator::Projection(_)));
        assert!(matches!(result[1], LogicalOperator::TopK(_)));
    }

    // ==================== Remove Unnecessary / Tautology Tests ====================

    #[test]
    fn test_remove_empty_projection() {
        let plan = vec![
            make_scan("Person"),
            LogicalOperator::Projection(LogicalProjection {
                expressions: vec![],
                children: Vec::new(),
                cardinality: 0,
            }),
        ];
        let pass = RemoveUnnecessaryOperators;
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 1); // Empty projection removed
    }

    #[test]
    fn test_is_tautology_true() {
        let expr = Expression::Constant(akar_parser::ast::Constant::Bool(true));
        assert!(is_tautology(&expr));
    }

    #[test]
    fn test_is_tautology_false() {
        let expr = Expression::Constant(akar_parser::ast::Constant::Bool(false));
        assert!(!is_tautology(&expr));
    }

    #[test]
    fn test_is_tautology_equal() {
        // 1 = 1 is a tautology
        let expr = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(1))),
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(1))),
        );
        assert!(is_tautology(&expr));
    }

    #[test]
    fn test_remove_tautology_filter() {
        let plan = vec![
            make_scan("Person"),
            LogicalOperator::Filter(LogicalFilter {
                expression: Expression::Constant(akar_parser::ast::Constant::Bool(true)),
                children: Vec::new(),
                cardinality: 0,
            }),
        ];
        let pass = RemoveUnnecessaryOperators;
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 1); // Tautology filter removed
    }

    // ==================== Constant Folding Tests ====================

    #[test]
    fn test_fold_integer_add() {
        let expr = Expression::BinaryOp(
            BinaryOp::Add,
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(1))),
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(2))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(akar_parser::ast::Constant::Integer(3)));
    }

    #[test]
    fn test_fold_integer_mul() {
        let expr = Expression::BinaryOp(
            BinaryOp::Multiply,
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(6))),
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(7))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(akar_parser::ast::Constant::Integer(42)));
    }

    #[test]
    fn test_fold_boolean_and() {
        let expr = Expression::BinaryOp(
            BinaryOp::And,
            Box::new(Expression::Constant(akar_parser::ast::Constant::Bool(true))),
            Box::new(Expression::Constant(akar_parser::ast::Constant::Bool(false))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(akar_parser::ast::Constant::Bool(false)));
    }

    #[test]
    fn test_fold_boolean_or() {
        let expr = Expression::BinaryOp(
            BinaryOp::Or,
            Box::new(Expression::Constant(akar_parser::ast::Constant::Bool(true))),
            Box::new(Expression::Constant(akar_parser::ast::Constant::Bool(false))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(akar_parser::ast::Constant::Bool(true)));
    }

    #[test]
    fn test_fold_string_concat() {
        let expr = Expression::BinaryOp(
            BinaryOp::Concat,
            Box::new(Expression::Constant(akar_parser::ast::Constant::String(
                "hello ".into(),
            ))),
            Box::new(Expression::Constant(akar_parser::ast::Constant::String("world".into()))),
        );
        let result = fold_expression(&expr);
        assert_eq!(
            result,
            Expression::Constant(akar_parser::ast::Constant::String("hello world".into()))
        );
    }

    #[test]
    fn test_fold_comparison_lt() {
        let expr = Expression::BinaryOp(
            BinaryOp::LessThan,
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(3))),
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(5))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(akar_parser::ast::Constant::Bool(true)));
    }

    #[test]
    fn test_fold_negate() {
        let expr = Expression::UnaryOp(
            UnaryOp::Negate,
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(42))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(akar_parser::ast::Constant::Integer(-42)));
    }

    #[test]
    fn test_fold_not() {
        let expr = Expression::UnaryOp(
            UnaryOp::Not,
            Box::new(Expression::Constant(akar_parser::ast::Constant::Bool(true))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(akar_parser::ast::Constant::Bool(false)));
    }

    #[test]
    fn test_fold_nested() {
        // (1 + 2) * 3 → 9
        let inner = Expression::BinaryOp(
            BinaryOp::Add,
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(1))),
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(2))),
        );
        let outer = Expression::BinaryOp(
            BinaryOp::Multiply,
            Box::new(inner),
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(3))),
        );
        let result = fold_expression(&outer);
        assert_eq!(result, Expression::Constant(akar_parser::ast::Constant::Integer(9)));
    }

    #[test]
    fn test_fold_mixed_types_no_fold() {
        // Variable + constant should NOT be folded
        let expr = Expression::BinaryOp(
            BinaryOp::Add,
            Box::new(Expression::Variable("x".into())),
            Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(1))),
        );
        let result = fold_expression(&expr);
        // Should remain unchanged
        assert!(matches!(result, Expression::BinaryOp(_, _, _)));
    }

    // ==================== Extract Root Variable Tests ====================

    #[test]
    fn test_extract_root_variable_simple() {
        let expr = Expression::Variable("x".into());
        assert_eq!(extract_root_variable(&expr), Some("x".into()));
    }

    #[test]
    fn test_extract_root_variable_property() {
        let expr = Expression::PropertyAccess(Box::new(Expression::Variable("p".into())), "name".into());
        assert_eq!(extract_root_variable(&expr), Some("p".into()));
    }

    #[test]
    fn test_extract_root_variable_constant() {
        let expr = Expression::Constant(akar_parser::ast::Constant::Integer(1));
        assert_eq!(extract_root_variable(&expr), None);
    }

    // ==================== Tree Pass Tests ====================

    #[test]
    fn test_factorization_rewriting_inserts_flatten() {
        // Build a small tree: HashJoin(ScanNode("A"), ScanNode("B"))
        let mut root = LogicalOperator::HashJoin(LogicalHashJoin {
            join_keys: vec![],
            build_side: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                predicate: None,
                table_name: "A".into(),
                table_id: 0,
                alias: None,
                columns: vec![],
                cardinality: 0,
                fts_query: None,
            })),
            probe_side: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                predicate: None,
                table_name: "B".into(),
                table_id: 1,
                alias: None,
                columns: vec![],
                cardinality: 0,
                fts_query: None,
            })),
            cardinality: 0,
            push_down_eligible: false,
        });

        let pass = FactorizationRewriting;
        pass.apply_tree(&mut root);

        // After rewriting, the hash join's children should be wrapped in Flatten
        match &root {
            LogicalOperator::HashJoin(hj) => {
                assert!(
                    matches!(&*hj.probe_side, LogicalOperator::Flatten(_)),
                    "Probe side should be wrapped in Flatten"
                );
                assert!(
                    matches!(&*hj.build_side, LogicalOperator::Flatten(_)),
                    "Build side should be wrapped in Flatten"
                );
            }
            _ => panic!("Expected HashJoin"),
        }
    }

    #[test]
    fn test_cardinality_estimation_scan_node() {
        let mut root = LogicalOperator::ScanNode(LogicalScanNode {
            predicate: None,
            table_name: "Person".into(),
            table_id: 0,
            alias: None,
            columns: vec![],
            cardinality: 0,
            fts_query: None,
        });

        let pass = CardinalityEstimation::new(None);
        pass.apply_tree(&mut root);

        // ScanNode should have default cardinality of 1000
        assert_eq!(root.cardinality(), 1000);
    }

    #[test]
    fn test_cardinality_estimation_aggregate_no_keys() {
        // Aggregate without GROUP BY → cardinality = 1
        let mut root = LogicalOperator::Aggregate(LogicalAggregate {
            group_by: vec![],
            aggregates: vec![],
            children: vec![LogicalOperator::ScanNode(LogicalScanNode {
                predicate: None,
                table_name: "T".into(),
                table_id: 0,
                alias: None,
                columns: vec![],
                cardinality: 0,
                fts_query: None,
            })],
            cardinality: 0,
        });

        let pass = CardinalityEstimation::new(None);
        pass.apply_tree(&mut root);

        assert_eq!(
            root.cardinality(),
            1,
            "Aggregate without GROUP BY should have cardinality 1"
        );
    }

    #[test]
    fn test_cardinality_estimation_limit() {
        // Limit(10) over ScanNode(1000) → cardinality = min(10, 1000) = 10
        let mut root = LogicalOperator::Limit(LogicalLimit {
            limit: 10,
            offset: 0,
            children: vec![LogicalOperator::ScanNode(LogicalScanNode {
                predicate: None,
                table_name: "T".into(),
                table_id: 0,
                alias: None,
                columns: vec![],
                cardinality: 1000,
                fts_query: None,
            })],
            cardinality: 0,
        });

        let pass = CardinalityEstimation::new(None);
        pass.apply_tree(&mut root);

        assert_eq!(
            root.cardinality(),
            10,
            "Limit should cap cardinality at its limit value"
        );
    }

    #[test]
    fn test_cardinality_estimation_cross_product() {
        let mut root = LogicalOperator::CrossProduct(LogicalCrossProduct {
            left: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                predicate: None,
                table_name: "A".into(),
                table_id: 0,
                alias: None,
                columns: vec![],
                cardinality: 0,
                fts_query: None,
            })),
            right: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                predicate: None,
                table_name: "B".into(),
                table_id: 1,
                alias: None,
                columns: vec![],
                cardinality: 0,
                fts_query: None,
            })),
            cardinality: 0,
        });

        let pass = CardinalityEstimation::new(None);
        pass.apply_tree(&mut root);

        // Both ScanNodes get default cardinality of 1000.
        // Cross product: 1000 * 1000 = 1,000,000
        assert_eq!(root.cardinality(), 1_000_000);
    }

    // ==================== AggKeyDependency Tests ====================

    fn make_aggregate(group_by: Vec<Expression>, child: LogicalOperator) -> LogicalOperator {
        LogicalOperator::Aggregate(LogicalAggregate {
            group_by,
            aggregates: vec![("COUNT".into(), vec![Expression::Constant(Constant::Integer(1))])],
            children: vec![child],
            cardinality: 0,
        })
    }

    #[test]
    fn test_agg_key_dependency_pk_only() {
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            predicate: None,
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec!["id".into(), "name".into(), "age".into()],
            cardinality: 0,
            fts_query: None,
        });
        let agg = make_aggregate(
            vec![
                Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "id".into()),
                Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "name".into()),
                Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "age".into()),
            ],
            scan,
        );

        let pass = AggKeyDependency;
        let mut plan = agg;
        pass.apply_tree(&mut plan);

        match plan {
            LogicalOperator::Aggregate(ref a) => {
                assert_eq!(a.group_by.len(), 1, "Should keep only a.id");
                assert_eq!(
                    a.group_by[0],
                    Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "id".into(),),
                );
            }
            _ => panic!("Expected Aggregate"),
        }
    }

    #[test]
    fn test_agg_key_dependency_no_pk_in_keys() {
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            predicate: None,
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec!["name".into(), "age".into()],
            cardinality: 0,
            fts_query: None,
        });
        let agg = make_aggregate(
            vec![
                Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "name".into()),
                Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "age".into()),
            ],
            scan,
        );

        let pass = AggKeyDependency;
        let mut plan = agg;
        pass.apply_tree(&mut plan);

        match plan {
            LogicalOperator::Aggregate(ref a) => {
                assert_eq!(a.group_by.len(), 1, "Should keep only first property");
                assert_eq!(
                    a.group_by[0],
                    Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "name".into(),),
                );
            }
            _ => panic!("Expected Aggregate"),
        }
    }

    #[test]
    fn test_agg_key_dependency_non_property_keys_unchanged() {
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            predicate: None,
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec!["id".into()],
            cardinality: 0,
            fts_query: None,
        });
        let agg = make_aggregate(
            vec![
                Expression::Constant(Constant::Integer(1)),
                Expression::Constant(Constant::Integer(2)),
            ],
            scan,
        );

        let pass = AggKeyDependency;
        let mut plan = agg;
        pass.apply_tree(&mut plan);

        match plan {
            LogicalOperator::Aggregate(ref a) => {
                // Constants are not dependent on each other — both stay
                assert_eq!(a.group_by.len(), 2, "Both constant keys should remain");
            }
            _ => panic!("Expected Aggregate"),
        }
    }

    #[test]
    fn test_agg_key_dependency_single_key_unchanged() {
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            predicate: None,
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec!["id".into()],
            cardinality: 0,
            fts_query: None,
        });
        let agg = make_aggregate(
            vec![Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )],
            scan,
        );

        let pass = AggKeyDependency;
        let mut plan = agg;
        pass.apply_tree(&mut plan);

        match plan {
            LogicalOperator::Aggregate(ref a) => {
                assert_eq!(a.group_by.len(), 1, "Single key should remain unchanged");
            }
            _ => panic!("Expected Aggregate"),
        }
    }
}
