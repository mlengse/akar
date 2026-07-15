import re

def process_file(filepath, pattern, repl):
    c = open(filepath, 'r', encoding='utf-8').read()
    c = re.sub(pattern, repl, c, flags=re.DOTALL)
    open(filepath, 'w', encoding='utf-8').write(c)

# expression_evaluator.rs
# vec![elem_vec] was consumed in the first iter(). We can fix it by using &elem_vec.
c = open('kuzu-processor/src/expression_evaluator.rs', 'r', encoding='utf-8').read()
c = c.replace('vec![elem_vec].iter()', 'vec![&elem_vec].into_iter()')
c = c.replace('vec![acc_vec, elem_vec].iter()', 'vec![&acc_vec, &elem_vec].into_iter()')
open('kuzu-processor/src/expression_evaluator.rs', 'w', encoding='utf-8').write(c)

# pathpropertyprobe.rs
process_file('kuzu-processor/src/physical/scan_filter/pathpropertyprobe.rs',
             r'output\.push\(DataChunk \{\s*fields,\s*field_types,\s*field_names: chunk\.field_names\.clone\(\),\s*size: chunk\.size,\s*sel_vector: None,\s*\}\)',
             r'output.push(DataChunk { fields, field_types, field_names: chunk.field_names.clone(), size: chunk.size, sel_vector: None })')
# wait, actually the error was missing field_types in `pathpropertyprobe.rs`!
# Let's fix that.
process_file('kuzu-processor/src/physical/scan_filter/pathpropertyprobe.rs',
             r'output\.push\(DataChunk \{\s*fields,\s*field_names: chunk\.field_names\.clone\(\),\s*size: chunk\.size,\s*\}\);',
             r'output.push(DataChunk { fields, field_types, field_names: chunk.field_names.clone(), size: chunk.size, sel_vector: None });')


# copyfrom.rs
process_file('kuzu-processor/src/physical/write_ops/copyfrom.rs',
             r'chunks\.push\(DataChunk \{\s*fields,\s*field_names: chunk\.field_names\.clone\(\),\s*size: chunk\.size,\s*sel_vector: None,\s*\}\);',
             r'let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\n            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\n            chunks.push(DataChunk { fields: arrow_fields, field_types: arrow_field_types, field_names: chunk.field_names.clone(), size: chunk.size, sel_vector: None });')

process_file('kuzu-processor/src/physical/write_ops/copyfrom.rs',
             r'chunks\.push\(DataChunk \{\s*fields,\s*field_names: chunk\.field_names\.clone\(\),\s*size: chunk\.size,\s*\}\);',
             r'let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\n            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\n            chunks.push(DataChunk { fields: arrow_fields, field_types: arrow_field_types, field_names: chunk.field_names.clone(), size: chunk.size, sel_vector: None });')


# recursiveextend.rs
process_file('kuzu-processor/src/physical/write_ops/recursiveextend.rs',
             r'output\.push\(DataChunk \{\s*fields,\s*field_names,\s*size: chunk\.size,\s*sel_vector: None,\s*\}\);',
             r'let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\n            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\n            output.push(DataChunk { fields: arrow_fields, field_types: arrow_field_types, field_names, size: chunk.size, sel_vector: None });')

process_file('kuzu-processor/src/physical/write_ops/recursiveextend.rs',
             r'output\.push\(DataChunk \{\s*fields,\s*field_names,\s*size: chunk\.size,\s*\}\);',
             r'let arrow_fields = fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\n            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\n            output.push(DataChunk { fields: arrow_fields, field_types: arrow_field_types, field_names, size: chunk.size, sel_vector: None });')

