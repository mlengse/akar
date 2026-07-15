import sys, re

content = open('kuzu-processor/src/physical/write_ops/recursiveextend.rs', 'r', encoding='utf-8').read()

# Fix empty datachunks
content = content.replace('DataChunk::new(vec![])', 'DataChunk::new(vec![], vec![])')
content = content.replace('.data()', '.to_data()')

# Fix chunk fields push
content = content.replace('chunk.fields.push(node_ids);', 'chunk.fields.push(kuzu_common::arrow_vector::ArrowVector::from_legacy(&node_ids).array);\n            chunk.field_types.push(kuzu_common::types::PhysicalTypeID::List);')

# Fix get_value/physical_type inside PathTracker
content = content.replace('chunk.fields[col].get_value(input_row)', 'chunk.get_value(col, input_row)')
content = content.replace('chunk.fields[col].physical_type()', 'chunk.field_types[col]')

# Fix DataChunk { fields: ... } to be DataChunk::new(fields, field_types)
old_weighted = """            Ok(vec![DataChunk {
                fields: vec![src_v, dst_v, len_v, path_nodes_v, path_edges_v, cost_v],
                field_names: vec![
                    "src".into(),
                    "dst".into(),
                    "length".into(),
                    "path_nodes".into(),
                    "path_edges".into(),
                    cost_name,
                ],
                size: num_results,
            }])"""

new_weighted = """            Ok(vec![DataChunk {
                fields: vec![
                    kuzu_common::arrow_vector::ArrowVector::from_legacy(&src_v).array,
                    kuzu_common::arrow_vector::ArrowVector::from_legacy(&dst_v).array,
                    kuzu_common::arrow_vector::ArrowVector::from_legacy(&len_v).array,
                    kuzu_common::arrow_vector::ArrowVector::from_legacy(&path_nodes_v).array,
                    kuzu_common::arrow_vector::ArrowVector::from_legacy(&path_edges_v).array,
                    kuzu_common::arrow_vector::ArrowVector::from_legacy(&cost_v).array,
                ],
                field_types: vec![
                    kuzu_common::types::PhysicalTypeID::Int64,
                    kuzu_common::types::PhysicalTypeID::Int64,
                    kuzu_common::types::PhysicalTypeID::Int64,
                    kuzu_common::types::PhysicalTypeID::List,
                    kuzu_common::types::PhysicalTypeID::List,
                    kuzu_common::types::PhysicalTypeID::Double,
                ],
                field_names: vec![
                    "src".into(),
                    "dst".into(),
                    "length".into(),
                    "path_nodes".into(),
                    "path_edges".into(),
                    cost_name,
                ],
                size: num_results,
            }])"""

content = content.replace(old_weighted, new_weighted)

old_unweighted = """            Ok(vec![DataChunk {
                fields: vec![src_v, dst_v, len_v, path_nodes_v, path_edges_v],
                field_names: vec![
                    "src".into(),
                    "dst".into(),
                    "length".into(),
                    "path_nodes".into(),
                    "path_edges".into(),
                ],
                size: num_results,
            }])"""

new_unweighted = """            Ok(vec![DataChunk {
                fields: vec![
                    kuzu_common::arrow_vector::ArrowVector::from_legacy(&src_v).array,
                    kuzu_common::arrow_vector::ArrowVector::from_legacy(&dst_v).array,
                    kuzu_common::arrow_vector::ArrowVector::from_legacy(&len_v).array,
                    kuzu_common::arrow_vector::ArrowVector::from_legacy(&path_nodes_v).array,
                    kuzu_common::arrow_vector::ArrowVector::from_legacy(&path_edges_v).array,
                ],
                field_types: vec![
                    kuzu_common::types::PhysicalTypeID::Int64,
                    kuzu_common::types::PhysicalTypeID::Int64,
                    kuzu_common::types::PhysicalTypeID::Int64,
                    kuzu_common::types::PhysicalTypeID::List,
                    kuzu_common::types::PhysicalTypeID::List,
                ],
                field_names: vec![
                    "src".into(),
                    "dst".into(),
                    "length".into(),
                    "path_nodes".into(),
                    "path_edges".into(),
                ],
                size: num_results,
            }])"""
content = content.replace(old_unweighted, new_unweighted)

old_path = """            output.push(DataChunk {
                fields,
                field_names,
                size: chunk.size,
            });"""

new_path = """            let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();
            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
            output.push(DataChunk {
                fields: arrow_fields,
                field_types: arrow_field_types,
                field_names,
                size: chunk.size,
            });"""
content = content.replace(old_path, new_path)

open('kuzu-processor/src/physical/write_ops/recursiveextend.rs', 'w', encoding='utf-8').write(content)
