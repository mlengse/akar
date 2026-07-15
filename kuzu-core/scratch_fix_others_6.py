import re

# primarykeyscan.rs
p = 'kuzu-processor/src/physical/scan_filter/primarykeyscan.rs'
c = open(p, 'r', encoding='utf-8').read()
c = c.replace('DataChunk {\\n            fields: vec![', 
              'DataChunk {\\n            field_types: vec![kuzu_common::types::PhysicalTypeID::Int64],\\n            fields: vec![')
c = c.replace('DataChunk {\\n            fields: arrow_fields', 
              'DataChunk {\\n            fields: arrow_fields,\\n            field_types: arrow_field_types')
open(p, 'w', encoding='utf-8').write(c)

# copyfrom.rs
p = 'kuzu-processor/src/physical/write_ops/copyfrom.rs'
c = open(p, 'r', encoding='utf-8').read()
c = c.replace('chunks.push(DataChunk {\\n                fields,\\n                field_names: chunk.field_names.clone(),\\n                size: chunk.size,\\n                sel_vector: None,\\n            });',
              'let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\\n            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\\n            chunks.push(DataChunk {\\n                fields: arrow_fields,\\n                field_types: arrow_field_types,\\n                field_names: chunk.field_names.clone(),\\n                size: chunk.size,\\n                sel_vector: None,\\n            });')
open(p, 'w', encoding='utf-8').write(c)

# recursiveextend.rs
p = 'kuzu-processor/src/physical/write_ops/recursiveextend.rs'
c = open(p, 'r', encoding='utf-8').read()
c = c.replace('output.push(DataChunk {\\n                fields,\\n                field_names,\\n                size: chunk.size,\\n                sel_vector: None,\\n            });',
              'let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\\n            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\\n            output.push(DataChunk {\\n                fields: arrow_fields,\\n                field_types: arrow_field_types,\\n                field_names,\\n                size: chunk.size,\\n                sel_vector: None,\\n            });')
open(p, 'w', encoding='utf-8').write(c)

# orderby.rs
p = 'kuzu-processor/src/physical/order_aggregate/orderby.rs'
c = open(p, 'r', encoding='utf-8').read()
c = re.sub(r'field\.get_value\(row\)\.unwrap_or\(Value::Null\)', r'chunk.get_value(col, row).unwrap_or(Value::Null)', c)
c = re.sub(r'output\.push\(DataChunk::new\((.*?)\)\);', r'let arrow_fields = \1.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\n            let arrow_field_types = \1.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\n            output.push(DataChunk::new(arrow_fields, arrow_field_types));', c)
open(p, 'w', encoding='utf-8').write(c)

# topk.rs
p = 'kuzu-processor/src/physical/order_aggregate/topk.rs'
c = open(p, 'r', encoding='utf-8').read()
c = re.sub(r'field\.get_value\(row\)\.unwrap_or\(Value::Null\)', r'chunk.get_value(col, row).unwrap_or(Value::Null)', c)
c = re.sub(r'output\.push\(DataChunk::new\((.*?)\)\);', r'let arrow_fields = \1.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\n            let arrow_field_types = \1.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\n            output.push(DataChunk::new(arrow_fields, arrow_field_types));', c)
open(p, 'w', encoding='utf-8').write(c)

# aggregatehashtable.rs
p = 'kuzu-processor/src/physical/order_aggregate/aggregatehashtable.rs'
c = open(p, 'r', encoding='utf-8').read()
c = c.replace('DataChunk::new(Vec::with_capacity(num_cols))', 'DataChunk::new(Vec::with_capacity(num_cols), Vec::with_capacity(num_cols))')
c = re.sub(r'chunks\.push\(DataChunk::new\(chunk_fields\)\);', r'let arrow_fields = chunk_fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\n            let arrow_field_types = chunk_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\n            chunks.push(DataChunk::new(arrow_fields, arrow_field_types));', c)
c = re.sub(r'\.and_then\(\|f\| f\.get_value\(row\)\)', r'.map(|_| chunk.get_value(col, row)).unwrap_or(Some(Value::Null))', c)
# Note: actually for `aggregatehashtable.rs`, `f` was `chunk.fields.get(col)`. I'll replace more carefully.
c = re.sub(r'chunk\.fields\.get\(col\)\s*\.and_then\(\|f\| f\.get_value\(row\)\)', r'chunk.fields.get(col).map(|_| chunk.get_value(col, row)).unwrap_or(Some(Value::Null))', c)
open(p, 'w', encoding='utf-8').write(c)

