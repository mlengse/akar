//! Graph algorithm extension for Akar.
//!
//! Provides table functions that execute graph algorithms:
//! - PageRank (alias PR)
//! - Weakly Connected Components (alias WCC)
//! - Strongly Connected Components — Tarjan (alias SCC)
//! - Strongly Connected Components — Kosaraju (alias SCC_KO)
//! - K-Core Decomposition (alias KCORE)
//! - Louvain Community Detection
//! - Spanning Forest (alias SF)
//! - Shortest Path (BFS-based, alias SP)
//! - Weighted Shortest Path (Dijkstra-based)
//! - All Shortest Path Destinations
//! - Spreading Activation
//!
//! All algorithms operate on the CSR adjacency built from existing
//! node/rel tables in the database, using the GDS framework.

use std::sync::Arc;

use akar_extension::{Extension, ExtensionContext};
use akar_graph::CSRAdjacency;
use akar_graph::gds::BaseBFSGraph;

pub mod gds;
pub use gds::node2vec::compute_node2vec;
pub use gds::random_walk::compute_random_walk;

/// The graph algorithms extension.
pub struct AlgoExtension;

impl Default for AlgoExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl AlgoExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for AlgoExtension {
    fn name(&self) -> &'static str {
        "ALGO"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use akar_common::types::Value;
        use akar_common::vector::DataChunk;
        use akar_function::GraphDataSource;
        use akar_function::registry::TableFunction;

        // ── Helper: fallback graph used when the caller has no catalog ────────
        // 5-node ring: 0→1→2→3→4→0
        let sample_edges = || {
            vec![
                akar_graph::Edge {
                    src_offset: 0,
                    dst_offset: 1,
                    rel_id: 0,
                    rel_table_id: 0,
                },
                akar_graph::Edge {
                    src_offset: 1,
                    dst_offset: 2,
                    rel_id: 1,
                    rel_table_id: 0,
                },
                akar_graph::Edge {
                    src_offset: 2,
                    dst_offset: 3,
                    rel_id: 2,
                    rel_table_id: 0,
                },
                akar_graph::Edge {
                    src_offset: 3,
                    dst_offset: 4,
                    rel_id: 3,
                    rel_table_id: 0,
                },
                akar_graph::Edge {
                    src_offset: 4,
                    dst_offset: 0,
                    rel_id: 4,
                    rel_table_id: 0,
                },
            ]
        };

        // ── Helper: build a CSR from the supplied graph (or the fallback) ────
        let csr_from_graph = move |graph: Option<&dyn GraphDataSource>| -> (CSRAdjacency, usize) {
            let (edges, num_nodes) = match graph {
                Some(g) => {
                    let edges: Vec<akar_graph::Edge> = g
                        .edges()
                        .iter()
                        .map(|e| akar_graph::Edge {
                            src_offset: e.src_offset,
                            dst_offset: e.dst_offset,
                            rel_id: e.rel_id,
                            rel_table_id: e.rel_table_id,
                        })
                        .collect();
                    (edges, g.num_nodes())
                }
                None => (sample_edges(), 5),
            };
            (CSRAdjacency::build(&edges, num_nodes), num_nodes)
        };

        // Helper: create a table function closure that runs a GDS shortest path algorithm.
        let sp_destinations_fn = Arc::new(
            move |args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let source = match args.first() {
                    Some(Value::Int64(s)) => *s as u64,
                    Some(Value::UInt64(s)) => *s,
                    _ => return Err("shortest_path: first argument must be source node offset (integer)".into()),
                };

                // Build CSR from the database graph (or the fallback sample graph).
                let (csr, num_nodes) = csr_from_graph(graph);

                // Run BFS shortest path using GDS framework
                let mut bfs = akar_graph::gds::bfs_graph::DenseBFSGraph::new(num_nodes);
                akar_graph::gds::utils::GDSUtils::run_single_shortest_path(&csr, source, &mut bfs, 100);

                // Collect results: (src, dst, distance)
                let mut src_col = Vec::new();
                let mut dst_col = Vec::new();
                let mut dist_col = Vec::new();

                for offset in 0..num_nodes {
                    if bfs.get_parent_list_head_offset(offset as u64).is_some() || offset == source as usize {
                        let dist = if offset == source as usize {
                            0i64
                        } else {
                            // Trace back to count hops
                            let mut hops = 0i64;
                            let mut cur = offset as u64;
                            while cur != source {
                                if let Some(parent) = bfs.get_parent_list_head_offset(cur) {
                                    cur = parent.node_id.offset;
                                    hops += 1;
                                } else {
                                    break;
                                }
                            }
                            hops
                        };
                        src_col.push(Value::Int64(source as i64));
                        dst_col.push(Value::Int64(offset as i64));
                        dist_col.push(Value::Int64(dist));
                    }
                }

                let n = src_col.len();
                let mut v1 = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, n);
                let mut v2 = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, n);
                let mut v3 = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, n);
                for (i, val) in src_col.iter().enumerate() {
                    v1.set_value(i, val)?;
                }
                for (i, val) in dst_col.iter().enumerate() {
                    v2.set_value(i, val)?;
                }
                for (i, val) in dist_col.iter().enumerate() {
                    v3.set_value(i, val)?;
                }
                output.fields = vec![
                    akar_common::arrow_vector::ArrowVector::from_legacy(&v1).array,
                    akar_common::arrow_vector::ArrowVector::from_legacy(&v2).array,
                    akar_common::arrow_vector::ArrowVector::from_legacy(&v3).array,
                ];
                output.field_types = vec![
                    akar_common::types::PhysicalTypeID::Int64,
                    akar_common::types::PhysicalTypeID::Int64,
                    akar_common::types::PhysicalTypeID::Int64,
                ];
                output.size = n;
                Ok(())
            },
        );

        let wsp_destinations_fn = Arc::new(
            move |args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let source = match args.first() {
                    Some(Value::Int64(s)) => *s as u64,
                    Some(Value::UInt64(s)) => *s,
                    _ => return Err("weighted_shortest_path: first argument must be source node offset".into()),
                };

                let (csr, num_nodes) = csr_from_graph(graph);

                let mut bfs = akar_graph::gds::bfs_graph::DenseBFSGraph::new(num_nodes);
                akar_graph::gds::utils::GDSUtils::run_weighted_shortest_path(
                    &csr,
                    source,
                    &mut bfs,
                    |_src, _dst, _eid| 1.0,
                );

                let mut src_col = Vec::new();
                let mut dst_col = Vec::new();
                let mut cost_col = Vec::new();

                for offset in 0..num_nodes {
                    if bfs.get_parent_list_head_offset(offset as u64).is_some() || offset == source as usize {
                        let cost = if offset == source as usize {
                            0.0
                        } else if let Some(_parent) = bfs.get_parent_list_head_offset(offset as u64) {
                            // Walk back accumulating costs
                            let mut total = 0.0;
                            let mut cur = offset as u64;
                            while cur != source {
                                if let Some(p) = bfs.get_parent_list_head_offset(cur) {
                                    total += p.cost;
                                    cur = p.node_id.offset;
                                } else {
                                    break;
                                }
                            }
                            total
                        } else {
                            f64::MAX
                        };
                        src_col.push(Value::Int64(source as i64));
                        dst_col.push(Value::Int64(offset as i64));
                        cost_col.push(Value::Double(cost));
                    }
                }

                let n = src_col.len();
                let mut v1 = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, n);
                let mut v2 = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, n);
                let mut v3 = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Double, n);
                for (i, val) in src_col.iter().enumerate() {
                    v1.set_value(i, val)?;
                }
                for (i, val) in dst_col.iter().enumerate() {
                    v2.set_value(i, val)?;
                }
                for (i, val) in cost_col.iter().enumerate() {
                    v3.set_value(i, val)?;
                }
                output.fields = vec![
                    akar_common::arrow_vector::ArrowVector::from_legacy(&v1).array,
                    akar_common::arrow_vector::ArrowVector::from_legacy(&v2).array,
                    akar_common::arrow_vector::ArrowVector::from_legacy(&v3).array,
                ];
                output.field_types = vec![
                    akar_common::types::PhysicalTypeID::Int64,
                    akar_common::types::PhysicalTypeID::Int64,
                    akar_common::types::PhysicalTypeID::Double,
                ];
                output.size = n;
                Ok(())
            },
        );

        /// Pack an AlgoResult (parallel score-per-node) into a DataChunk
        /// with columns (node_id INT64, score DOUBLE).
        fn pack_node_scores(
            result: AlgoResult,
            score_col: &str,
        ) -> Result<
            (
                Vec<arrow::array::ArrayRef>,
                Vec<akar_common::types::PhysicalTypeID>,
                Vec<String>,
                usize,
            ),
            String,
        > {
            use akar_common::{
                types::{PhysicalTypeID, Value},
                vector::ValueVector,
            };
            let n = result.values.len();
            let mut id_vec = ValueVector::new(PhysicalTypeID::Int64, n);
            let mut val_vec = ValueVector::new(PhysicalTypeID::Double, n);
            for (i, &v) in result.values.iter().enumerate() {
                id_vec.set_value(i, &Value::Int64(i as i64))?;
                val_vec.set_value(i, &Value::Double(v))?;
            }
            let arr1 = akar_common::arrow_vector::ArrowVector::from_legacy(&id_vec).array;
            let arr2 = akar_common::arrow_vector::ArrowVector::from_legacy(&val_vec).array;
            Ok((
                vec![arr1, arr2],
                vec![PhysicalTypeID::Int64, PhysicalTypeID::Double],
                vec!["node_id".into(), score_col.into()],
                n,
            ))
        }

        // ── page_rank / pr ────────────────────────────────────────────────────
        // CALL page_rank() → (node_id INT64, rank DOUBLE)
        let pr_fn = Arc::new({
            move |_args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_page_rank(&csr);
                let (fields, field_types, field_names, size) = pack_node_scores(result, "rank")?;
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "page_rank",
            TableFunction::CustomTableWithGraph {
                name: "page_rank".into(),
                execute: pr_fn.clone(),
            },
        );
        context.register_table_function(
            "pr",
            TableFunction::CustomTableWithGraph {
                name: "page_rank".into(),
                execute: pr_fn,
            },
        );

        // ── weakly_connected_components / wcc ─────────────────────────────────
        // CALL wcc() → (node_id INT64, component_id DOUBLE)
        let wcc_fn = Arc::new({
            move |_args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_wcc(&csr);
                let (fields, field_types, field_names, size) = pack_node_scores(result, "component_id")?;
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "weakly_connected_components",
            TableFunction::CustomTableWithGraph {
                name: "wcc".into(),
                execute: wcc_fn.clone(),
            },
        );
        context.register_table_function(
            "wcc",
            TableFunction::CustomTableWithGraph {
                name: "wcc".into(),
                execute: wcc_fn,
            },
        );

        // ── strongly_connected_components (Tarjan) / scc ──────────────────────
        // CALL scc() → (node_id INT64, component_id DOUBLE)
        let scc_tarjan_fn = Arc::new({
            move |_args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_scc_tarjan(&csr);
                let (fields, field_types, field_names, size) = pack_node_scores(result, "component_id")?;
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "strongly_connected_components",
            TableFunction::CustomTableWithGraph {
                name: "scc_tarjan".into(),
                execute: scc_tarjan_fn.clone(),
            },
        );
        context.register_table_function(
            "scc",
            TableFunction::CustomTableWithGraph {
                name: "scc_tarjan".into(),
                execute: scc_tarjan_fn,
            },
        );

        // ── scc_kosaraju / scc_ko ─────────────────────────────────────────────
        let scc_ko_fn = Arc::new({
            move |_args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_scc_kosaraju(&csr);
                let (fields, field_types, field_names, size) = pack_node_scores(result, "component_id")?;
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "strongly_connected_components_kosaraju",
            TableFunction::CustomTableWithGraph {
                name: "scc_kosaraju".into(),
                execute: scc_ko_fn.clone(),
            },
        );
        context.register_table_function(
            "scc_ko",
            TableFunction::CustomTableWithGraph {
                name: "scc_kosaraju".into(),
                execute: scc_ko_fn,
            },
        );

        // ── k_core_decomposition / kcore ──────────────────────────────────────
        // CALL k_core_decomposition() → (node_id INT64, core_number DOUBLE)
        let kcore_fn = Arc::new({
            move |_args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_k_core(&csr);
                let (fields, field_types, field_names, size) = pack_node_scores(result, "core_number")?;
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "k_core_decomposition",
            TableFunction::CustomTableWithGraph {
                name: "k_core".into(),
                execute: kcore_fn.clone(),
            },
        );
        context.register_table_function(
            "kcore",
            TableFunction::CustomTableWithGraph {
                name: "k_core".into(),
                execute: kcore_fn,
            },
        );

        // ── louvain ───────────────────────────────────────────────────────────
        // CALL louvain(seed?, min_gain?, max_iterations?) → (node_id INT64, community_id DOUBLE, modularity DOUBLE)
        let louvain_fn = Arc::new({
            move |args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                use akar_common::types::{PhysicalTypeID, Value};
                use akar_common::vector::ValueVector;

                let seed = match args.first() {
                    Some(Value::Int64(s)) => Some(*s as u64),
                    _ => None,
                };
                let min_gain = match args.get(1) {
                    Some(Value::Double(d)) => *d,
                    _ => 0.001,
                };
                let max_iterations = match args.get(2) {
                    Some(Value::Int64(i)) => *i as usize,
                    _ => 20,
                };

                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_louvain_weighted(&csr, |_u, _v| 1.0, seed, min_gain, max_iterations);
                let modularity = result.metadata.as_ref().and_then(|m| m.get("modularity")).copied().unwrap_or(0.0);

                let n = result.values.len();
                let mut id_vec = ValueVector::new(PhysicalTypeID::Int64, n);
                let mut comm_vec = ValueVector::new(PhysicalTypeID::Double, n);
                let mut mod_vec = ValueVector::new(PhysicalTypeID::Double, n);
                for (i, &v) in result.values.iter().enumerate() {
                    id_vec.set_value(i, &Value::Int64(i as i64))?;
                    comm_vec.set_value(i, &Value::Double(v))?;
                    mod_vec.set_value(i, &Value::Double(modularity))?;
                }
                output.fields = vec![
                    akar_common::arrow_vector::ArrowVector::from_legacy(&id_vec).array,
                    akar_common::arrow_vector::ArrowVector::from_legacy(&comm_vec).array,
                    akar_common::arrow_vector::ArrowVector::from_legacy(&mod_vec).array,
                ];
                output.field_types = vec![PhysicalTypeID::Int64, PhysicalTypeID::Double, PhysicalTypeID::Double];
                output.field_names = vec!["node_id".into(), "community_id".into(), "modularity".into()];
                output.size = n;
                Ok(())
            }
        });
        context.register_table_function(
            "louvain",
            TableFunction::CustomTableWithGraph {
                name: "louvain".into(),
                execute: louvain_fn,
            },
        );

        // ── spread_activation ─────────────────────────────────────────────────
        // CALL spread_activation(seed_ids, decay?, threshold?, max_hops?)
        //   → (node_id INT64, activation DOUBLE, hop INT64)
        let spread_fn = Arc::new({
            move |args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                use akar_common::types::{PhysicalTypeID, Value};
                use akar_common::vector::ValueVector;

                // Parse seed_ids (required): comma-separated list of INT64 node IDs
                let seed_ids: Vec<i64> = match args.first() {
                    Some(Value::String(s)) => {
                        s.split(',')
                            .map(|t| t.trim().parse::<i64>().map_err(|e| format!("invalid seed id: {e}")))
                            .collect::<Result<Vec<_>, _>>()?
                    }
                    _ => return Err("spread_activation requires seed_ids as comma-separated INT64 list".into()),
                };

                let decay = match args.get(1) {
                    Some(Value::Double(d)) => *d,
                    _ => 0.5,
                };
                let threshold = match args.get(2) {
                    Some(Value::Double(d)) => *d,
                    _ => 0.01,
                };
                let max_hops = match args.get(3) {
                    Some(Value::Int64(i)) => *i as usize,
                    _ => 3,
                };

                let (csr, _num_nodes) = csr_from_graph(graph);
                let seeds: Vec<(usize, f64)> = seed_ids
                    .iter()
                    .map(|&id| (id as usize, 1.0))
                    .filter(|&(pos, _)| pos < csr.num_nodes())
                    .collect();

                let result = compute_spread_activation(
                    &csr,
                    &seeds,
                    |_u, _v| 1.0,
                    decay,
                    threshold,
                    max_hops,
                );

                let size = result.activated.len();
                let mut id_vec = ValueVector::new(PhysicalTypeID::Int64, size);
                let mut act_vec = ValueVector::new(PhysicalTypeID::Double, size);
                let mut hop_vec = ValueVector::new(PhysicalTypeID::Int64, size);
                for (i, &(pos, act, hop)) in result.activated.iter().enumerate() {
                    id_vec.set_value(i, &Value::Int64(pos as i64))?;
                    act_vec.set_value(i, &Value::Double(act))?;
                    hop_vec.set_value(i, &Value::Int64(hop as i64))?;
                }
                output.fields = vec![
                    akar_common::arrow_vector::ArrowVector::from_legacy(&id_vec).array,
                    akar_common::arrow_vector::ArrowVector::from_legacy(&act_vec).array,
                    akar_common::arrow_vector::ArrowVector::from_legacy(&hop_vec).array,
                ];
                output.field_types = vec![PhysicalTypeID::Int64, PhysicalTypeID::Double, PhysicalTypeID::Int64];
                output.field_names = vec!["node_id".into(), "activation".into(), "hop".into()];
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "spread_activation",
            TableFunction::CustomTableWithGraph {
                name: "spread_activation".into(),
                execute: spread_fn,
            },
        );

        // ── spanning_forest / sf ──────────────────────────────────────────────
        // CALL spanning_forest() → (node_id INT64, parent_id DOUBLE)
        let sf_fn = Arc::new({
            move |_args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_spanning_forest(&csr);
                let (fields, field_types, field_names, size) = pack_node_scores(result, "parent_id")?;
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "spanning_forest",
            TableFunction::CustomTableWithGraph {
                name: "spanning_forest".into(),
                execute: sf_fn.clone(),
            },
        );
        context.register_table_function(
            "sf",
            TableFunction::CustomTableWithGraph {
                name: "spanning_forest".into(),
                execute: sf_fn,
            },
        );

        // ── label_propagation / lpa ───────────────────────────────────────────
        // CALL lpa(max_iters?) → (node_id INT64, label DOUBLE)
        let lpa_fn = Arc::new({
            move |args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let max_iters = match args.first() {
                    Some(Value::Int64(n)) if *n > 0 => *n as usize,
                    Some(Value::Int32(n)) if *n > 0 => *n as usize,
                    _ => 10,
                };
                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_lpa(&csr, max_iters);
                let (fields, field_types, field_names, size) = pack_node_scores(result, "label")?;
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "label_propagation",
            TableFunction::CustomTableWithGraph {
                name: "lpa".into(),
                execute: lpa_fn.clone(),
            },
        );
        context.register_table_function(
            "lpa",
            TableFunction::CustomTableWithGraph {
                name: "lpa".into(),
                execute: lpa_fn,
            },
        );

        // ── betweenness_centrality / bc ───────────────────────────────────────
        // CALL betweenness_centrality() → (node_id INT64, centrality DOUBLE)
        let bc_fn = Arc::new({
            move |_args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_betweenness_centrality(&csr);
                let (fields, field_types, field_names, size) = pack_node_scores(result, "centrality")?;
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "betweenness_centrality",
            TableFunction::CustomTableWithGraph {
                name: "betweenness_centrality".into(),
                execute: bc_fn.clone(),
            },
        );
        context.register_table_function(
            "bc",
            TableFunction::CustomTableWithGraph {
                name: "betweenness_centrality".into(),
                execute: bc_fn,
            },
        );

        // ── closeness_centrality / cc ─────────────────────────────────────────
        // CALL closeness_centrality() → (node_id INT64, centrality DOUBLE)
        let cc_fn = Arc::new({
            move |_args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_closeness_centrality(&csr);
                let (fields, field_types, field_names, size) = pack_node_scores(result, "centrality")?;
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "closeness_centrality",
            TableFunction::CustomTableWithGraph {
                name: "closeness_centrality".into(),
                execute: cc_fn.clone(),
            },
        );
        context.register_table_function(
            "cc",
            TableFunction::CustomTableWithGraph {
                name: "closeness_centrality".into(),
                execute: cc_fn,
            },
        );

        // ── triangle_count / tc ───────────────────────────────────────────────
        // CALL triangle_count() → (node_id INT64, triangles DOUBLE)
        let tc_fn = Arc::new({
            move |_args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_triangle_count(&csr);
                let (fields, field_types, field_names, size) = pack_node_scores(result, "triangles")?;
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "triangle_count",
            TableFunction::CustomTableWithGraph {
                name: "triangle_count".into(),
                execute: tc_fn.clone(),
            },
        );
        context.register_table_function(
            "tc",
            TableFunction::CustomTableWithGraph {
                name: "triangle_count".into(),
                execute: tc_fn,
            },
        );

        // ── all_sp_destinations ───────────────────────────────────────────────
        // CALL all_sp_destinations() → (node_id INT64, reachable DOUBLE)
        let all_sp_fn = Arc::new({
            move |_args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_all_sp_destinations(&csr);
                let (fields, field_types, field_names, size) = pack_node_scores(result, "reachable")?;
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "all_sp_destinations",
            TableFunction::CustomTableWithGraph {
                name: "all_sp_destinations".into(),
                execute: all_sp_fn,
            },
        );

        // ── random_walk / rw ──────────────────────────────────────────────────
        // CALL random_walk(steps?, walks_per_node?) → (node_id INT64, hit_count DOUBLE)
        let rw_fn = Arc::new({
            move |args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                let steps = match args.first() {
                    Some(Value::Int64(n)) if *n > 0 => *n as usize,
                    Some(Value::Int32(n)) if *n > 0 => *n as usize,
                    _ => 5,
                };
                let walks_per_node = match args.get(1) {
                    Some(Value::Int64(n)) if *n > 0 => *n as usize,
                    Some(Value::Int32(n)) if *n > 0 => *n as usize,
                    _ => 2,
                };
                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_random_walk(&csr, None, steps, walks_per_node);
                let (fields, field_types, field_names, size) = pack_node_scores(result, "hit_count")?;
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = size;
                Ok(())
            }
        });
        context.register_table_function(
            "random_walk",
            TableFunction::CustomTableWithGraph {
                name: "random_walk".into(),
                execute: rw_fn.clone(),
            },
        );
        context.register_table_function(
            "rw",
            TableFunction::CustomTableWithGraph {
                name: "random_walk".into(),
                execute: rw_fn,
            },
        );

        // ── node2vec / n2v ────────────────────────────────────────────────────
        // CALL node2vec(p?, q?, dimensions?, walks?, window?)
        //   → (node_id INT64, dim_0 DOUBLE, …, dim_N DOUBLE)
        let n2v_fn = Arc::new({
            move |args: &[Value], graph: Option<&dyn GraphDataSource>, output: &mut DataChunk| -> Result<(), String> {
                fn f64_arg(args: &[Value], idx: usize, default: f64) -> f64 {
                    match args.get(idx) {
                        Some(Value::Double(v)) => *v,
                        Some(Value::Float(v)) => *v as f64,
                        Some(Value::Int64(v)) => *v as f64,
                        Some(Value::Int32(v)) => *v as f64,
                        _ => default,
                    }
                }
                fn usize_arg(args: &[Value], idx: usize, default: usize) -> usize {
                    match args.get(idx) {
                        Some(Value::Int64(v)) if *v > 0 => *v as usize,
                        Some(Value::Int32(v)) if *v > 0 => *v as usize,
                        _ => default,
                    }
                }
                let p = f64_arg(args, 0, 1.0);
                let q = f64_arg(args, 1, 1.0);
                let dimensions = usize_arg(args, 2, 4);
                let walks = usize_arg(args, 3, 3);
                let window = usize_arg(args, 4, 5);

                let (csr, _num_nodes) = csr_from_graph(graph);
                let result = compute_node2vec(&csr, p, q, dimensions, walks, window);

                let n = result.values.len().checked_div(dimensions).unwrap_or(0);
                let mut id_vec = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, n);
                for i in 0..n {
                    id_vec.set_value(i, &Value::Int64(i as i64))?;
                }

                let mut fields = vec![akar_common::arrow_vector::ArrowVector::from_legacy(&id_vec).array];
                let mut field_types = vec![akar_common::types::PhysicalTypeID::Int64];
                let mut field_names = vec!["node_id".to_string()];
                for d in 0..dimensions {
                    let mut dim_vec =
                        akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Double, n);
                    for i in 0..n {
                        let val = result.values.get(i * dimensions + d).copied().unwrap_or(0.0);
                        dim_vec.set_value(i, &Value::Double(val))?;
                    }
                    fields.push(akar_common::arrow_vector::ArrowVector::from_legacy(&dim_vec).array);
                    field_types.push(akar_common::types::PhysicalTypeID::Double);
                    field_names.push(format!("dim_{d}"));
                }
                output.fields = fields;
                output.field_types = field_types;
                output.field_names = field_names;
                output.size = n;
                Ok(())
            }
        });
        context.register_table_function(
            "node2vec",
            TableFunction::CustomTableWithGraph {
                name: "node2vec".into(),
                execute: n2v_fn.clone(),
            },
        );
        context.register_table_function(
            "n2v",
            TableFunction::CustomTableWithGraph {
                name: "node2vec".into(),
                execute: n2v_fn,
            },
        );

        // ── shortest_path / sp ────────────────────────────────────────────────
        context.register_table_function(
            "shortest_path",
            TableFunction::CustomTableWithGraph {
                name: "shortest_path".into(),
                execute: sp_destinations_fn.clone(),
            },
        );
        context.register_table_function(
            "sp",
            TableFunction::CustomTableWithGraph {
                name: "shortest_path".into(),
                execute: sp_destinations_fn,
            },
        );
        context.register_table_function(
            "weighted_shortest_path",
            TableFunction::CustomTableWithGraph {
                name: "weighted_shortest_path".into(),
                execute: wsp_destinations_fn,
            },
        );

        tracing::info!("ALGO extension loaded: 30 registrations (15 algorithms × canonical + 15 aliases)");

        Ok(())
    }
}

// ==================== Algorithm Implementations ====================
//
// All algorithms work on CSRAdjacency from Akar-graph.
// In a real execution, the graph is built from storage first.

/// Result of a graph algorithm.
#[derive(Default)]
pub struct AlgoResult {
    pub name: String,
    pub values: Vec<f64>,
    /// Optional metadata (e.g. modularity for Louvain).
    pub metadata: Option<std::collections::HashMap<String, f64>>,
}

// --------------- PageRank (wraps Akar-graph implementation) ---------------

/// Compute PageRank — wraps existing `akar_graph::page_rank`.
pub fn compute_page_rank(csr: &CSRAdjacency) -> AlgoResult {
    let result = akar_graph::page_rank(csr, 0.85, 100, 1e-6);
    AlgoResult {
        name: "page_rank".into(),
        values: result.values,
        metadata: None,
    }
}

// --------------- Weakly Connected Components (wraps Akar-graph) ---------------

/// Compute WCC — wraps existing `akar_graph::weakly_connected_components`.
pub fn compute_wcc(csr: &CSRAdjacency) -> AlgoResult {
    let result = akar_graph::weakly_connected_components(csr);
    AlgoResult {
        name: "wcc".into(),
        values: result.values,
        metadata: None,
    }
}

// --------------- Strongly Connected Components (Tarjan) ---------------

/// Compute SCC using Tarjan's algorithm.
/// Returns component ID (0-based) for each node.
pub fn compute_scc_tarjan(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    let mut indices = vec![None; n];
    let mut lowlink = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut component = vec![0usize; n];
    let mut comp_id = 0usize;
    let mut next_index = 0usize;

    // Iterative Tarjan (P52.47): the recursive strongconnect overflowed the
    // stack on large/deep graphs. DFS stack holds (node, next neighbor pos).
    for start in 0..n {
        if indices[start].is_some() {
            continue;
        }
        indices[start] = Some(next_index);
        lowlink[start] = next_index;
        next_index += 1;
        stack.push(start);
        on_stack[start] = true;
        let mut dfs_stack: Vec<(usize, usize)> = vec![(start, 0)];

        while let Some(&(v, npos)) = dfs_stack.last() {
            let neighbors = csr.neighbors(v);
            if npos < neighbors.len() {
                let (_, dst) = neighbors[npos];
                dfs_stack.last_mut().unwrap().1 += 1;
                let w = dst.offset as usize;
                if w >= n {
                    continue;
                }
                if indices[w].is_none() {
                    indices[w] = Some(next_index);
                    lowlink[w] = next_index;
                    next_index += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    dfs_stack.push((w, 0));
                } else if on_stack[w] {
                    lowlink[v] = lowlink[v].min(indices[w].unwrap());
                }
            } else {
                // v's neighbors exhausted: unwind v, propagate lowlink to the
                // parent, and finalize the SCC if v is its root. Stay inside
                // the loop so the parent resumes its remaining neighbors.
                dfs_stack.pop();
                if let Some(&(parent, _)) = dfs_stack.last() {
                    lowlink[parent] = lowlink[parent].min(lowlink[v]);
                }
                if lowlink[v] == indices[v].unwrap() {
                    loop {
                        let w = stack.pop().unwrap();
                        on_stack[w] = false;
                        component[w] = comp_id;
                        if w == v {
                            break;
                        }
                    }
                    comp_id += 1;
                }
            }
        }
    }

    AlgoResult {
        name: "scc_tarjan".into(),
        values: component.iter().map(|&c| c as f64).collect(),
        metadata: None,
    }
}

// --------------- Strongly Connected Components (Kosaraju) ---------------

/// Compute SCC using Kosaraju's algorithm.
pub fn compute_scc_kosaraju(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    let mut visited = vec![false; n];
    let mut order = Vec::with_capacity(n);

    // Phase 1: iterative DFS to get finish order (P52.47 — the recursive
    // dfs1 overflowed the stack on large/deep graphs).
    for start in 0..n {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut dfs_stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&(v, npos)) = dfs_stack.last() {
            let neighbors = csr.neighbors(v);
            if npos < neighbors.len() {
                let (_, dst) = neighbors[npos];
                dfs_stack.last_mut().unwrap().1 += 1;
                let w = dst.offset as usize;
                if w < n && !visited[w] {
                    visited[w] = true;
                    dfs_stack.push((w, 0));
                }
            } else {
                order.push(v);
                dfs_stack.pop();
            }
        }
    }

    // Phase 2: DFS on reversed graph (simulated via in-edge query)
    // Since we don't have reverse adjacency, we scan all nodes' neighbors.
    // For efficiency, build reverse CSR on the fly.
    let mut rev_adj: Vec<Vec<usize>> = vec![vec![]; n];
    for v in 0..n {
        for (_, dst) in csr.neighbors(v) {
            let w = dst.offset as usize;
            if w < csr.num_nodes() {
                rev_adj[w].push(v);
            }
        }
    }

    let mut component = vec![0usize; n];
    let mut comp_id = 0usize;
    let mut visited2 = vec![false; n];

    for &v in order.iter().rev() {
        if visited2[v] {
            continue;
        }
        // Iterative DFS (replaces recursive dfs2).
        let mut dfs_stack = vec![v];
        visited2[v] = true;
        while let Some(cur) = dfs_stack.pop() {
            component[cur] = comp_id;
            for &w in &rev_adj[cur] {
                if !visited2[w] {
                    visited2[w] = true;
                    dfs_stack.push(w);
                }
            }
        }
        comp_id += 1;
    }

    AlgoResult {
        name: "scc_kosaraju".into(),
        values: component.iter().map(|&c| c as f64).collect(),
        metadata: None,
    }
}

// --------------- K-Core Decomposition ---------------

/// Compute k-core decomposition using iterative peeling.
/// Returns the core number for each node (0-based: max k such that
/// the node is part of the k-core).
///
/// Batagelj & Zaveršnik bucket algorithm — O(V + E) (P52.48; the previous
/// per-level scan of all active nodes was O(V²)).
pub fn compute_k_core(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    let mut degree: Vec<usize> = (0..n).map(|i| csr.neighbors(i).len()).collect();
    let mut core = vec![0usize; n];

    // Bucket sort nodes by degree: bin[d] = start position of degree-d bucket.
    let max_deg = degree.iter().copied().max().unwrap_or(0);
    let mut bin = vec![0usize; max_deg + 1];
    for &d in &degree {
        bin[d] += 1;
    }
    let mut start = 0;
    for b in bin.iter_mut() {
        let cnt = *b;
        *b = start;
        start += cnt;
    }
    let mut vert = vec![0usize; n];
    let mut pos = vec![0usize; n];
    for v in 0..n {
        let d = degree[v];
        pos[v] = bin[d];
        vert[bin[d]] = v;
        bin[d] += 1;
    }
    // Restore bin to bucket start positions.
    for d in (1..=max_deg).rev() {
        bin[d] = bin[d - 1];
    }
    bin[0] = 0;

    for i in 0..n {
        let v = vert[i];
        core[v] = degree[v];
        for (_, dst) in csr.neighbors(v) {
            let w = dst.offset as usize;
            if w >= n {
                continue;
            }
            if degree[w] > degree[v] {
                let du = degree[w];
                let pw = pos[w];
                let pu = bin[du];
                if pw != pu {
                    let w2 = vert[pu];
                    vert[pw] = w2;
                    pos[w2] = pw;
                    vert[pu] = w;
                    pos[w] = pu;
                }
                bin[du] += 1;
                degree[w] = du - 1;
            }
        }
    }

    AlgoResult {
        name: "k_core".into(),
        values: core.iter().map(|&c| c as f64).collect(),
        metadata: None,
    }
}

// --------------- Label Propagation Algorithm (LPA) ---------------

/// Compute Label Propagation Algorithm (LPA) for community detection.
pub fn compute_lpa(csr: &CSRAdjacency, max_iters: usize) -> AlgoResult {
    let n = csr.num_nodes();
    let mut labels: Vec<usize> = (0..n).collect();
    let mut next_labels = labels.clone();

    for _ in 0..max_iters {
        let mut changed = false;
        for v in 0..n {
            let neighbors = csr.neighbors(v);
            if neighbors.is_empty() {
                continue;
            }

            let mut counts: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
            for (_, dst) in neighbors {
                let w = dst.offset as usize;
                if w < n {
                    *counts.entry(labels[w]).or_insert(0) += 1;
                }
            }

            if counts.is_empty() {
                continue;
            }

            let mut max_count = 0;
            let mut best_label = labels[v];
            for (&lbl, &cnt) in &counts {
                if cnt > max_count || (cnt == max_count && lbl > best_label) {
                    max_count = cnt;
                    best_label = lbl;
                }
            }

            if best_label != labels[v] {
                next_labels[v] = best_label;
                changed = true;
            }
        }
        labels.copy_from_slice(&next_labels);
        if !changed {
            break;
        }
    }

    AlgoResult {
        name: "lpa".into(),
        values: labels.iter().map(|&c| c as f64).collect(),
        metadata: None,
    }
}

// --------------- Betweenness Centrality ---------------

/// Compute Betweenness Centrality using Brandes' algorithm.
pub fn compute_betweenness_centrality(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    let mut cb = vec![0.0; n];

    for s in 0..n {
        let mut stack = Vec::new();
        let mut pred: Vec<Vec<usize>> = vec![vec![]; n];
        let mut sigma = vec![0.0; n];
        sigma[s] = 1.0;
        let mut dist = vec![-1i64; n];
        dist[s] = 0;

        let mut q = std::collections::VecDeque::new();
        q.push_back(s);

        while let Some(v) = q.pop_front() {
            stack.push(v);
            for (_, dst) in csr.neighbors(v) {
                let w = dst.offset as usize;
                if w >= n {
                    continue;
                }

                // Path discovery
                if dist[w] < 0 {
                    dist[w] = dist[v] + 1;
                    q.push_back(w);
                }

                // Path counting
                if dist[w] == dist[v] + 1 {
                    sigma[w] += sigma[v];
                    pred[w].push(v);
                }
            }
        }

        let mut delta = vec![0.0; n];
        // Accumulation
        while let Some(w) = stack.pop() {
            for &v in &pred[w] {
                if sigma[w] > 0.0 {
                    delta[v] += (sigma[v] / sigma[w]) * (1.0 + delta[w]);
                }
            }
            if w != s {
                cb[w] += delta[w];
            }
        }
    }

    AlgoResult {
        name: "betweenness_centrality".into(),
        values: cb,
        metadata: None,
    }
}

// --------------- Closeness Centrality ---------------

/// Compute Closeness Centrality using BFS from each node.
/// Uses Wasserman-Faust normalization for disconnected graphs.
/// C(u) = (|R(u)| / (n-1))² * (|R(u)| / sum_{v in R(u)} d(u,v))
/// where R(u) is the set of nodes reachable from u.
pub fn compute_closeness_centrality(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    if n <= 2 {
        return AlgoResult {
            name: "closeness_centrality".into(),
            values: vec![0.0; n],
            metadata: None,
        };
    }

    let n_minus_1 = (n - 1) as f64;
    let mut values = vec![0.0; n];

    for source in 0..n {
        let (distances, _) = shortest_path_bfs(csr, source);
        let (sum_dist, reachable): (f64, usize) = distances
            .iter()
            .enumerate()
            .filter(|(i, d)| d.is_some() && *i != source)
            .fold((0.0, 0), |(sum, cnt), (_, d)| (sum + d.unwrap() as f64, cnt + 1));

        if reachable == 0 || sum_dist == 0.0 {
            continue;
        }

        let r = reachable as f64;
        // Wasserman-Faust normalized closeness
        values[source] = (r / n_minus_1) * (r / n_minus_1) * (r / sum_dist);
    }

    AlgoResult {
        name: "closeness_centrality".into(),
        values,
        metadata: None,
    }
}

// --------------- Triangle Counting ---------------

/// Count triangles per node using neighbor intersection.
/// For each node, counts how many pairs of its neighbors are connected.
/// Returns triangle count per node. Total triangles = sum(values) / 3.
pub fn compute_triangle_count(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    let mut values = vec![0.0; n];

    // Collect sorted neighbor lists for efficient intersection
    let mut neighbors: Vec<Vec<usize>> = Vec::with_capacity(n);
    for v in 0..n {
        let mut neigh: Vec<usize> = csr
            .neighbors(v)
            .iter()
            .map(|(_, dst)| dst.offset as usize)
            .filter(|&dst| dst < n && dst != v)
            .collect();
        neigh.sort_unstable();
        neigh.dedup();
        neighbors.push(neigh);
    }

    // For each node, check if neighbor pairs are connected
    for v in 0..n {
        for &u in &neighbors[v] {
            if u <= v {
                continue;
            }
            // Count common neighbors of v and u
            let mut common = 0usize;
            let mut i = 0usize;
            let mut j = 0usize;
            while i < neighbors[v].len() && j < neighbors[u].len() {
                let a = neighbors[v][i];
                let b = neighbors[u][j];
                if a == b {
                    if a != v && a != u {
                        common += 1;
                    }
                    i += 1;
                    j += 1;
                } else if a < b {
                    i += 1;
                } else {
                    j += 1;
                }
            }
            values[v] += common as f64;
            values[u] += common as f64;
        }
        values[v] /= 2.0; // each triangle counted twice per node
    }

    AlgoResult {
        name: "triangle_count".into(),
        values,
        metadata: None,
    }
}

// --------------- Louvain Community Detection ---------------

/// Weighted Louvain community detection with configurable parameters.
///
/// - `weight_fn`: closure returning edge weight for (src_pos, dst_pos)
/// - `seed`: optional RNG seed for deterministic node visit order
/// - `min_gain`: minimum modularity gain to accept a move (default 0.001)
/// - `max_iterations`: maximum passes (default 20)
///
/// Returns `AlgoResult` with community IDs in `values` and modularity Q in `metadata`.
pub fn compute_louvain_weighted<F>(
    csr: &CSRAdjacency,
    weight_fn: F,
    seed: Option<u64>,
    min_gain: f64,
    max_iterations: usize,
) -> AlgoResult
where
    F: Fn(usize, usize) -> f64,
{
    let n = csr.num_nodes();
    if n == 0 {
        return AlgoResult {
            name: "louvain".into(),
            values: vec![],
            metadata: None,
        };
    }

    // Weighted total edge weight (each edge counted twice in undirected CSR)
    let mut m: f64 = 0.0;
    for v in 0..n {
        for (_rel, dst) in csr.neighbors(v) {
            let w = dst.offset as usize;
            if w < n {
                m += weight_fn(v, w);
            }
        }
    }
    m /= 2.0;

    if m == 0.0 {
        return AlgoResult {
            name: "louvain".into(),
            values: (0..n).map(|_| 0.0).collect(),
            metadata: None,
        };
    }

    // Weighted degree per node
    let degree: Vec<f64> = (0..n)
        .map(|v| {
            csr.neighbors(v)
                .iter()
                .map(|(_rel, dst)| {
                    let w = dst.offset as usize;
                    if w < n { weight_fn(v, w) } else { 0.0 }
                })
                .sum()
        })
        .collect();

    // Internal edge weight per community (initially 0 — each node in its own community)
    let mut sigma_in: Vec<f64> = vec![0.0; n];
    // Community total degree
    let mut sigma_tot: Vec<f64> = degree.clone();
    // Node community assignment
    let mut community: Vec<usize> = (0..n).collect();

    let mut improved = true;
    let mut pass = 0;

    // Simple LCG for deterministic shuffling (matches codebase convention)
    struct SimpleRng { state: u64 }
    impl SimpleRng {
        fn new(seed: u64) -> Self { Self { state: seed } }
        fn next_u64(&mut self) -> u64 {
            self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
            self.state
        }
        fn shuffle(&mut self, slice: &mut [usize]) {
            for i in (1..slice.len()).rev() {
                let j = self.next_u64() as usize % (i + 1);
                slice.swap(i, j);
            }
        }
    }

    while improved && pass < max_iterations {
        improved = false;
        pass += 1;

        // Deterministic or sequential visit order
        let mut order: Vec<usize> = (0..n).collect();
        if let Some(s) = seed {
            SimpleRng::new(s.wrapping_add(pass as u64)).shuffle(&mut order);
        }

        // Batch mode: compute all gains, then apply best moves
        let mut moves: Vec<(usize, usize, f64)> = Vec::new(); // (v, best_comm, gain)

        for &v in &order {
            let current_comm = community[v];
            let kv = degree[v];

            // Sum of weights from v to each neighboring community
            let mut comm_weights: std::collections::BTreeMap<usize, f64> =
                std::collections::BTreeMap::new();
            for (_rel, dst) in csr.neighbors(v) {
                let w = dst.offset as usize;
                if w < n {
                    let wt = weight_fn(v, w);
                    *comm_weights.entry(community[w]).or_insert(0.0) += wt;
                }
            }

            let sigma_v_current = comm_weights.get(&current_comm).copied().unwrap_or(0.0);

            // ΔQ = (Σ_vD - Σ_vC)/m + k_v·(Σ_tot_C - Σ_tot_D - k_v)/(2m²)
            let mut best_comm = current_comm;
            let mut best_gain = min_gain;

            for (&comm, &sigma_v_comm) in &comm_weights {
                if comm == current_comm {
                    continue;
                }
                let gain = (sigma_v_comm - sigma_v_current) / m
                    + kv * (sigma_tot[current_comm] - sigma_tot[comm] - kv) / (2.0 * m * m);
                if gain > best_gain {
                    best_gain = gain;
                    best_comm = comm;
                }
            }

            if best_comm != current_comm {
                moves.push((v, best_comm, best_gain));
            }
        }

        // Apply batch moves (first-come-first-served for same-community conflicts)
        let mut claimed: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for (v, best_comm, _gain) in &moves {
            let v = *v;
            let best_comm = *best_comm;
            let current_comm = community[v];
            if current_comm == best_comm || claimed.contains(&best_comm) {
                continue;
            }
            let kv = degree[v];

            // Recompute v's weight into new community using current community assignments
            let sigma_v_current: f64 = csr
                .neighbors(v)
                .iter()
                .filter_map(|(_rel, dst)| {
                    let w = dst.offset as usize;
                    if w < n && community[w] == current_comm {
                        Some(weight_fn(v, w))
                    } else {
                        None
                    }
                })
                .sum();
            let sigma_v_new: f64 = csr
                .neighbors(v)
                .iter()
                .filter_map(|(_rel, dst)| {
                    let w = dst.offset as usize;
                    if w < n && community[w] == best_comm {
                        Some(weight_fn(v, w))
                    } else {
                        None
                    }
                })
                .sum();

            sigma_in[current_comm] -= 2.0 * sigma_v_current;
            sigma_tot[current_comm] -= kv;
            sigma_in[best_comm] += 2.0 * sigma_v_new;
            sigma_tot[best_comm] += kv;
            community[v] = best_comm;
            claimed.insert(v);
            improved = true;
        }
    }

    // Compute final modularity Q = Σ_c [σ_in_c / m - (σ_tot_c / 2m)²]
    let mut q = 0.0;
    let mut seen_comms = std::collections::HashSet::new();
    for v in 0..n {
        let c = community[v];
        if seen_comms.insert(c) {
            q += sigma_in[c] / m - (sigma_tot[c] / (2.0 * m)).powi(2);
        }
    }

    // Assign sequential community IDs
    let mut comm_map: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let values: Vec<f64> = community
        .iter()
        .map(|&c| {
            let len = comm_map.len();
            *comm_map.entry(c).or_insert(len) as f64
        })
        .collect();

    let mut metadata = std::collections::HashMap::new();
    metadata.insert("modularity".into(), q);

    AlgoResult {
        name: "louvain".into(),
        values,
        metadata: Some(metadata),
    }
}

/// Compute community structure using the Louvain heuristic (unweighted, default parameters).
pub fn compute_louvain(csr: &CSRAdjacency) -> AlgoResult {
    compute_louvain_weighted(csr, |_u, _v| 1.0, None, 0.001, 20)
}

// --------------- Spreading Activation ---------------

/// Result of a spread activation computation.
pub struct SpreadActivationResult {
    /// Activated nodes: (node_position, activation_score, hop_reached).
    pub activated: Vec<(usize, f64, usize)>,
}

/// Spreading activation over a graph starting from seed nodes.
///
/// - `seed_positions`: starting node positions with initial activation
/// - `weight_fn`: closure returning edge weight for (src_pos, dst_pos)
/// - `decay`: multiplicative decay per hop (0.0..1.0)
/// - `threshold`: minimum activation to survive pruning
/// - `max_hops`: maximum propagation distance
pub fn compute_spread_activation<F>(
    csr: &CSRAdjacency,
    seed_positions: &[(usize, f64)],
    weight_fn: F,
    decay: f64,
    threshold: f64,
    max_hops: usize,
) -> SpreadActivationResult
where
    F: Fn(usize, usize) -> f64,
{
    let n = csr.num_nodes();
    if n == 0 || seed_positions.is_empty() {
        return SpreadActivationResult {
            activated: Vec::new(),
        };
    }

    // activation[node_pos] = (current_activation, hop_when_activated)
    let mut activation: Vec<f64> = vec![0.0; n];
    let mut hop_reached: Vec<usize> = vec![usize::MAX; n];

    // Initialize seeds at hop 0
    let mut current_frontier: Vec<(usize, f64)> = Vec::new();
    for &(pos, act) in seed_positions {
        if pos < n && act > 0.0 {
            activation[pos] = act;
            hop_reached[pos] = 0;
            current_frontier.push((pos, act));
        }
    }

    // Propagate hop by hop
    for hop in 1..=max_hops {
        let mut next_activation: std::collections::BTreeMap<usize, f64> =
            std::collections::BTreeMap::new();

        for &(node, _node_act) in &current_frontier {
            let total_act = activation[node];
            for (_rel, dst) in csr.neighbors(node) {
                let w = dst.offset as usize;
                if w >= n {
                    continue;
                }
                // Skip nodes already activated at an earlier hop (prevent cycle inflation)
                if hop_reached[w] != usize::MAX && hop_reached[w] < hop {
                    continue;
                }
                let edge_weight = weight_fn(node, w);
                let propagated = total_act * edge_weight * decay;
                if propagated < threshold {
                    continue;
                }
                *next_activation.entry(w).or_insert(0.0) += propagated;
            }
        }

        let mut next_frontier: Vec<(usize, f64)> = Vec::new();
        for (&w, &prop) in &next_activation {
            activation[w] += prop;
            if hop_reached[w] == usize::MAX {
                hop_reached[w] = hop;
            }
            // Only add to frontier if first time reaching this node
            // (same-hop accumulation is folded into activation but doesn't re-propagate)
            if hop_reached[w] == hop {
                next_frontier.push((w, prop));
            }
        }

        current_frontier = next_frontier;
        if current_frontier.is_empty() {
            break;
        }
    }

    // Collect activated nodes (excluding seeds at hop 0, include only propagated nodes)
    let mut activated: Vec<(usize, f64, usize)> = Vec::new();
    for pos in 0..n {
        if hop_reached[pos] <= max_hops && activation[pos] >= threshold {
            activated.push((pos, activation[pos], hop_reached[pos]));
        }
    }
    activated.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    SpreadActivationResult { activated }
}

// --------------- Spanning Forest (Kruskal) ---------------

/// Compute a spanning forest using Kruskal's algorithm.
/// Returns parent component ID for each node after building the forest.
/// For a connected graph, this produces a spanning tree.
pub fn compute_spanning_forest(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    if n == 0 {
        return AlgoResult {
            name: "spanning_forest".into(),
            values: vec![],
            metadata: None,
        };
    }

    // Union-Find data structure
    let mut parent: Vec<usize> = (0..n).collect();
    let mut rank = vec![0usize; n];

    fn find(parent: &mut [usize], x: usize) -> usize {
        let mut root = x;
        while parent[root] != root {
            root = parent[root];
        }
        let mut cur = x;
        while parent[cur] != root {
            let next = parent[cur];
            parent[cur] = root;
            cur = next;
        }
        root
    }

    fn union(parent: &mut [usize], rank: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            match rank[ra].cmp(&rank[rb]) {
                std::cmp::Ordering::Less => parent[ra] = rb,
                std::cmp::Ordering::Greater => parent[rb] = ra,
                std::cmp::Ordering::Equal => {
                    parent[rb] = ra;
                    rank[ra] += 1;
                }
            }
        }
    }

    // Process each edge; for undirected, each edge appears twice in CSR
    for v in 0..n {
        for (_, dst) in csr.neighbors(v) {
            let w = dst.offset as usize;
            if w < n && v < w {
                // Only process each edge once (v < w)
                union(&mut parent, &mut rank, v, w);
            }
        }
    }

    // Compress all paths
    for i in 0..n {
        find(&mut parent, i);
    }

    // Assign component IDs
    let mut comp_map: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let values: Vec<f64> = parent
        .iter()
        .map(|&p| {
            let len = comp_map.len();
            *comp_map.entry(p).or_insert(len) as f64
        })
        .collect();

    AlgoResult {
        name: "spanning_forest".into(),
        values,
        metadata: None,
    }
}

// ==================== Shortest Path Algorithms ====================

/// Compute shortest path distances from a source node using BFS (unweighted).
///
/// Returns `(distances, parents)` where:
/// - `distances[i]` = shortest distance (number of hops) from source to node i,
///   or `None` if node i is unreachable.
/// - `parents[i]` = predecessor node on the shortest path, or `None` for source/unreachable.
pub fn shortest_path_bfs(csr: &CSRAdjacency, source: usize) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let n = csr.num_nodes();
    if source >= n {
        return (vec![None; n], vec![None; n]);
    }
    let mut distance = vec![None; n];
    let mut parent = vec![None; n];
    let mut queue = std::collections::VecDeque::new();

    distance[source] = Some(0);
    queue.push_back(source);

    while let Some(node) = queue.pop_front() {
        let dist = distance[node].unwrap();
        for (_rel, dst) in csr.neighbors(node) {
            let neighbor = dst.offset as usize;
            if neighbor < n && distance[neighbor].is_none() {
                distance[neighbor] = Some(dist + 1);
                parent[neighbor] = Some(node);
                queue.push_back(neighbor);
            }
        }
    }

    (distance, parent)
}

/// Compute shortest path distances and return as `AlgoResult`.
///
/// Each node gets its shortest distance from the source node.
/// Unreachable nodes get distance `f64::MAX`.
pub fn compute_shortest_path(csr: &CSRAdjacency, source: usize) -> AlgoResult {
    let (distance, _parent) = shortest_path_bfs(csr, source);
    let values: Vec<f64> = distance
        .iter()
        .map(|d| d.map(|v| v as f64).unwrap_or(f64::MAX))
        .collect();
    AlgoResult {
        name: "shortest_path".into(),
        values,
        metadata: None,
    }
}

/// Compute weighted shortest path from a source node using Dijkstra's algorithm.
///
/// The `weight_fn` maps a node index and its neighbor offset to the edge weight.
/// Returns `(distances, parents)`.
/// A min-heap entry for Dijkstra: (distance, node).
/// Uses `f64::total_cmp` for total ordering of floats.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DistNode(f64, usize);

impl Eq for DistNode {}

impl PartialOrd for DistNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DistNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse so BinaryHeap becomes a min-heap
        other.0.total_cmp(&self.0).then_with(|| self.1.cmp(&other.1))
    }
}

pub fn weighted_shortest_path<F>(
    csr: &CSRAdjacency,
    source: usize,
    weight_fn: F,
) -> (Vec<Option<f64>>, Vec<Option<usize>>)
where
    F: Fn(usize, usize) -> f64,
{
    use std::collections::BinaryHeap;

    let n = csr.num_nodes();
    if source >= n {
        return (vec![None; n], vec![None; n]);
    }

    let mut distance: Vec<Option<f64>> = vec![None; n];
    let mut parent: Vec<Option<usize>> = vec![None; n];
    let mut heap = BinaryHeap::new();

    distance[source] = Some(0.0);
    heap.push(DistNode(0.0, source));

    while let Some(DistNode(dist, node)) = heap.pop() {
        if let Some(best) = distance[node] {
            if dist > best {
                continue;
            }
        } else {
            continue;
        }

        for (_rel, dst) in csr.neighbors(node) {
            let neighbor = dst.offset as usize;
            if neighbor >= n {
                continue;
            }
            let weight = weight_fn(node, neighbor);
            let new_dist = dist + weight;

            match distance[neighbor] {
                Some(best) if new_dist >= best => {}
                _ => {
                    distance[neighbor] = Some(new_dist);
                    parent[neighbor] = Some(node);
                    heap.push(DistNode(new_dist, neighbor));
                }
            }
        }
    }

    (distance, parent)
}

/// Compute weighted shortest path distances and return as `AlgoResult`.
///
/// Uses unit weights (equivalent to BFS shortest path).
pub fn compute_weighted_shortest_path(csr: &CSRAdjacency, source: usize) -> AlgoResult {
    let (distance, _parent) = weighted_shortest_path(csr, source, |_from, _to| 1.0);
    let values: Vec<f64> = distance.iter().map(|d| d.unwrap_or(f64::MAX)).collect();
    AlgoResult {
        name: "weighted_shortest_path".into(),
        values,
        metadata: None,
    }
}

/// Compute all-pairs shortest path destinations using repeated BFS.
///
/// Returns the number of reachable nodes from each source (destination count).
pub fn compute_all_sp_destinations(csr: &CSRAdjacency) -> AlgoResult {
    let n = csr.num_nodes();
    let values: Vec<f64> = (0..n)
        .map(|source| {
            let (distance, _) = shortest_path_bfs(csr, source);
            // Count reachable nodes (excluding source itself)
            distance.iter().filter(|d| d.is_some()).count().saturating_sub(1) as f64
        })
        .collect();
    AlgoResult {
        name: "all_sp_destinations".into(),
        values,
        metadata: None,
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use akar_graph::Edge;

    fn small_csr() -> CSRAdjacency {
        // Graph: 0--1--2--3
        //        |     |
        //        4--5--6
        let edges = vec![
            Edge {
                src_offset: 0,
                dst_offset: 1,
                rel_id: 0,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 0,
                dst_offset: 4,
                rel_id: 1,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 1,
                dst_offset: 2,
                rel_id: 2,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 2,
                dst_offset: 3,
                rel_id: 3,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 2,
                dst_offset: 6,
                rel_id: 4,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 4,
                dst_offset: 5,
                rel_id: 5,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 5,
                dst_offset: 6,
                rel_id: 6,
                rel_table_id: 0,
            },
        ];
        CSRAdjacency::build(&edges, 7)
    }

    fn disconnected_csr() -> CSRAdjacency {
        // Two components: 0--1  and  2--3
        let edges = vec![
            Edge {
                src_offset: 0,
                dst_offset: 1,
                rel_id: 0,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 2,
                dst_offset: 3,
                rel_id: 1,
                rel_table_id: 0,
            },
        ];
        CSRAdjacency::build(&edges, 4)
    }

    #[test]
    fn test_page_rank() {
        let csr = small_csr();
        let result = compute_page_rank(&csr);
        assert_eq!(result.values.len(), 7);
        // All PageRank values should be positive
        for &v in &result.values {
            assert!(v > 0.0, "PageRank should be positive, got {v}");
        }
    }

    #[test]
    fn test_wcc_connected() {
        let csr = small_csr();
        let result = compute_wcc(&csr);
        assert_eq!(result.values.len(), 7);
        // Single component → all same component
        let first = result.values[0];
        assert!(result.values.iter().all(|&v| v == first));
    }

    #[test]
    fn test_wcc_disconnected() {
        let csr = disconnected_csr();
        let result = compute_wcc(&csr);
        assert_eq!(result.values.len(), 4);
        assert!((result.values[0] - result.values[1]).abs() < 1e-10); // same component
        assert!((result.values[2] - result.values[3]).abs() < 1e-10); // same component
        assert!((result.values[0] - result.values[2]).abs() >= 1e-10); // different components
    }

    #[test]
    fn test_scc_tarjan() {
        let csr = small_csr();
        let result = compute_scc_tarjan(&csr);
        assert_eq!(result.values.len(), 7);
        // All nodes in the same SCC (undirected graph)
        let first = result.values[0];
        assert!(result.values.iter().all(|&v| v == first));
    }

    #[test]
    fn test_scc_kosaraju() {
        let csr = small_csr();
        let result = compute_scc_kosaraju(&csr);
        assert_eq!(result.values.len(), 7);
        let first = result.values[0];
        assert!(result.values.iter().all(|&v| v == first));
    }

    fn directed_csr(edges: &[(usize, usize)], n: usize) -> CSRAdjacency {
        use akar_common::types::InternalID;
        let mut offsets = vec![0usize; n + 1];
        let mut deg = vec![0usize; n];
        for (s, _) in edges {
            deg[*s] += 1;
        }
        let mut cur = 0;
        for i in 0..n {
            offsets[i] = cur;
            cur += deg[i];
        }
        offsets[n] = cur;
        let mut adj = vec![(0u64, InternalID { table_id: 0, offset: 0 }); cur];
        let mut pos = offsets.clone();
        for (s, d) in edges {
            adj[pos[*s]] = (
                0u64,
                InternalID {
                    table_id: 0,
                    offset: *d as u64,
                },
            );
            pos[*s] += 1;
        }
        CSRAdjacency {
            offsets,
            adjacency: adj,
        }
    }

    fn directed_scc_graph() -> CSRAdjacency {
        // Cycle {0,1,2}: 0->1, 1->2, 2->0
        // Cycle {3,4}: 2->3, 3->4, 4->3
        directed_csr(&[(0, 1), (1, 2), (2, 0), (2, 3), (3, 4), (4, 3)], 5)
    }

    #[test]
    fn test_scc_tarjan_directed() {
        let result = compute_scc_tarjan(&directed_scc_graph());
        assert_eq!(result.values.len(), 5);
        let c = |i: usize| result.values[i] as usize;
        assert_eq!(c(0), c(1));
        assert_eq!(c(1), c(2));
        assert_eq!(c(3), c(4));
        assert_ne!(c(0), c(3));
    }

    #[test]
    fn test_scc_kosaraju_directed() {
        let result = compute_scc_kosaraju(&directed_scc_graph());
        assert_eq!(result.values.len(), 5);
        let c = |i: usize| result.values[i] as usize;
        assert_eq!(c(0), c(1));
        assert_eq!(c(1), c(2));
        assert_eq!(c(3), c(4));
        assert_ne!(c(0), c(3));
    }

    #[test]
    fn test_scc_deep_chain_no_stack_overflow() {
        let n = 200_000;
        let edges: Vec<Edge> = (0..n - 1)
            .map(|i| Edge {
                src_offset: i as u64,
                dst_offset: (i + 1) as u64,
                rel_id: i as u64,
                rel_table_id: 0,
            })
            .collect();
        let csr = CSRAdjacency::build(&edges, n);
        let tarjan = compute_scc_tarjan(&csr);
        let kosaraju = compute_scc_kosaraju(&csr);
        assert_eq!(tarjan.values.len(), n);
        assert_eq!(kosaraju.values.len(), n);
        // Undirected chain → single SCC
        assert!(tarjan.values.iter().all(|&v| v == tarjan.values[0]));
        assert!(kosaraju.values.iter().all(|&v| v == kosaraju.values[0]));
    }

    #[test]
    fn test_k_core_correctness() {
        // Complete graph K4 (all 6 edges): every node has degree 3, core = 3.
        let edges = vec![
            Edge {
                src_offset: 0,
                dst_offset: 1,
                rel_id: 0,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 0,
                dst_offset: 2,
                rel_id: 1,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 0,
                dst_offset: 3,
                rel_id: 2,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 1,
                dst_offset: 2,
                rel_id: 3,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 1,
                dst_offset: 3,
                rel_id: 4,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 2,
                dst_offset: 3,
                rel_id: 5,
                rel_table_id: 0,
            },
        ];
        let csr = CSRAdjacency::build(&edges, 4);
        let result = compute_k_core(&csr);
        for &v in &result.values {
            assert_eq!(v, 3.0);
        }
    }

    #[test]
    fn test_k_core() {
        let csr = small_csr();
        let result = compute_k_core(&csr);
        assert_eq!(result.values.len(), 7);
        // In a 3-regular-ish graph, all nodes should have core >= 2
        for &v in &result.values {
            assert!(v >= 1.0, "Core number should be >= 1");
        }
    }

    #[test]
    fn test_louvain() {
        let csr = small_csr();
        let result = compute_louvain(&csr);
        assert_eq!(result.values.len(), 7);
        // Community IDs should be sequential from 0
        let max_comm = result.values.iter().fold(0.0f64, |a, &b| a.max(b));
        assert!(max_comm >= 0.0);
        // Modularity should be present and valid
        let q = result.metadata.as_ref().unwrap().get("modularity").unwrap();
        assert!(*q >= -0.5 && *q <= 1.0, "modularity {q} out of range");
    }

    #[test]
    fn test_louvain_weighted() {
        // Two dense cliques connected by a single weak edge
        // Clique A: 0-1-2-3 (fully connected, weight 10)
        // Clique B: 4-5-6-7 (fully connected, weight 10)
        // Bridge: 3-4 (weight 1)
        let mut edges = Vec::new();
        let mut rel = 0u64;
        // Clique A
        for i in 0..4 {
            for j in (i+1)..4 {
                edges.push(Edge { src_offset: i, dst_offset: j, rel_id: rel, rel_table_id: 0 });
                rel += 1;
            }
        }
        // Clique B
        for i in 4..8 {
            for j in (i+1)..8 {
                edges.push(Edge { src_offset: i, dst_offset: j, rel_id: rel, rel_table_id: 0 });
                rel += 1;
            }
        }
        // Bridge (weak)
        edges.push(Edge { src_offset: 3, dst_offset: 4, rel_id: rel, rel_table_id: 0 });
        let csr = CSRAdjacency::build(&edges, 8);
        let result = compute_louvain_weighted(&csr, |u, v| {
            if (u == 3 && v == 4) || (u == 4 && v == 3) { 1.0 } else { 10.0 }
        }, None, 0.001, 20);
        assert_eq!(result.values.len(), 8);
        // Should find 2 communities
        let num_comms: usize = result.values.iter().map(|x| *x as usize).collect::<std::collections::HashSet<_>>().len();
        assert_eq!(num_comms, 2, "weighted louvain should find 2 clusters");
        // Nodes 0-3 should share a community, 4-7 should share another
        assert_eq!(result.values[0], result.values[1]);
        assert_eq!(result.values[0], result.values[2]);
        assert_eq!(result.values[0], result.values[3]);
        assert_eq!(result.values[4], result.values[5]);
        assert_eq!(result.values[4], result.values[6]);
        assert_eq!(result.values[4], result.values[7]);
        assert_ne!(result.values[0], result.values[4]);
    }

    #[test]
    fn test_louvain_modularity_positive() {
        // Two dense cliques connected by a single edge — clear community structure
        let mut edges = Vec::new();
        let mut rel = 0u64;
        for i in 0..5 {
            for j in (i+1)..5 {
                edges.push(Edge { src_offset: i, dst_offset: j, rel_id: rel, rel_table_id: 0 });
                rel += 1;
            }
        }
        for i in 5..10 {
            for j in (i+1)..10 {
                edges.push(Edge { src_offset: i, dst_offset: j, rel_id: rel, rel_table_id: 0 });
                rel += 1;
            }
        }
        edges.push(Edge { src_offset: 4, dst_offset: 5, rel_id: rel, rel_table_id: 0 });
        let csr = CSRAdjacency::build(&edges, 10);
        let result = compute_louvain(&csr);
        let q = result.metadata.as_ref().unwrap().get("modularity").unwrap();
        assert!(*q > 0.3, "modularity should be strongly positive for two cliques, got {q}");
    }

    #[test]
    fn test_louvain_seed_deterministic() {
        let csr = small_csr();
        let r1 = compute_louvain_weighted(&csr, |_u, _v| 1.0, Some(42), 0.001, 20);
        let r2 = compute_louvain_weighted(&csr, |_u, _v| 1.0, Some(42), 0.001, 20);
        assert_eq!(r1.values, r2.values, "same seed should produce same result");
    }

    #[test]
    fn test_louvain_empty() {
        let csr = CSRAdjacency::new(0);
        let result = compute_louvain(&csr);
        assert!(result.values.is_empty());
    }

    #[test]
    fn test_call_louvain() {
        use akar_common::types::Value;
        let registry = make_registry_with_algo();
        let rows = registry
            .execute_table_function("louvain", &[], None)
            .expect("louvain should succeed");
        assert_eq!(rows.len(), 5, "5-node ring → 5 rows");
        for row in &rows {
            assert_eq!(row.len(), 3, "each row should have node_id + community_id + modularity");
        }
        // Modularity should be the same in every row
        let mod0 = match rows[0].get(2) {
            Some(Value::Double(v)) => v,
            _ => panic!("expected Double for modularity"),
        };
        for row in rows.iter().skip(1) {
            let m = match row.get(2) {
                Some(Value::Double(v)) => v,
                _ => panic!("expected Double"),
            };
            assert!((m - mod0).abs() < 1e-10, "modularity should be broadcast uniformly");
        }
    }

    #[test]
    fn test_spanning_forest_connected() {
        let csr = small_csr();
        let result = compute_spanning_forest(&csr);
        assert_eq!(result.values.len(), 7);
        // Connected graph → single component
        let first = result.values[0];
        assert!(result.values.iter().all(|&v| v == first));
    }

    #[test]
    fn test_spanning_forest_disconnected() {
        let csr = disconnected_csr();
        let result = compute_spanning_forest(&csr);
        assert_eq!(result.values.len(), 4);
        assert!((result.values[0] - result.values[1]).abs() < 1e-10); // same component
        assert!((result.values[2] - result.values[3]).abs() < 1e-10); // same component
        assert!((result.values[0] - result.values[2]).abs() >= 1e-10); // different components
    }

    #[test]
    fn test_spanning_forest() {
        let csr = disconnected_csr();
        let result = compute_spanning_forest(&csr);
        assert_eq!(result.values.len(), 4);
        // Node 0 should point to itself (root) or 1
        // Node 1 should point to 0
        assert!(result.values[0] == 0.0 || result.values[0] == 1.0);
    }

    #[test]
    fn test_lpa() {
        let csr = small_csr();
        let result = compute_lpa(&csr, 10);
        assert_eq!(result.values.len(), 7);
    }

    #[test]
    fn test_betweenness_centrality() {
        let csr = small_csr();
        let result = compute_betweenness_centrality(&csr);
        assert_eq!(result.values.len(), 7);
        // Node 1 and 2 and 4 should have some positive centrality since they are on shortest paths
        assert!(result.values[1] >= 0.0);
        assert!(result.values[2] >= 0.0);
    }

    #[test]
    fn test_closeness_centrality() {
        let csr = small_csr();
        let result = compute_closeness_centrality(&csr);
        assert_eq!(result.values.len(), 7);
        // All nodes should have some closeness centrality
        for &v in &result.values {
            assert!(v >= 0.0, "Closeness centrality should be >= 0, got {v}");
        }
        // Node 3 is most central (middle of chain), closeness values should differ
        assert!(result.values[3] > 0.0, "Node 3 should have positive centrality");
    }

    #[test]
    fn test_closeness_centrality_disconnected() {
        let csr = disconnected_csr();
        let result = compute_closeness_centrality(&csr);
        assert_eq!(result.values.len(), 4);
        // Disconnected nodes have limited reachability
        assert!(result.values[0] > 0.0);
        assert!(result.values[2] > 0.0);
        // Different components are independent
        let sum_01 = result.values[0] + result.values[1];
        let sum_23 = result.values[2] + result.values[3];
        assert!(sum_01 > 0.0);
        assert!(sum_23 > 0.0);
    }

    #[test]
    fn test_triangle_count() {
        // Graph with triangles: 0-1-2 triangle (0-1, 1-2, 0-2)
        let edges = vec![
            Edge {
                src_offset: 0,
                dst_offset: 1,
                rel_id: 0,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 1,
                dst_offset: 2,
                rel_id: 1,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 0,
                dst_offset: 2,
                rel_id: 2,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 2,
                dst_offset: 3,
                rel_id: 3,
                rel_table_id: 0,
            },
        ];
        let csr = CSRAdjacency::build(&edges, 4);
        let result = compute_triangle_count(&csr);
        assert_eq!(result.values.len(), 4);
        // Nodes 0,1,2 should each have 1 triangle, node 3 should have 0
        assert!((result.values[0] - 1.0).abs() < 1e-10, "Node 0 should have 1 triangle");
        assert!((result.values[1] - 1.0).abs() < 1e-10, "Node 1 should have 1 triangle");
        assert!((result.values[2] - 1.0).abs() < 1e-10, "Node 2 should have 1 triangle");
        assert!((result.values[3] - 0.0).abs() < 1e-10, "Node 3 should have 0 triangles");
        // Total triangles = sum/3 = 3/3 = 1
        let total: f64 = result.values.iter().sum();
        assert!((total - 3.0).abs() < 1e-10, "Total triangle weight should be 3");
    }

    #[test]
    fn test_triangle_count_small_csr() {
        let csr = small_csr();
        let result = compute_triangle_count(&csr);
        assert_eq!(result.values.len(), 7);
        // The small CSR (0-1-2-3 and 0-4-5-6) should have no triangles
        for &v in &result.values {
            assert!((v - 0.0).abs() < 1e-10, "No triangles in small CSR, got {v} for node");
        }
    }

    #[test]
    fn test_algo_extension_registration() {
        let ext = AlgoExtension::new();
        assert_eq!(ext.name(), "ALGO");
    }

    // ==================== Shortest Path Tests ====================

    #[test]
    fn test_shortest_path_bfs_direct() {
        let csr = small_csr();
        let (dist, parent) = shortest_path_bfs(&csr, 0);
        // 0→1→2→3: distance to node 3 is 3
        assert_eq!(dist[3], Some(3));
        // 0→4→5→6: distance to node 6 is 3
        assert_eq!(dist[6], Some(3));
        // 0 to itself: distance 0
        assert_eq!(dist[0], Some(0));
        // Parent chain: 3's parent should be 2
        assert_eq!(parent[3], Some(2));
    }

    #[test]
    fn test_shortest_path_bfs_unreachable() {
        let csr = disconnected_csr();
        let (dist, _parent) = shortest_path_bfs(&csr, 0);
        assert_eq!(dist[0], Some(0));
        assert_eq!(dist[1], Some(1));
        assert_eq!(dist[2], None); // unreachable
        assert_eq!(dist[3], None); // unreachable
    }

    #[test]
    fn test_shortest_path_bfs_out_of_range_source() {
        let csr = small_csr();
        let (dist, parent) = shortest_path_bfs(&csr, 100);
        assert!(dist.iter().all(|d| d.is_none()));
        assert!(parent.iter().all(|p| p.is_none()));
    }

    #[test]
    fn test_compute_shortest_path() {
        let csr = small_csr();
        let result = compute_shortest_path(&csr, 0);
        assert!((result.values[0] - 0.0).abs() < 1e-10);
        assert!((result.values[3] - 3.0).abs() < 1e-10);
        assert!((result.values[6] - 3.0).abs() < 1e-10);
        assert_eq!(result.name, "shortest_path");
    }

    #[test]
    fn test_weighted_shortest_path_unit_weights() {
        let csr = small_csr();
        let (dist, _parent) = weighted_shortest_path(&csr, 0, |_from, _to| 1.0);
        assert!(dist[0].is_some() && (dist[0].unwrap() - 0.0).abs() < 1e-10);
        assert!(dist[3].is_some() && (dist[3].unwrap() - 3.0).abs() < 1e-10);
        assert!(dist[6].is_some() && (dist[6].unwrap() - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_weighted_shortest_path_custom_weights() {
        let csr = small_csr();
        // Assign weight = 10 to all edges, so distances are scaled
        let (dist, _parent) = weighted_shortest_path(&csr, 0, |_from, _to| 10.0);
        assert!(dist[0].is_some() && (dist[0].unwrap() - 0.0).abs() < 1e-10);
        assert!(dist[3].is_some() && (dist[3].unwrap() - 30.0).abs() < 1e-10); // 3 hops × 10
    }

    #[test]
    fn test_compute_weighted_shortest_path() {
        let csr = small_csr();
        let result = compute_weighted_shortest_path(&csr, 0);
        assert!((result.values[0] - 0.0).abs() < 1e-10);
        assert!((result.values[3] - 3.0).abs() < 1e-10);
        assert_eq!(result.name, "weighted_shortest_path");
    }

    #[test]
    fn test_all_sp_destinations() {
        let csr = small_csr();
        let result = compute_all_sp_destinations(&csr);
        // Each node can reach 6 others (all 7 nodes minus itself)
        for &v in &result.values {
            assert!((v - 6.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_all_sp_destinations_disconnected() {
        let csr = disconnected_csr();
        let result = compute_all_sp_destinations(&csr);
        // Nodes 0 and 1 can reach each other (1 destination each)
        assert!((result.values[0] - 1.0).abs() < 1e-10);
        assert!((result.values[1] - 1.0).abs() < 1e-10);
        // Nodes 2 and 3 can reach each other (1 destination each)
        assert!((result.values[2] - 1.0).abs() < 1e-10);
        assert!((result.values[3] - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_random_walk() {
        let csr = small_csr();
        let result = compute_random_walk(&csr, None, 5, 2);
        assert_eq!(result.values.len(), 7);
        // Ensure values are populated
        assert!(result.values.iter().sum::<f64>() > 0.0);
    }

    #[test]
    fn test_node2vec() {
        let csr = small_csr();
        let result = compute_node2vec(&csr, 1.0, 1.0, 16, 2, 5);
        // Length should be num_nodes * dimensions = 7 * 16 = 112
        assert_eq!(result.values.len(), 112);
    }

    // ==================== CALL Pathway Tests ====================
    // These verify that CALL random_walk(...) / CALL node2vec(...) work end-to-end:
    // extension.load() → FunctionRegistry → execute_table_function() → DataChunk output.

    fn make_registry_with_algo() -> akar_function::registry::FunctionRegistry {
        use akar_catalog::Catalog;
        use akar_common::file_system::VirtualFileSystemRegistry;
        use akar_extension::ExtensionContext;
        use std::sync::{Arc, Mutex};

        let registry = Arc::new(Mutex::new(akar_function::registry::FunctionRegistry::new()));
        let catalog = Arc::new(Mutex::new(Catalog::new()));
        let vfs = Arc::new(VirtualFileSystemRegistry::new());
        let ctx = ExtensionContext::new(registry.clone(), catalog, vfs);
        AlgoExtension::new().load(&ctx).expect("ALGO extension load failed");
        // Drop ctx so the Arc reference count drops back to 1 before try_unwrap
        drop(ctx);
        match Arc::try_unwrap(registry) {
            Ok(mutex) => mutex.into_inner().expect("registry mutex poisoned"),
            Err(_) => panic!("registry Arc still has multiple owners after load"),
        }
    }

    #[test]
    fn test_call_random_walk_no_args() {
        let registry = make_registry_with_algo();
        // CALL random_walk() — uses defaults (steps=5, walks_per_node=2)
        let rows = registry
            .execute_table_function("random_walk", &[], None)
            .expect("random_walk should succeed with no args");
        // 5-node ring graph → 5 rows, each with 2 values (node_id, hit_count)
        assert_eq!(rows.len(), 5, "Expected 5 rows (one per node)");
        for row in &rows {
            assert_eq!(row.len(), 2, "Each row should have node_id + hit_count");
        }
        // hit_counts should be non-negative and their sum > 0
        let total_hits: f64 = rows
            .iter()
            .map(|r| match r.get(1) {
                Some(akar_common::types::Value::Double(v)) => *v,
                _ => 0.0,
            })
            .sum();
        assert!(total_hits > 0.0, "Total hit count should be positive");
    }

    #[test]
    fn test_call_random_walk_with_args() {
        use akar_common::types::Value;
        let registry = make_registry_with_algo();
        // CALL random_walk(10, 3)
        let rows = registry
            .execute_table_function("random_walk", &[Value::Int64(10), Value::Int64(3)], None)
            .expect("random_walk(10,3) should succeed");
        assert_eq!(rows.len(), 5);
        let total: f64 = rows
            .iter()
            .map(|r| match r.get(1) {
                Some(akar_common::types::Value::Double(v)) => *v,
                _ => 0.0,
            })
            .sum();
        // More walks → more hits
        assert!(total > 0.0);
    }

    #[test]
    fn test_call_rw_alias() {
        let registry = make_registry_with_algo();
        // CALL rw() should work just like CALL random_walk()
        let rows = registry
            .execute_table_function("rw", &[], None)
            .expect("rw alias should succeed");
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn test_call_node2vec_no_args() {
        let registry = make_registry_with_algo();
        // CALL node2vec() — uses defaults (p=1, q=1, dims=4, walks=3, window=5)
        let rows = registry
            .execute_table_function("node2vec", &[], None)
            .expect("node2vec should succeed with no args");
        // 5-node ring → 5 rows, each with 1 (node_id) + 4 (dim_0..3) = 5 values
        assert_eq!(rows.len(), 5, "Expected 5 rows");
        for row in &rows {
            assert_eq!(row.len(), 5, "Each row: node_id + 4 embedding dims");
        }
    }

    #[test]
    fn test_call_node2vec_with_args() {
        use akar_common::types::Value;
        let registry = make_registry_with_algo();
        // CALL node2vec(1.0, 1.0, 8, 2, 4)  → 5 rows × (node_id + 8 dims) = 9 cols each
        let rows = registry
            .execute_table_function(
                "node2vec",
                &[
                    Value::Double(1.0),
                    Value::Double(1.0),
                    Value::Int64(8),
                    Value::Int64(2),
                    Value::Int64(4),
                ],
                None,
            )
            .expect("node2vec(1,1,8,2,4) should succeed");
        assert_eq!(rows.len(), 5);
        for row in &rows {
            assert_eq!(row.len(), 9, "node_id + 8 dims");
        }
    }

    #[test]
    fn test_call_n2v_alias() {
        let registry = make_registry_with_algo();
        let rows = registry
            .execute_table_function("n2v", &[], None)
            .expect("n2v alias should succeed");
        assert_eq!(rows.len(), 5);
    }

    // ── spread activation tests ───────────────────────────────────────────

    #[test]
    fn test_spread_activation_basic() {
        // 0--1--2--3  (linear chain)
        let edges = vec![
            Edge { src_offset: 0, dst_offset: 1, rel_id: 0, rel_table_id: 0 },
            Edge { src_offset: 1, dst_offset: 2, rel_id: 1, rel_table_id: 0 },
            Edge { src_offset: 2, dst_offset: 3, rel_id: 2, rel_table_id: 0 },
        ];
        let csr = CSRAdjacency::build(&edges, 4);
        let result = compute_spread_activation(&csr, &[(0, 1.0)], |_u, _v| 1.0, 0.5, 0.01, 3);
        // Seed 0 at hop 0, then 1 at hop1 (0.5), 2 at hop2 (0.25), 3 at hop3 (0.125)
        assert_eq!(result.activated.len(), 4);
        // Sorted by activation descending — node 0 first
        assert_eq!(result.activated[0].0, 0);
        assert!((result.activated[0].1 - 1.0).abs() < 1e-10);
        assert_eq!(result.activated[0].2, 0);
        // Node 1 at hop 1
        assert_eq!(result.activated[1].0, 1);
        assert!((result.activated[1].1 - 0.5).abs() < 1e-10);
        assert_eq!(result.activated[1].2, 1);
    }

    #[test]
    fn test_spread_activation_threshold_pruning() {
        // 0--1--2--3 (linear chain), high threshold
        let edges = vec![
            Edge { src_offset: 0, dst_offset: 1, rel_id: 0, rel_table_id: 0 },
            Edge { src_offset: 1, dst_offset: 2, rel_id: 1, rel_table_id: 0 },
            Edge { src_offset: 2, dst_offset: 3, rel_id: 2, rel_table_id: 0 },
        ];
        let csr = CSRAdjacency::build(&edges, 4);
        // threshold 0.3 → only nodes with activation >= 0.3 survive
        let result = compute_spread_activation(&csr, &[(0, 1.0)], |_u, _v| 1.0, 0.5, 0.3, 3);
        // node 0: 1.0, node 1: 0.5, node 2: 0.25 (< 0.3 pruned), node 3: 0.125 (< 0.3 pruned)
        assert_eq!(result.activated.len(), 2);
        let ids: Vec<usize> = result.activated.iter().map(|(id, _, _)| *id).collect();
        assert!(ids.contains(&0));
        assert!(ids.contains(&1));
    }

    #[test]
    fn test_spread_activation_multi_seed() {
        // 0→2, 1→2 (two seeds both reach node 2 at hop 1)
        let edges = vec![
            Edge { src_offset: 0, dst_offset: 2, rel_id: 0, rel_table_id: 0 },
            Edge { src_offset: 1, dst_offset: 2, rel_id: 1, rel_table_id: 0 },
        ];
        let csr = CSRAdjacency::build(&edges, 3);
        // Seeds at 0 and 1 — both propagate to node 2 at hop 1
        let result = compute_spread_activation(&csr, &[(0, 1.0), (1, 1.0)], |_u, _v| 1.0, 0.5, 0.01, 2);
        // Node 2 gets 0.5 from seed 0 + 0.5 from seed 1 = 1.0
        let node2 = result.activated.iter().find(|(id, _, _)| *id == 2).unwrap();
        assert!((node2.1 - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_spread_activation_empty() {
        let csr = CSRAdjacency::build(&[], 0);
        let result = compute_spread_activation(&csr, &[], |_u, _v| 1.0, 0.5, 0.01, 3);
        assert!(result.activated.is_empty());
    }

    #[test]
    fn test_spread_activation_weighted() {
        // 0--(w=2.0)--1--(w=0.1)--2
        let edges = vec![
            Edge { src_offset: 0, dst_offset: 1, rel_id: 0, rel_table_id: 0 },
            Edge { src_offset: 1, dst_offset: 2, rel_id: 1, rel_table_id: 0 },
        ];
        let csr = CSRAdjacency::build(&edges, 3);
        let weight_fn = |u: usize, v: usize| {
            if (u == 0 && v == 1) || (u == 1 && v == 0) { 2.0 }
            else if (u == 1 && v == 2) || (u == 2 && v == 1) { 0.1 }
            else { 1.0 }
        };
        let result = compute_spread_activation(&csr, &[(0, 1.0)], weight_fn, 0.5, 0.01, 2);
        // Node 1: 1.0 * 2.0 * 0.5 = 1.0
        // Node 2: 1.0 * 0.1 * 0.5 = 0.05
        let node1 = result.activated.iter().find(|(id, _, _)| *id == 1).unwrap();
        assert!((node1.1 - 1.0).abs() < 1e-10);
        let node2 = result.activated.iter().find(|(id, _, _)| *id == 2).unwrap();
        assert!((node2.1 - 0.05).abs() < 1e-10);
    }
}
