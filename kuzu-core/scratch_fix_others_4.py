import sys

def replace_exact(filename, old, new):
    content = open(filename, 'r', encoding='utf-8').read()
    if old in content:
        content = content.replace(old, new)
        open(filename, 'w', encoding='utf-8').write(content)
        print(f'Replaced in {filename}')
    else:
        print(f'Old not found in {filename}')

old_join = """        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            field_names: vec![],
            size: output_size,
        }])"""
new_join = """        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            field_names: vec![],
            size: output_size,
            sel_vector: None,
        }])"""
replace_exact('kuzu-processor/src/physical/join_ops.rs', old_join, new_join)

# vectorsimilarityscan.rs missing sel_vector
old_vss = """        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            size: num_results,
            field_names: vec![],
        }])"""
new_vss = """        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            size: num_results,
            field_names: vec![],
            sel_vector: None,
        }])"""
replace_exact('kuzu-processor/src/physical/write_ops/vectorsimilarityscan.rs', old_vss, new_vss)

# copyfrom.rs
old_cf = """            chunks.push(DataChunk {
                fields: arrow_fields,
                field_types: arrow_field_types,
                field_names: chunk.field_names.clone(),
                size: chunk.size,
            });"""
new_cf = """            chunks.push(DataChunk {
                fields: arrow_fields,
                field_types: arrow_field_types,
                field_names: chunk.field_names.clone(),
                size: chunk.size,
                sel_vector: None,
            });"""
replace_exact('kuzu-processor/src/physical/write_ops/copyfrom.rs', old_cf, new_cf)

# physicalexplain.rs
old_pe = """        let chunk = DataChunk {
            fields: vec![kuzu_common::arrow_vector::ArrowVector::from_legacy(&vv).array],
            field_types: vec![kuzu_common::types::PhysicalTypeID::String],
            field_names: vec!["explain".into()],
            size: 1,
        };"""
new_pe = """        let chunk = DataChunk {
            fields: vec![kuzu_common::arrow_vector::ArrowVector::from_legacy(&vv).array],
            field_types: vec![kuzu_common::types::PhysicalTypeID::String],
            field_names: vec!["explain".into()],
            size: 1,
            sel_vector: None,
        };"""
replace_exact('kuzu-processor/src/physical/write_ops/physicalexplain.rs', old_pe, new_pe)

# recursiveextend.rs weighted
old_rec_w = """            Ok(vec![DataChunk {
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
new_rec_w = old_rec_w.replace('size: num_results,', 'size: num_results,\n                sel_vector: None,')
replace_exact('kuzu-processor/src/physical/write_ops/recursiveextend.rs', old_rec_w, new_rec_w)

old_rec_uw = """            Ok(vec![DataChunk {
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
new_rec_uw = old_rec_uw.replace('size: num_results,', 'size: num_results,\n                sel_vector: None,')
replace_exact('kuzu-processor/src/physical/write_ops/recursiveextend.rs', old_rec_uw, new_rec_uw)

old_rec_path = """            output.push(DataChunk {
                fields: arrow_fields,
                field_types: arrow_field_types,
                field_names,
                size: chunk.size,
            });"""
new_rec_path = old_rec_path.replace('size: chunk.size,', 'size: chunk.size,\n                sel_vector: None,')
replace_exact('kuzu-processor/src/physical/write_ops/recursiveextend.rs', old_rec_path, new_rec_path)
