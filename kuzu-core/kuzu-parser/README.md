# Kuzu Parser

Cypher query parser using pest.rs PEG grammar.

**Supported syntax:**
- `CREATE NODE TABLE` / `CREATE REL TABLE` / `DROP TABLE`
- `MATCH` with node and edge patterns
- `WHERE` with boolean expressions
- `RETURN` with expressions
- Full operator precedence (OR, AND, NOT, comparison, arithmetic, unary)
- Literals: strings, integers, floats, booleans, null, lists, maps
- Function calls with arguments
- Property access (`a.name`)

**Tests:** 12
