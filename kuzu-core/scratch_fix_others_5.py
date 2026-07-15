import re

content = open('kuzu-processor/src/physical/join_ops.rs', 'r', encoding='utf-8').read()

def fix_macro(m):
    return (f'let arrow_fields = {m.group(1)}.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>();\n'
            f'        let arrow_field_types = {m.group(2)}.iter().map(|v| v.physical_type()).collect::<Vec<_>>();\n'
            f'        Ok(vec![DataChunk {{ fields: arrow_fields, field_types: arrow_field_types, {m.group(3)} }}])')

content = re.sub(r'Ok\(vec!\[let arrow_fields = ([a-zA-Z_0-9]+)\.iter.*?\\n\s*let arrow_field_types = ([a-zA-Z_0-9]+)\.iter.*?\\n\s*DataChunk \{\s*fields: arrow_fields,\s*field_types: arrow_field_types,\s*(.*?)\}\]\)', fix_macro, content, flags=re.DOTALL)
open('kuzu-processor/src/physical/join_ops.rs', 'w', encoding='utf-8').write(content)

algo = open('kuzu-algo/src/lib.rs', 'r', encoding='utf-8').read()
algo = algo.replace('kuzu_common::arrow_vector::ArrayRef', 'std::sync::Arc<dyn kuzu_common::arrow_vector::Array>')
open('kuzu-algo/src/lib.rs', 'w', encoding='utf-8').write(algo)

httpfs = open('kuzu-httpfs/Cargo.toml', 'r', encoding='utf-8').read()
if 'arrow-array' not in httpfs:
    httpfs = httpfs.replace('[dependencies]', '[dependencies]\narrow-array = "52.0.0"')
    open('kuzu-httpfs/Cargo.toml', 'w', encoding='utf-8').write(httpfs)
