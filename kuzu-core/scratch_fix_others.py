import sys, re

content = open('kuzu-processor/src/physical/write_ops/copyfrom.rs', 'r', encoding='utf-8').read()

content = content.replace('DataChunk::new(vec![v])', 'DataChunk::new(vec![kuzu_common::arrow_vector::ArrowVector::from_legacy(&v).array], vec![kuzu_common::types::PhysicalTypeID::Int64])')
content = content.replace('DataChunk::new(vec![])', 'DataChunk::new(vec![], vec![])')

old_chunk = """            chunks.push(DataChunk {
                fields,
                field_names: chunk.field_names.clone(),
                size: chunk.size,
            });"""
new_chunk = """            let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();
            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
            chunks.push(DataChunk {
                fields: arrow_fields,
                field_types: arrow_field_types,
                field_names: chunk.field_names.clone(),
                size: chunk.size,
            });"""
content = content.replace(old_chunk, new_chunk)

open('kuzu-processor/src/physical/write_ops/copyfrom.rs', 'w', encoding='utf-8').write(content)

content = open('kuzu-processor/src/physical/write_ops/physicalexplain.rs', 'r', encoding='utf-8').read()
old_explain = """        let chunk = DataChunk {
            fields: vec![vv],
            field_names: vec!["explain".into()],
            size: 1,
        };"""
new_explain = """        let chunk = DataChunk {
            fields: vec![kuzu_common::arrow_vector::ArrowVector::from_legacy(&vv).array],
            field_types: vec![kuzu_common::types::PhysicalTypeID::String],
            field_names: vec!["explain".into()],
            size: 1,
        };"""
content = content.replace(old_explain, new_explain)
open('kuzu-processor/src/physical/write_ops/physicalexplain.rs', 'w', encoding='utf-8').write(content)
