    use super::*;
    use hashbrown::HashMap;
    use akar_binder::bound_statement::BoundExpression;
    use akar_common::types::{LogicalTypeID, Value};
    use akar_parser::ast::{Constant, Expression};
    use akar_storage::table::ColumnDefinition;

    fn make_scan_op() -> LogicalOperator {
        LogicalOperator::ScanNode(akar_planner::logical_operator::LogicalScanNode { predicate: None,
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec![],
            cardinality: 0,
            fts_query: None,
        })
    }

    fn make_filter_op() -> LogicalOperator {
        LogicalOperator::Filter(akar_planner::logical_operator::LogicalFilter {
            expression: Expression::Constant(Constant::Bool(true)),
            children: vec![],
            cardinality: 0,
        })
    }

    fn make_proj_op() -> LogicalOperator {
        LogicalOperator::Projection(akar_planner::logical_operator::LogicalProjection {
            expressions: vec![BoundExpression {
                expression: Expression::Variable("a".into()),
                resolved_type: LogicalTypeID::Any,
                is_constant: false,
            }],
            children: vec![],
            cardinality: 0,
        })
    }

    fn make_limit_op() -> LogicalOperator {
        LogicalOperator::Limit(akar_planner::logical_operator::LogicalLimit {
            limit: 10,
            offset: 0,
            children: vec![],
            cardinality: 0,
        })
    }

    /// Create a processor with a Person table containing test data.
    fn make_processor_with_person_table() -> QueryProcessor {
        let catalog = Arc::new(TableCatalog::new());
        {
            catalog.create_node_table(
                "Person".into(),
                vec![
                    ColumnDefinition { compression: akar_common::enums::CompressionType::Uncompressed,
                        name: "name".into(),
                        logical_type: LogicalTypeID::String,
                        is_primary_key: true,
                    },
                    ColumnDefinition { compression: akar_common::enums::CompressionType::Uncompressed,
                        name: "age".into(),
                        logical_type: LogicalTypeID::Int64,
                        is_primary_key: false,
                    },
                ],
            );
            // Insert some data
            let mut table = catalog.get_node_table_by_name_mut("Person").unwrap();
            table
                .insert_row(vec![Value::String("Alice".into()), Value::Int64(30)])
                .unwrap();
            table
                .insert_row(vec![Value::String("Bob".into()), Value::Int64(25)])
                .unwrap();
        }
        let registry = Arc::new(Mutex::new(FunctionRegistry::new()));
        QueryProcessor::with_catalog(
            registry,
            catalog,
            std::sync::Arc::new(akar_common::file_system::VirtualFileSystemRegistry::new()),
        )
    }

    #[test]
    fn test_empty_plan() {
        let proc = QueryProcessor::new();
        let result = proc.execute(&[]).unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_scan_only() {
        let proc = make_processor_with_person_table();
        let result = proc.execute(&[make_scan_op()]).unwrap();
        assert!(!result.is_empty());
        assert!(result[0].num_fields() > 0);
        assert_eq!(result[0].size, 2); // 2 rows
    }

    #[test]
    fn test_scan_filter_projection() {
        let proc = make_processor_with_person_table();
        let plan = vec![make_scan_op(), make_filter_op(), make_proj_op()];
        let result = proc.execute(&plan).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_scan_filter_limit() {
        let proc = make_processor_with_person_table();
        let plan = vec![make_scan_op(), make_filter_op(), make_limit_op()];
        let result = proc.execute(&plan).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_filter_true_passthrough() {
        let filter = PhysicalFilter::new(Expression::Constant(Constant::Bool(true)));
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v.set_i64(i, i as i64);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = filter.execute(input).unwrap();
        assert!(!result.is_empty());
        assert_eq!(result[0].size, 5); // All rows pass through
    }

    #[test]
    fn test_filter_false_removes_all() {
        let filter = PhysicalFilter::new(Expression::Constant(Constant::Bool(false)));
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v.set_i64(i, i as i64);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = filter.execute(input).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_limit() {
        let limit = PhysicalLimit { limit: 3, offset: 0 };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 10);
        for i in 0..10 {
            v.set_i64(i, i as i64);
        }
        v.resize(10);
        let input = vec![DataChunk::new(vec![v])];
        let result = limit.execute(input).unwrap();
        assert_eq!(result[0].size, 3);
    }

    #[test]
    fn test_limit_with_offset() {
        let limit = PhysicalLimit { limit: 2, offset: 5 };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 10);
        for i in 0..10 {
            v.set_i64(i, i as i64);
        }
        v.resize(10);
        let input = vec![DataChunk::new(vec![v])];
        let result = limit.execute(input).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_projection() {
        let proj = PhysicalProjection {
            column_indices: vec![0],
        };
        let mut v1 = ValueVector::new(PhysicalTypeID::Int64, 5);
        let mut v2 = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v1.set_i64(i, i as i64);
            v2.set_i64(i, (i * 10) as i64);
        }
        v1.resize(5);
        v2.resize(5);
        let input = vec![DataChunk::new(vec![v1, v2])];
        let result = proj.execute(input).unwrap();
        assert_eq!(result[0].num_fields(), 1); // Only first column
    }

    #[test]
    fn test_projection_evaluates_function_call_no_input_source() {
        let state = Arc::new(Mutex::new(HashMap::new()));
        state.lock().unwrap().insert("s".to_string(), 1_i64);
        let state_for_fn = state.clone();
        let seq_fn: Arc<dyn Fn(&str, bool) -> Result<Value, akar_common::error::ProcessorError> + Send + Sync> =
            Arc::new(move |seq_name: &str, is_nextval: bool| {
                let mut m = state_for_fn.lock().map_err(|e| format!("Lock error: {e}"))?;
                let v = m
                    .get_mut(seq_name)
                    .ok_or_else(|| format!("Sequence '{}' not found", seq_name))?;
                if is_nextval {
                    let out = *v;
                    *v += 1;
                    Ok(Value::Int64(out))
                } else {
                    Ok(Value::Int64(*v))
                }
            });

        let proc =
            QueryProcessor::with_registry(Arc::new(Mutex::new(FunctionRegistry::new()))).with_sequence_fn(seq_fn);

        let plan = vec![LogicalOperator::Projection(
            akar_planner::logical_operator::LogicalProjection {
                expressions: vec![BoundExpression {
                    expression: Expression::FunctionCall(
                        "nextval".into(),
                        vec![Expression::Constant(Constant::String("s".into()))],
                    ),
                    resolved_type: LogicalTypeID::Int64,
                    is_constant: false,
                }],
                children: vec![],
                cardinality: 1,
            },
        )];

        let result = proc.execute(&plan).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 1);
        assert_eq!(result[0].fields[0].get_value(0), Some(Value::Int64(1)));
    }

    #[test]
    fn test_projection_sequence_missing_callback_errors() {
        let proc = QueryProcessor::with_registry(Arc::new(Mutex::new(FunctionRegistry::new())));

        let plan = vec![LogicalOperator::Projection(
            akar_planner::logical_operator::LogicalProjection {
                expressions: vec![BoundExpression {
                    expression: Expression::FunctionCall(
                        "nextval".into(),
                        vec![Expression::Constant(Constant::String("s".into()))],
                    ),
                    resolved_type: LogicalTypeID::Int64,
                    is_constant: false,
                }],
                children: vec![],
                cardinality: 1,
            },
        )];

        let err = proc.execute(&plan).unwrap_err();
        assert!(
            err.to_string().contains("No sequence callback configured"),
            "Unexpected error: {err}"
        );
    }

    // ==================== OrderBy Tests ====================

    #[test]
    fn test_order_by_ascending() {
        let order = PhysicalOrderBy {
            sort_keys: vec![(0, true)],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        let vals = [5, 3, 1, 4, 2];
        for i in 0..5 {
            v.set_i64(i, vals[i]);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = order.execute(input).unwrap();
        assert!(!result.is_empty());
        let sorted = result[0].fields[0].get_i64(0).unwrap();
        assert_eq!(sorted, 1); // Min should be first
    }

    #[test]
    fn test_order_by_descending() {
        let order = PhysicalOrderBy {
            sort_keys: vec![(0, false)],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        let vals = [5, 3, 1, 4, 2];
        for i in 0..5 {
            v.set_i64(i, vals[i]);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = order.execute(input).unwrap();
        assert!(!result.is_empty());
        let sorted = result[0].fields[0].get_i64(0).unwrap();
        assert_eq!(sorted, 5); // Max should be first
    }

    #[test]
    fn test_order_by_empty_input() {
        let order = PhysicalOrderBy {
            sort_keys: vec![(0, true)],
        };
        let result = order.execute(vec![]).unwrap();
        assert!(result.is_empty());
    }

    // ==================== Aggregate Tests ====================

    #[test]
    fn test_aggregate_count() {
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["COUNT".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v.set_i64(i, i as i64);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(5)); // COUNT = 5
    }

    #[test]
    fn test_aggregate_sum() {
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["SUM".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 4);
        for i in 0..4 {
            v.set_i64(i, (i + 1) as i64);
        }
        v.resize(4);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(10)); // 1+2+3+4 = 10
    }

    #[test]
    fn test_aggregate_min_max() {
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["MIN".into(), "MAX".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        let vals = [42, 7, 99, 15, 3];
        for i in 0..5 {
            v.set_i64(i, vals[i]);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(3)); // MIN = 3
        assert_eq!(result[0].fields[1].get_value(0).unwrap(), Value::Int64(99)); // MAX = 99
    }

    #[test]
    fn test_aggregate_avg() {
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["AVG".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 4);
        for i in 0..4 {
            v.set_i64(i, (i + 1) as i64);
        }
        v.resize(4);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        // AVG now returns Double (Value::Double)
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Double(2.5)); // (1+2+3+4)/4 = 2.5
    }

    #[test]
    fn test_aggregate_empty_input() {
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["COUNT".into()],
        };
        let result = agg.execute(vec![]).unwrap();
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(0)); // COUNT of empty = 0
    }

    // ==================== HashJoin Tests ====================

    #[test]
    fn test_hash_join_basic() {
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: None,
        };
        // Build side: keys [1, 2, 3]
        let mut build = ValueVector::new(PhysicalTypeID::Int64, 3);
        for i in 0..3 {
            build.set_i64(i, (i + 1) as i64);
        }
        build.resize(3);
        // Probe side: keys [2, 3, 4]
        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 3);
        probe.set_i64(0, 2);
        probe.set_i64(1, 3);
        probe.set_i64(2, 4);
        probe.resize(3);
        let build_chunk = DataChunk::new(vec![build]);
        let probe_chunk = DataChunk::new(vec![probe]);
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();
        // Should match 2 and 3 (2 rows)
        assert!(!result.is_empty());
    }

    #[test]
    fn test_hash_join_no_match() {
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: None,
        };
        // Build: [1, 2]
        let mut build = ValueVector::new(PhysicalTypeID::Int64, 2);
        build.set_i64(0, 1);
        build.set_i64(1, 2);
        build.resize(2);
        // Probe: [3, 4]
        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 2);
        probe.set_i64(0, 3);
        probe.set_i64(1, 4);
        probe.resize(2);
        let build_chunk = DataChunk::new(vec![build]);
        let probe_chunk = DataChunk::new(vec![probe]);
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();
        assert!(result.is_empty()); // No matches
    }

    #[test]
    fn test_hash_join_empty_build() {
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: None,
        };
        let build = ValueVector::new(PhysicalTypeID::Int64, 0);
        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 3);
        probe.set_i64(0, 1);
        probe.set_i64(1, 2);
        probe.set_i64(2, 3);
        probe.resize(3);
        let build_chunk = DataChunk::new(vec![build]);
        let probe_chunk = DataChunk::new(vec![probe]);
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();
        assert!(result.is_empty()); // Empty build G�� no matches
    }

    // ==================== Edge Case Tests ====================

    #[test]
    fn test_hash_join_null_keys_no_match() {
        // SQL semantics: NULL keys should never match in a join
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: None,
        };
        // Build side with NULLs mixed with real values
        let mut build = ValueVector::new(PhysicalTypeID::Int64, 3);
        build.set_i64(0, 1);
        // Row 1 stays NULL
        build.set_i64(2, 3);
        build.resize(3);
        // Probe side also has NULLs
        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 3);
        probe.set_i64(0, 1);
        probe.set_i64(1, 3);
        // Row 2 stays NULL
        probe.resize(3);
        let build_chunk = DataChunk::new(vec![build]);
        let probe_chunk = DataChunk::new(vec![probe]);
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();
        // Should match 1G��1 (1 row) and 3G��3 (1 row)
        // NULLs should NOT match each other
        assert!(!result.is_empty(), "Expected at least one matching row");
    }

    #[test]
    fn test_hash_join_all_null_keys() {
        // When both sides have all NULL keys G�� no matches
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: None,
        };
        let mut build = ValueVector::new(PhysicalTypeID::Int64, 3);
        build.resize(3);
        build.set_null(0, true);
        build.set_null(1, true);
        build.set_null(2, true);

        let mut probe = ValueVector::new(PhysicalTypeID::Int64, 3);
        probe.resize(3);
        probe.set_null(0, true);
        probe.set_null(1, true);
        probe.set_null(2, true);

        let build_chunk = DataChunk::new(vec![build]);
        let probe_chunk = DataChunk::new(vec![probe]);
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();
        // NULL = NULL is unknown in SQL, so no matches
        assert!(result.is_empty());
    }

    // ==================== SemiMasker (SIP) Tests ====================

    #[test]
    fn test_semi_masker_basic() {
        // Create a semi-masker that collects Int64 values (node offsets)
        let mask = NodeSemiMask::new(0);
        let masker = PhysicalSemiMasker {
            key_column: 0,
            mask: mask.clone(),
        };

        // Input: chunk with Int64 values representing node offsets
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 3);
        v.resize(3);
        v.set_i64(0, 10);
        v.set_i64(1, 20);
        v.set_i64(2, 30);
        let input = vec![DataChunk::new(vec![v])];
        let result = masker.execute(input).unwrap();
        assert_eq!(result.len(), 1, "SemiMasker should pass through input");

        // Verify mask collected offsets by checking the underlying shared set
        let collected = mask.masked_offsets.lock().unwrap();
        assert!(collected.contains(&10), "Offset 10 should be masked");
        assert!(collected.contains(&20), "Offset 20 should be masked");
        assert!(collected.contains(&30), "Offset 30 should be masked");
        assert!(!collected.contains(&40), "Offset 40 should NOT be masked");
    }

    #[test]
    fn test_scan_with_semi_mask() {
        // Create a semi-mask with offsets 1, 3 (only allow these)
        let mask = NodeSemiMask::new(0);
        mask.mask(1);
        mask.mask(3);
        mask.finalize();

        // Create scan with 4 rows: offsets 0..3
        let mut scan = PhysicalScan::new("test".into(), 0, 10);
        let data = vec![
            vec![
                Value::InternalID(akar_common::types::InternalID { offset: 0, table_id: 0 }),
                Value::InternalID(akar_common::types::InternalID { offset: 1, table_id: 0 }),
                Value::InternalID(akar_common::types::InternalID { offset: 2, table_id: 0 }),
                Value::InternalID(akar_common::types::InternalID { offset: 3, table_id: 0 }),
            ],
            vec![
                Value::Int64(100),
                Value::Int64(200),
                Value::Int64(300),
                Value::Int64(400),
            ],
        ];
        let columns = vec![
            ColumnDefinition { compression: akar_common::enums::CompressionType::Uncompressed,
                name: "id".into(),
                logical_type: LogicalTypeID::InternalID,
                is_primary_key: false,
            },
            ColumnDefinition { compression: akar_common::enums::CompressionType::Uncompressed,
                name: "val".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
            },
        ];
        scan = scan.with_data(data, columns);
        scan = scan.with_semi_mask(mask, 0); // mask on column 0 (InternalID)

        let result = scan.execute(vec![]).unwrap();
        assert_eq!(result.len(), 1, "Should produce one chunk");
        assert_eq!(result[0].size, 2, "Should have 2 rows (offsets 1 and 3)");

        // Verify the values
        let val_field = &result[0].fields[1];
        assert_eq!(val_field.get_value(0), Some(Value::Int64(200)));
        assert_eq!(val_field.get_value(1), Some(Value::Int64(400)));
    }

    #[test]
    fn test_semi_mask_uninitialized_passes_all() {
        // An uninitialized mask should pass all rows (initialized = false)
        let mask = NodeSemiMask::new(0);
        // Don't call finalize G�� mask is not initialized

        assert!(mask.is_masked(999), "Uninitialized mask should pass all offsets");
    }

    #[test]
    fn test_hash_join_with_semi_mask_collects_build_keys() {
        // When a PhysicalHashJoin has a semi_mask, build-side keys are collected
        let mask = NodeSemiMask::new(0);
        let join = PhysicalHashJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
            semi_mask: Some(mask.clone()),
        };

        // Build side with Int64 keys
        let mut build_v = ValueVector::new(PhysicalTypeID::Int64, 3);
        build_v.set_i64(0, 5);
        build_v.set_i64(1, 15);
        build_v.set_i64(2, 25);
        build_v.resize(3);

        // Probe side
        let mut probe_v = ValueVector::new(PhysicalTypeID::Int64, 3);
        probe_v.set_i64(0, 5);
        probe_v.set_i64(1, 15);
        probe_v.set_i64(2, 35);
        probe_v.resize(3);

        let build_chunk = DataChunk::new(vec![build_v]);
        let probe_chunk = DataChunk::new(vec![probe_v]);
        let result = join.execute_binary(&[build_chunk], &[probe_chunk]).unwrap();

        // Should match 5G��5 and 15G��15 (2 rows). 35 has no build match.
        assert!(!result.is_empty(), "Expected 2 matching rows");

        // Verify mask collected build-side keys via underlying shared set
        let collected = mask.masked_offsets.lock().unwrap();
        assert!(collected.contains(&5), "Offset 5 should be in mask");
        assert!(collected.contains(&15), "Offset 15 should be in mask");
        assert!(collected.contains(&25), "Offset 25 should be in mask");
    }

    #[test]
    fn test_order_by_with_nulls() {
        // NULLs should sort last (ASC)
        let order = PhysicalOrderBy {
            sort_keys: vec![(0, true)],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        v.set_i64(0, 3);
        v.set_null(1, true); // NULL
        v.set_i64(2, 1);
        v.set_i64(3, 2);
        v.set_null(4, true); // NULL
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = order.execute(input).unwrap();
        assert!(!result.is_empty());
        // First three should be 1, 2, 3 (sorted ascending)
        assert_eq!(result[0].fields[0].get_i64(0).unwrap(), 1);
        assert_eq!(result[0].fields[0].get_i64(1).unwrap(), 2);
        assert_eq!(result[0].fields[0].get_i64(2).unwrap(), 3);
        // Last two should be NULL
        assert!(result[0].fields[0].is_null(3));
        assert!(result[0].fields[0].is_null(4));
    }

    #[test]
    fn test_limit_zero() {
        // LIMIT 0 should return empty result
        let limit = PhysicalLimit { limit: 0, offset: 0 };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v.set_i64(i, i as i64);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = limit.execute(input).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_limit_offset_exceeds_total() {
        // OFFSET larger than total rows G�� empty result
        let limit = PhysicalLimit { limit: 5, offset: 100 };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v.set_i64(i, i as i64);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = limit.execute(input).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_aggregate_count_with_nulls() {
        // COUNT should NOT count NULL values (standard SQL semantics)
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["COUNT".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        v.set_i64(0, 10);
        v.set_null(1, true);
        v.set_i64(2, 20);
        v.set_null(3, true);
        v.set_i64(4, 30);
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        // COUNT of [10, NULL, 20, NULL, 30] = 3
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(3));
    }

    #[test]
    fn test_aggregate_sum_with_nulls() {
        // SUM should skip NULLs
        let agg = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["SUM".into()],
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        v.set_i64(0, 10);
        v.set_null(1, true);
        v.set_i64(2, 20);
        v.set_null(3, true);
        v.set_i64(4, 30);
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = agg.execute(input).unwrap();
        // SUM of [10, NULL, 20, NULL, 30] = 60
        assert_eq!(result[0].fields[0].get_value(0).unwrap(), Value::Int64(60));
    }

    #[test]
    fn test_aggregate_group_by_with_nulls() {
        // GROUP BY with NULL keys: NULLs should group together
        let agg = PhysicalAggregate {
            group_by_cols: vec![0],
            aggregate_functions: vec!["COUNT".into()],
        };
        let n = 6;
        let mut keys = ValueVector::new(PhysicalTypeID::Int64, n);
        keys.set_i64(0, 1);
        keys.set_i64(1, 1);
        keys.set_null(2, true);
        keys.set_null(3, true);
        keys.set_i64(4, 2);
        keys.set_i64(5, 2);
        keys.resize(n);
        let mut vals = ValueVector::new(PhysicalTypeID::Int64, n);
        for i in 0..n {
            vals.set_i64(i, i as i64);
        }
        vals.resize(n);
        let input = vec![DataChunk::new(vec![keys, vals])];
        let result = agg.execute(input).unwrap();
        assert!(!result.is_empty());
        // Result should have 3 groups: key=1 (count=2), key=NULL (count=2), key=2 (count=2)
        assert_eq!(result[0].size, 3);
    }

    #[test]
    fn test_filter_with_nulls() {
        // Filter should treat NULL as false
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 4);
        v.set_i64(0, 1);
        v.set_null(1, true);
        v.set_i64(2, 3);
        v.set_i64(3, 4);
        v.resize(4);
        let input = vec![DataChunk::new(vec![v])];

        // Variable filter on first field: non-null rows pass
        let filter = PhysicalFilter::new(Expression::Variable("a".into()));
        let result = filter.execute(input.clone()).unwrap();
        assert!(!result.is_empty());
        assert_eq!(result[0].size, 3); // 3 non-null rows pass
    }

    #[test]
    fn test_empty_table_scan() {
        // Scan of an empty table should return empty result, not error
        let scan = PhysicalScan::new("EmptyTable".into(), 0, 0);
        let result = scan.execute(vec![]).unwrap();
        // Should return a valid empty DataChunk
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 0);
    }

    #[test]
    fn test_empty_input_through_pipeline() {
        // Empty input should produce empty output (no rows to process)
        let filter = PhysicalFilter::new(Expression::Constant(Constant::Bool(true)));
        let result = filter.execute(vec![DataChunk::new(vec![])]).unwrap();
        // Filter with 0 rows produces 0 output chunks (nothing to filter)
        assert!(result.is_empty());
    }

    // ==================== UNION Tests ====================

    fn make_i64_chunk(values: &[i64]) -> DataChunk {
        let mut v = ValueVector::new(PhysicalTypeID::Int64, values.len().max(1));
        for (i, val) in values.iter().enumerate() {
            v.set_i64(i, *val);
        }
        v.resize(values.len());
        DataChunk::new(vec![v])
    }

    #[test]
    fn test_union_all_basic() {
        // UNION ALL: two single-column Int64 vectors concatenated
        let mut left_v = ValueVector::new(PhysicalTypeID::Int64, 3);
        left_v.set_i64(0, 1);
        left_v.set_i64(1, 2);
        left_v.set_i64(2, 3);
        left_v.resize(3);
        let left_data = vec![DataChunk::new(vec![left_v])];
        let mut right_v = ValueVector::new(PhysicalTypeID::Int64, 2);
        right_v.set_i64(0, 4);
        right_v.set_i64(1, 5);
        right_v.resize(2);
        let right_data = vec![DataChunk::new(vec![right_v])];
        let result = merge_union_chunks(left_data, right_data, true).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 5);
        // Verify values in order
        assert_eq!(result[0].field(0).get_i64(0), Some(1));
        assert_eq!(result[0].field(0).get_i64(1), Some(2));
        assert_eq!(result[0].field(0).get_i64(2), Some(3));
        assert_eq!(result[0].field(0).get_i64(3), Some(4));
        assert_eq!(result[0].field(0).get_i64(4), Some(5));
    }

    #[test]
    fn test_union_all_multiple_chunks() {
        // UNION ALL: multiple chunks per side
        let mut v1 = ValueVector::new(PhysicalTypeID::Int64, 2);
        v1.set_i64(0, 1);
        v1.set_i64(1, 2);
        v1.resize(2);
        let mut v2 = ValueVector::new(PhysicalTypeID::Int64, 1);
        v2.set_i64(0, 3);
        v2.resize(1);
        let left = vec![DataChunk::new(vec![v1]), DataChunk::new(vec![v2])];
        let mut rv = ValueVector::new(PhysicalTypeID::Int64, 2);
        rv.set_i64(0, 4);
        rv.set_i64(1, 5);
        rv.resize(2);
        let right = vec![DataChunk::new(vec![rv])];
        let result = merge_union_chunks(left, right, true).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 5);
        assert_eq!(result[0].field(0).get_i64(0), Some(1));
        assert_eq!(result[0].field(0).get_i64(4), Some(5));
    }

    #[test]
    fn test_union_distinct_dedup() {
        // UNION (distinct): duplicates removed
        let mut lv = ValueVector::new(PhysicalTypeID::Int64, 3);
        lv.set_i64(0, 1);
        lv.set_i64(1, 2);
        lv.set_i64(2, 3);
        lv.resize(3);
        let left = vec![DataChunk::new(vec![lv])];
        let mut rv = ValueVector::new(PhysicalTypeID::Int64, 3);
        rv.set_i64(0, 2);
        rv.set_i64(1, 3);
        rv.set_i64(2, 4);
        rv.resize(3);
        let right = vec![DataChunk::new(vec![rv])];
        let result = merge_union_chunks(left, right, false).unwrap();
        assert_eq!(result.len(), 1);
        // Distinct values: {1, 2, 3, 4} G�� 4 rows
        assert_eq!(result[0].size, 4);
    }

    #[test]
    fn test_union_column_mismatch() {
        // Column count mismatch should produce an error
        let left = vec![DataChunk::new(vec![
            ValueVector::new(PhysicalTypeID::Int64, 1),
            ValueVector::new(PhysicalTypeID::Int64, 1),
        ])];
        let right = vec![DataChunk::new(vec![ValueVector::new(PhysicalTypeID::Int64, 1)])];
        let result = merge_union_chunks(left, right, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("column count mismatch"));
    }

    #[test]
    fn test_union_empty_left() {
        let left = vec![];
        let mut rv = ValueVector::new(PhysicalTypeID::Int64, 2);
        rv.set_i64(0, 42);
        rv.set_i64(1, 43);
        rv.resize(2);
        let right = vec![DataChunk::new(vec![rv])];
        let result = merge_union_chunks(left, right, true).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 2);
        assert_eq!(result[0].field(0).get_i64(0), Some(42));
    }

    #[test]
    fn test_union_empty_right() {
        let mut lv = ValueVector::new(PhysicalTypeID::Int64, 2);
        lv.set_i64(0, 99);
        lv.set_i64(1, 100);
        lv.resize(2);
        let left = vec![DataChunk::new(vec![lv])];
        let right = vec![];
        let result = merge_union_chunks(left, right, true).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 2);
    }

    #[test]
    fn test_union_all_multi_column() {
        // UNION ALL with multiple columns
        let mut left_v1 = ValueVector::new(PhysicalTypeID::Int64, 2);
        left_v1.set_i64(0, 1);
        left_v1.set_i64(1, 2);
        left_v1.resize(2);
        let mut left_v2 = ValueVector::new(PhysicalTypeID::String, 2);
        left_v2.push_string("hello");
        left_v2.push_string("world");
        let left = vec![DataChunk::new(vec![left_v1, left_v2])];

        let mut right_v1 = ValueVector::new(PhysicalTypeID::Int64, 1);
        right_v1.set_i64(0, 3);
        right_v1.resize(1);
        let mut right_v2 = ValueVector::new(PhysicalTypeID::String, 1);
        right_v2.push_string("foo");
        let right = vec![DataChunk::new(vec![right_v1, right_v2])];

        let result = merge_union_chunks(left, right, true).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 3);
        assert_eq!(result[0].field(0).get_i64(0), Some(1));
        assert_eq!(result[0].field(0).get_i64(1), Some(2));
        assert_eq!(result[0].field(0).get_i64(2), Some(3));
    }

    #[test]
    fn test_union_distinct_all_duplicates() {
        // All rows identical G�� single row after dedup
        let mut lv = ValueVector::new(PhysicalTypeID::Int64, 2);
        lv.set_i64(0, 1);
        lv.set_i64(1, 1);
        lv.resize(2);
        let left = vec![DataChunk::new(vec![lv])];
        let mut rv = ValueVector::new(PhysicalTypeID::Int64, 2);
        rv.set_i64(0, 1);
        rv.set_i64(1, 1);
        rv.resize(2);
        let right = vec![DataChunk::new(vec![rv])];
        let result = merge_union_chunks(left, right, false).unwrap();
        assert_eq!(result[0].size, 1);
        assert_eq!(result[0].field(0).get_i64(0), Some(1));
    }

    #[test]
    fn test_union_all_empty_chunks() {
        // Empty DataChunks should be handled gracefully
        let empty = ValueVector::new(PhysicalTypeID::Int64, 0);
        let left = vec![DataChunk::new(vec![empty])];
        let mut rv = ValueVector::new(PhysicalTypeID::Int64, 1);
        rv.set_i64(0, 42);
        rv.resize(1);
        let right = vec![DataChunk::new(vec![rv])];
        let result = merge_union_chunks(left, right, true).unwrap();
        assert_eq!(result[0].size, 1);
        assert_eq!(result[0].field(0).get_i64(0), Some(42));
    }

    // ==================== CrossProduct Tests ====================

    #[test]
    fn test_cross_product_basic() {
        let cross = PhysicalCrossProduct;
        // Left: [1, 2, 3], Right: [4, 5]
        let left = vec![make_i64_chunk(&[1, 2, 3])];
        let right = vec![make_i64_chunk(&[4, 5])];
        let result = cross.execute_binary(&left, &right).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 6); // 3 +� 2 = 6
        assert_eq!(result[0].field(0).get_i64(0), Some(1));
        assert_eq!(result[0].field(0).get_i64(1), Some(1));
        assert_eq!(result[0].field(0).get_i64(2), Some(2));
        assert_eq!(result[0].field(0).get_i64(3), Some(2));
        assert_eq!(result[0].field(0).get_i64(4), Some(3));
        assert_eq!(result[0].field(0).get_i64(5), Some(3));
    }

    #[test]
    fn test_cross_product_multi_column() {
        let cross = PhysicalCrossProduct;
        // Left: [{1, "a"}, {2, "b"}], Right: [{10}, {20}]
        let mut l1 = ValueVector::new(PhysicalTypeID::Int64, 2);
        l1.set_i64(0, 1);
        l1.set_i64(1, 2);
        l1.resize(2);
        let mut l2 = ValueVector::new(PhysicalTypeID::String, 2);
        l2.push_string("a");
        l2.push_string("b");
        let left = DataChunk::new(vec![l1, l2]);

        let mut r1 = ValueVector::new(PhysicalTypeID::Int64, 2);
        r1.set_i64(0, 10);
        r1.set_i64(1, 20);
        r1.resize(2);
        let right = DataChunk::new(vec![r1]);

        let result = cross.execute_binary(&[left], &[right]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 4); // 2 +� 2 = 4
        // Row 0: left(1,"a") +� right(10) G�� [1, "a", 10]
        assert_eq!(result[0].field(0).get_i64(0), Some(1));
        // Row 1: left(1,"a") +� right(20) G�� [1, "a", 20]
        assert_eq!(result[0].field(0).get_i64(1), Some(1));
        // Row 2: left(2,"b") +� right(10) G�� [2, "b", 10]
        assert_eq!(result[0].field(0).get_i64(2), Some(2));
        // Row 3: left(2,"b") +� right(20) G�� [2, "b", 20]
        assert_eq!(result[0].field(0).get_i64(3), Some(2));
        // Column 2 should have right-side values: [10, 20, 10, 20]
        assert_eq!(result[0].field(2).get_i64(0), Some(10));
        assert_eq!(result[0].field(2).get_i64(1), Some(20));
        assert_eq!(result[0].field(2).get_i64(2), Some(10));
        assert_eq!(result[0].field(2).get_i64(3), Some(20));
    }

    #[test]
    fn test_cross_product_empty_left() {
        let cross = PhysicalCrossProduct;
        let left = make_i64_chunk(&[]);
        let right = make_i64_chunk(&[1, 2]);
        let result = cross.execute_binary(&[left], &[right]).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_cross_product_empty_right() {
        let cross = PhysicalCrossProduct;
        let left = make_i64_chunk(&[1, 2, 3]);
        let right = make_i64_chunk(&[]);
        let result = cross.execute_binary(&[left], &[right]).unwrap();
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_cross_product_multi_chunk() {
        let cross = PhysicalCrossProduct;
        // Left: two chunks [1,2] and [3]
        let left = vec![make_i64_chunk(&[1, 2]), make_i64_chunk(&[3])];
        // Right: one chunk [4,5]
        let right = vec![make_i64_chunk(&[4, 5])];
        let result = cross.execute_binary(&left, &right).unwrap();
        assert_eq!(result[0].size, 6); // 3 +� 2 = 6
    }

    // ==================== SemiJoin / AntiJoin Tests ====================

    #[test]
    fn test_semi_join_basic() {
        let semi = PhysicalSemiJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
        };
        // Build (right): [2, 3]
        let build = make_i64_chunk(&[2, 3]);
        // Probe (left): [1, 2, 3]
        let probe = make_i64_chunk(&[1, 2, 3]);
        let result = semi.execute_binary(&[build], &[probe]).unwrap();
        assert_eq!(result[0].size, 2); // [2, 3] match
    }

    #[test]
    fn test_semi_join_no_match() {
        let semi = PhysicalSemiJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
        };
        let build = make_i64_chunk(&[4, 5]);
        let probe = make_i64_chunk(&[1, 2, 3]);
        let result = semi.execute_binary(&[build], &[probe]).unwrap();
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_anti_join_basic() {
        let anti = PhysicalAntiJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
        };
        // Build (right): [2, 3]
        let build = make_i64_chunk(&[2, 3]);
        // Probe (left): [1, 2, 3]
        let probe = make_i64_chunk(&[1, 2, 3]);
        let result = anti.execute_binary(&[build], &[probe]).unwrap();
        assert_eq!(result[0].size, 1); // Only [1] has no match
    }

    #[test]
    fn test_anti_join_all_match() {
        let anti = PhysicalAntiJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
        };
        let build = make_i64_chunk(&[1, 2, 3]);
        let probe = make_i64_chunk(&[1, 2, 3]);
        let result = anti.execute_binary(&[build], &[probe]).unwrap();
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_semi_join_empty_build() {
        let semi = PhysicalSemiJoin {
            build_columns: vec![0],
            probe_columns: vec![0],
        };
        let build = make_i64_chunk(&[]);
        let probe = make_i64_chunk(&[1, 2, 3]);
        let result = semi.execute_binary(&[build], &[probe]).unwrap();
        assert!(result.is_empty() || result[0].size == 0);
    }

    // --- Intersect tests ---

    #[test]
    fn test_intersect_basic() {
        let intersect = PhysicalIntersect {
            num_build_sides: 2,
            probe_key_col: 0,
            build_key_col: 0,
        };
        // Two build sides with overlapping keys
        let build1 = make_i64_chunk(&[1, 2, 3]);
        let build2 = make_i64_chunk(&[2, 3, 4]);
        // Probe with keys that should match across both builds
        let probe = make_i64_chunk(&[2, 3]);
        let build_chunks = vec![build1, build2];
        let probe_chunks = vec![probe];
        let result = intersect.execute_binary(&build_chunks, &probe_chunks).unwrap();
        // Keys 2 and 3 exist in both build sides G�� both should produce output
        assert!(!result.is_empty(), "Expected non-empty result");
        assert!(result[0].size > 0, "Expected at least one output row");
    }

    #[test]
    fn test_intersect_no_common() {
        let intersect = PhysicalIntersect {
            num_build_sides: 2,
            probe_key_col: 0,
            build_key_col: 0,
        };
        // Build sides have no overlapping keys
        let build1 = make_i64_chunk(&[1, 2, 3]);
        let build2 = make_i64_chunk(&[4, 5, 6]);
        let probe = make_i64_chunk(&[1, 2, 3, 4, 5, 6]);
        let build_chunks = vec![build1, build2];
        let probe_chunks = vec![probe];
        let result = intersect.execute_binary(&build_chunks, &probe_chunks).unwrap();
        // No key appears in ALL build sides G�� empty
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_intersect_probe_key_missing() {
        let intersect = PhysicalIntersect {
            num_build_sides: 2,
            probe_key_col: 0,
            build_key_col: 0,
        };
        // Build sides share key 3, but probe doesn't probe for 3
        let build1 = make_i64_chunk(&[1, 3]);
        let build2 = make_i64_chunk(&[3, 5]);
        let probe = make_i64_chunk(&[1, 5]); // probes for 1 and 5 G�� only 1 is in build1, only 5 is in build2
        let build_chunks = vec![build1, build2];
        let probe_chunks = vec![probe];
        let result = intersect.execute_binary(&build_chunks, &probe_chunks).unwrap();
        // No key appears in ALL build sides G�� empty (1 not in build2, 5 not in build1)
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_intersect_single_build_side() {
        let intersect = PhysicalIntersect {
            num_build_sides: 1,
            probe_key_col: 0,
            build_key_col: 0,
        };
        // Single build side G�� acts like semi-join
        let build = make_i64_chunk(&[2, 3]);
        let probe = make_i64_chunk(&[1, 2, 3, 4]);
        let build_chunks = vec![build];
        let probe_chunks = vec![probe];
        let result = intersect.execute_binary(&build_chunks, &probe_chunks).unwrap();
        assert!(!result.is_empty(), "Expected non-empty result for single build side");
        assert!(result[0].size > 0, "Expected matching rows");
    }

    #[test]
    fn test_intersect_empty_build() {
        let intersect = PhysicalIntersect {
            num_build_sides: 2,
            probe_key_col: 0,
            build_key_col: 0,
        };
        let build1 = make_i64_chunk(&[]);
        let build2 = make_i64_chunk(&[1, 2, 3]);
        let probe = make_i64_chunk(&[1, 2, 3]);
        let build_chunks = vec![build1, build2];
        let probe_chunks = vec![probe];
        let result = intersect.execute_binary(&build_chunks, &probe_chunks).unwrap();
        // Empty build side G�� empty result
        assert!(result.is_empty() || result[0].size == 0);
    }

    #[test]
    fn test_intersect_no_probe() {
        let intersect = PhysicalIntersect {
            num_build_sides: 2,
            probe_key_col: 0,
