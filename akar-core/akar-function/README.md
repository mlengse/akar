# Akar Function

Built-in function registry with scalar, aggregate, and table function dispatch.

**Registered functions (259):** 244 scalar + 14 aggregate + 1 table. Includes arithmetic, comparison, boolean, string, date/time, cast, list, map, struct, array, path, schema/utility, and 14 aggregates.

**Custom function support:**
- `CustomScalar` — closure-based scalar functions via extension framework
- `CustomTable` — closure-based table functions

**Aggregate state machine:** `AggValueState` enum with Count, Sum, Avg, Min, Max, Collect variants. Finalize() produces typed Value.

**Tests:** 176
