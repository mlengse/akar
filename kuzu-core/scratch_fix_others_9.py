import re

# types.rs
c = open('kuzu-processor/src/physical/types.rs', 'r', encoding='utf-8').read()
c = c.replace('field.data()[', 'field.to_data().buffers()[0].as_slice()[')
open('kuzu-processor/src/physical/types.rs', 'w', encoding='utf-8').write(c)

# scan.rs
c = open('kuzu-processor/src/physical/scan_filter/scan.rs', 'r', encoding='utf-8').read()
c = c.replace('DataChunk::new(vec![])', 'DataChunk::new(vec![], vec![])')
c = re.sub(r'id_vec\.get_i64\(row\)', r'chunk.get_i64(self.id_column_idx, row)', c)
open('kuzu-processor/src/physical/scan_filter/scan.rs', 'w', encoding='utf-8').write(c)

# pathpropertyprobe.rs
c = open('kuzu-processor/src/physical/scan_filter/pathpropertyprobe.rs', 'r', encoding='utf-8').read()
c = re.sub(r'let path_val = node_ids_field\.get_value\(row\);', r'let path_val = chunk.get_value(col, row);', c)
c = re.sub(r'fields\.push\(fv\);', r'fields.push(kuzu_common::arrow_vector::ArrowVector::from_legacy(&fv).array);\n                field_types.push(fv.physical_type());', c)
c = c.replace('let mut fields: Vec<ValueVector> = Vec::with_capacity(chunk.fields.len());', 'let mut fields: Vec<kuzu_common::arrow_vector::ArrayRef> = Vec::with_capacity(chunk.fields.len());\n                let mut field_types = Vec::with_capacity(chunk.fields.len());')
c = c.replace('DataChunk {\\n                fields,\\n                field_names: chunk.field_names.clone(),\\n                size: chunk.size,\\n            }',
              'DataChunk {\\n                fields,\\n                field_types,\\n                field_names: chunk.field_names.clone(),\\n                size: chunk.size,\\n                sel_vector: None,\\n            }')
open('kuzu-processor/src/physical/scan_filter/pathpropertyprobe.rs', 'w', encoding='utf-8').write(c)

# scanrel.rs
c = open('kuzu-processor/src/physical/scan_filter/scanrel.rs', 'r', encoding='utf-8').read()
c = c.replace('Ok(vec![DataChunk {\\n                fields,\\n                field_names: self.field_names.clone(),\\n                size: output_size,\\n                sel_vector: None,\\n            }])',
              'let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\\n            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\\n            Ok(vec![DataChunk {\\n                fields: arrow_fields,\\n                field_types: arrow_field_types,\\n                field_names: self.field_names.clone(),\\n                size: output_size,\\n                sel_vector: None,\\n            }])')
open('kuzu-processor/src/physical/scan_filter/scanrel.rs', 'w', encoding='utf-8').write(c)

# projection.rs
c = open('kuzu-processor/src/physical/scan_filter/projection.rs', 'r', encoding='utf-8').read()
c = c.replace('f.size()', 'f.len()')
c = c.replace('DataChunk {\\n                    fields,\\n                    field_names: chunk.field_names.clone(),\\n                    size: chunk.size,\\n                    sel_vector: None,\\n                }',
              'DataChunk {\\n                    fields,\\n                    field_types,\\n                    field_names: chunk.field_names.clone(),\\n                    size: chunk.size,\\n                    sel_vector: None,\\n                }')
open('kuzu-processor/src/physical/scan_filter/projection.rs', 'w', encoding='utf-8').write(c)

# primarykeyscan.rs
c = open('kuzu-processor/src/physical/scan_filter/primarykeyscan.rs', 'r', encoding='utf-8').read()
c = c.replace('key_column_idxumn_idx', 'key_column_idx')
open('kuzu-processor/src/physical/scan_filter/primarykeyscan.rs', 'w', encoding='utf-8').write(c)

# copyfrom.rs
c = open('kuzu-processor/src/physical/write_ops/copyfrom.rs', 'r', encoding='utf-8').read()
c = re.sub(r'chunks\.push\(DataChunk \{\s*fields,\s*field_names: chunk\.field_names\.clone\(\),\s*size: chunk\.size,\s*\}\);', 
           r'let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\n            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\n            chunks.push(DataChunk { fields: arrow_fields, field_types: arrow_field_types, field_names: chunk.field_names.clone(), size: chunk.size, sel_vector: None });', c)
open('kuzu-processor/src/physical/write_ops/copyfrom.rs', 'w', encoding='utf-8').write(c)

# recursiveextend.rs
c = open('kuzu-processor/src/physical/write_ops/recursiveextend.rs', 'r', encoding='utf-8').read()
c = re.sub(r'output\.push\(DataChunk \{\s*fields,\s*field_names,\s*size: chunk\.size,\s*\}\);', 
           r'let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\n            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\n            output.push(DataChunk { fields: arrow_fields, field_types: arrow_field_types, field_names, size: chunk.size, sel_vector: None });', c)
open('kuzu-processor/src/physical/write_ops/recursiveextend.rs', 'w', encoding='utf-8').write(c)

