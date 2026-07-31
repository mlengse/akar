# Akar Function

Built-in function registry with scalar, aggregate, and table function dispatch.

**Registered functions (50+):**
- Arithmetic: `+`, `-`, `*`, `/`, `%`, `^`, `abs`, `ceil`, `floor`, `round`, `sqrt`, `power`, `gamma`, `log`, `log2`, `exp`, `sin`, `cos`, `tan`, `degrees`, `radians`
- String: `concat`, `contains`, `starts_with`, `ends_with`, `substr`, `length`, `lower`, `upper`, `trim`, `ltrim`, `rtrim`, `replace`, `reverse`, `repeat`, `split_part`, `left`, `right`
- Date/Timestamp: `date_part`, `date_trunc`, `day`, `month`, `year`, `hour`, `minute`, `second`
- Aggregate: `COUNT`, `COUNT(*)`, `SUM`, `AVG`, `MIN`, `MAX`, `COLLECT`, `STDDEV`, `VARIANCE`
- Conditional: `coalesce`, `ifnull`, `nullif`
- Type conversion: `CAST`

**Custom function support:**
- `CustomScalar` — closure-based scalar functions via extension framework
- `CustomTable` — closure-based table functions

**Aggregate state machine:** `AggValueState` enum with Count, Sum, Avg, Min, Max, Collect variants. Finalize() produces typed Value.

**Tests:** 70
