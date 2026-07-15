content = open('kuzu-processor/src/physical/scan_filter/primarykeyscan.rs', 'r', encoding='utf-8').read()
content = content.replace('::<Vec<_>>();\\n        let arrow_field_types', '::<Vec<_>>();\n        let arrow_field_types')
content = content.replace('::<Vec<_>>();\\n        DataChunk', '::<Vec<_>>();\n        DataChunk')
content = content.replace('Ok(vec![let arrow_fields =', 'let arrow_fields =')
content = content.replace('DataChunk { fields: arrow_fields, field_types: arrow_field_types,', 'Ok(vec![DataChunk { fields: arrow_fields, field_types: arrow_field_types,')
open('kuzu-processor/src/physical/scan_filter/primarykeyscan.rs', 'w', encoding='utf-8').write(content)

algo = open('kuzu-algo/src/lib.rs', 'r', encoding='utf-8').read()
algo = algo.replace('std::sync::Arc<dyn kuzu_common::arrow_vector::Array>', 'arrow_array::array::ArrayRef')
open('kuzu-algo/src/lib.rs', 'w', encoding='utf-8').write(algo)

algo_cargo = open('kuzu-algo/Cargo.toml', 'r', encoding='utf-8').read()
if 'arrow-array' not in algo_cargo:
    algo_cargo = algo_cargo.replace('[dependencies]', '[dependencies]\narrow-array = "52.0.0"')
    open('kuzu-algo/Cargo.toml', 'w', encoding='utf-8').write(algo_cargo)
