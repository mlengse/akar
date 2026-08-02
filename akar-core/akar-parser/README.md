# Akar Parser

Cypher query parser using pest.rs PEG grammar.

**Supported syntax:**
- `CREATE NODE TABLE` / `CREATE REL TABLE` / `DROP TABLE`
- `COPY FROM` with options (header, delimiter, quote, escape)
- `MATCH` / `OPTIONAL MATCH` with node and edge patterns
- `WHERE` / `HAVING` with boolean expressions
- `RETURN` / `WITH` with expressions, aliases, ordering
- `ORDER BY` / `LIMIT` / `OFFSET` / `SKIP`
- `DELETE` / `SET`
- `ALTER TABLE` (ADD/DROP/RENAME COLUMN, RENAME TABLE)
- `UNION ALL`
- `UNWIND` expression AS variable
- `CREATE` (node patterns with properties)
- Full operator precedence (OR, AND, NOT, comparison, arithmetic, unary)
- Literals: strings, integers, floats, booleans, null, lists, maps, parameters
- Function calls with arguments
- Property access (`a.name`)

**AST types:** 33 Statement variants, 10 Clause variants, Expression enum with 10 variants, Constant with 7 variants.

**Tests:** 67
