c = open('kuzu-algo/src/lib.rs', 'r', encoding='utf-8').read()
c = c.replace('arrow_array::array::ArrayRef', 'arrow::array::ArrayRef')
c = c.replace('arrow_array::array::Array', 'arrow::array::Array')
open('kuzu-algo/src/lib.rs', 'w', encoding='utf-8').write(c)

c = open('kuzu-httpfs/Cargo.toml', 'r', encoding='utf-8').read()
c = c.replace('arrow-array = "52.0.0"', 'arrow.workspace = true')
open('kuzu-httpfs/Cargo.toml', 'w', encoding='utf-8').write(c)

c = open('kuzu-algo/Cargo.toml', 'r', encoding='utf-8').read()
c = c.replace('arrow-array = "52.0.0"', 'arrow.workspace = true')
open('kuzu-algo/Cargo.toml', 'w', encoding='utf-8').write(c)

c = open('kuzu-httpfs/src/lib.rs', 'r', encoding='utf-8').read()
c = c.replace('arrow_array', 'arrow::array')
open('kuzu-httpfs/src/lib.rs', 'w', encoding='utf-8').write(c)
