import sys, re

def replace_in_file(filepath, operations):
    content = open(filepath, 'r', encoding='utf-8').read()
    for pattern, repl in operations:
        content = re.sub(pattern, repl, content, flags=re.MULTILINE | re.DOTALL)
    open(filepath, 'w', encoding='utf-8').write(content)

# join_ops.rs
replace_in_file('kuzu-processor/src/physical/join_ops.rs', [
    (r'\.and_then\(\|f\| f\.get_value\((.*?)\)\)', r'.and_then(|_| { /* this is wrong, need to fix */ None })'), # I'll do join_ops.rs manually
])

# copyfrom.rs
replace_in_file('kuzu-processor/src/physical/write_ops/copyfrom.rs', [
    (r'DataChunk::new\(vec!\[v\]\)', r'DataChunk::new(vec![kuzu_common::arrow_vector::ArrowVector::from_legacy(&v).array], vec![kuzu_common::types::PhysicalTypeID::Int64])'),
    (r'DataChunk::new\(vec!\[\]\)', r'DataChunk::new(vec![], vec![])'),
    (r'chunks\.push\(DataChunk \{\s*fields,\s*field_names: chunk\.field_names\.clone\(\),\s*size: chunk\.size,\s*\}\);', 
     r'let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\n            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\n            chunks.push(DataChunk { fields: arrow_fields, field_types: arrow_field_types, field_names: chunk.field_names.clone(), size: chunk.size });'),
])

# physicalexplain.rs
replace_in_file('kuzu-processor/src/physical/write_ops/physicalexplain.rs', [
    (r'let chunk = DataChunk \{\s*fields: vec!\[vv\],\s*field_names: vec!\["explain"\.into\(\)\],\s*size: 1,\s*\};',
     r'let chunk = DataChunk { fields: vec![kuzu_common::arrow_vector::ArrowVector::from_legacy(&vv).array], field_types: vec![kuzu_common::types::PhysicalTypeID::String], field_names: vec!["explain".into()], size: 1 };')
])

# recursiveextend.rs
replace_in_file('kuzu-processor/src/physical/write_ops/recursiveextend.rs', [
    (r'Ok\(vec\!\[DataChunk \{\s*fields: vec!\[src_v, dst_v, len_v, path_nodes_v, path_edges_v, cost_v\],\s*field_names: vec!\[[^\]]+cost_name,\s*\],\s*size: num_results,\s*\}\]\)',
     r'Ok(vec![DataChunk { fields: vec![ kuzu_common::arrow_vector::ArrowVector::from_legacy(&src_v).array, kuzu_common::arrow_vector::ArrowVector::from_legacy(&dst_v).array, kuzu_common::arrow_vector::ArrowVector::from_legacy(&len_v).array, kuzu_common::arrow_vector::ArrowVector::from_legacy(&path_nodes_v).array, kuzu_common::arrow_vector::ArrowVector::from_legacy(&path_edges_v).array, kuzu_common::arrow_vector::ArrowVector::from_legacy(&cost_v).array, ], field_types: vec![ kuzu_common::types::PhysicalTypeID::Int64, kuzu_common::types::PhysicalTypeID::Int64, kuzu_common::types::PhysicalTypeID::Int64, kuzu_common::types::PhysicalTypeID::List, kuzu_common::types::PhysicalTypeID::List, kuzu_common::types::PhysicalTypeID::Double, ], field_names: vec![ "src".into(), "dst".into(), "length".into(), "path_nodes".into(), "path_edges".into(), cost_name, ], size: num_results, }])'),
     
    (r'Ok\(vec\!\[DataChunk \{\s*fields: vec!\[src_v, dst_v, len_v, path_nodes_v, path_edges_v\],\s*field_names: vec!\[[^\]]+\],\s*size: num_results,\s*\}\]\)',
     r'Ok(vec![DataChunk { fields: vec![ kuzu_common::arrow_vector::ArrowVector::from_legacy(&src_v).array, kuzu_common::arrow_vector::ArrowVector::from_legacy(&dst_v).array, kuzu_common::arrow_vector::ArrowVector::from_legacy(&len_v).array, kuzu_common::arrow_vector::ArrowVector::from_legacy(&path_nodes_v).array, kuzu_common::arrow_vector::ArrowVector::from_legacy(&path_edges_v).array, ], field_types: vec![ kuzu_common::types::PhysicalTypeID::Int64, kuzu_common::types::PhysicalTypeID::Int64, kuzu_common::types::PhysicalTypeID::Int64, kuzu_common::types::PhysicalTypeID::List, kuzu_common::types::PhysicalTypeID::List, ], field_names: vec![ "src".into(), "dst".into(), "length".into(), "path_nodes".into(), "path_edges".into(), ], size: num_results, }])'),

    (r'output\.push\(DataChunk \{\s*fields,\s*field_names,\s*size: chunk\.size,\s*\}\);',
     r'let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\n            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\n            output.push(DataChunk { fields: arrow_fields, field_types: arrow_field_types, field_names, size: chunk.size, });')
])

