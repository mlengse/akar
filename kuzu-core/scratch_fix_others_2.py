import sys, re

content = open('kuzu-processor/src/physical/write_ops/foreach.rs', 'r', encoding='utf-8').read()
content = content.replace('DataChunk::new(vec![v])', 'DataChunk::new(vec![kuzu_common::arrow_vector::ArrowVector::from_legacy(&v).array], vec![kuzu_common::types::PhysicalTypeID::Int64])')
open('kuzu-processor/src/physical/write_ops/foreach.rs', 'w', encoding='utf-8').write(content)

content = open('kuzu-processor/src/physical/write_ops/vectorsimilarityscan.rs', 'r', encoding='utf-8').read()
content = content.replace('DataChunk::new(vec![])', 'DataChunk::new(vec![], vec![])')
old_chunk = """        Ok(vec![DataChunk {
            fields,
            field_names: vec!["node_id".into(), "score".into()],
            size: results.len(),
        }])"""
new_chunk = """        let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();
        let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            field_names: vec!["node_id".into(), "score".into()],
            size: results.len(),
        }])"""
content = content.replace(old_chunk, new_chunk)
open('kuzu-processor/src/physical/write_ops/vectorsimilarityscan.rs', 'w', encoding='utf-8').write(content)

content = open('kuzu-processor/src/physical/write_ops/recursiveextend.rs', 'r', encoding='utf-8').read()
content = content.replace('let offset = i64::from_le_bytes(field.to_data()[i * 8..i * 8 + 8].try_into().unwrap());', 'let offset = if let Some(Value::Int64(val)) = input[0].get_value(0, i) { val } else { 0 };')
content = content.replace('src_bytes.copy_from_slice(&src_vec.to_data()[i * 8..(i + 1) * 8]);', 'src_bytes.copy_from_slice(&src_vec.to_data().buffers()[0].as_slice()[i * 8..(i + 1) * 8]);')
content = content.replace('dst_bytes.copy_from_slice(&dst_vec.to_data()[i * 8..(i + 1) * 8]);', 'dst_bytes.copy_from_slice(&dst_vec.to_data().buffers()[0].as_slice()[i * 8..(i + 1) * 8]);')
content = content.replace('src_bytes.copy_from_slice(&src_vec_data[offset..offset + 8]);', 'src_bytes.copy_from_slice(&src_vec_data.buffers()[0].as_slice()[offset..offset + 8]);')

open('kuzu-processor/src/physical/write_ops/recursiveextend.rs', 'w', encoding='utf-8').write(content)

