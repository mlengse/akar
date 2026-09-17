# Deep Exploration — akar-function

Function registration is Akar's expression extension point: 259 built-in functions (244 scalar, 14 aggregate, 1 table function) plus user-defined registration via the public API. Families cover strings, numbers, date/time, JSON, lists, maps, casting, type predicates, and the vector metrics that the ANN layer relies on (`cosine_similarity`, `euclidean_distance`, `dot_product`, `l2_distance`, `vector_to_...`).

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `FunctionRegistry` | Name → implementation dispatch for 259 functions | `akar-core/akar-function/src/lib.rs` |
| scalar functions | Arithmetic, string, date/time, JSON, cast, type-check | `akar-core/akar-function/src/` |
| aggregate functions | Count, sum, avg, min/max, list-multiplicity aggregators | `akar-core/akar-function/src/` |
| vector functions | cosine_similarity / euclidean_distance / dot_product / l2_distance / vector metrics | `akar-core/akar-function/src/` |
| custom UDF registration | Extensions can register their own scalars/aggregates | `akar-core/akar-function/src/` |

## Design Decisions

- **Central registry with lazy resolution.** Function lookup is by name at plan/execute time through one registry, so extensions (`akar-extension`) append their own functions without touching the core crate. Alternative: per-crate hardcoded function tables — rejected because extensions couldn't add functions independently.
- **Equal footing for vector metrics.** `cosine_similarity` etc. are ordinary registered functions, so the planner's vector rewrite (`planner.rs:211`) can detect them syntactically and swap in the ANN scan.

## Why It Matters

Every `WHERE`, `RETURN`, and `HAVING` expression is evaluated through these functions. The vector metrics specifically are what make in-database semantic similarity possible. UDF support via the extension framework is what lets `akar-algo` expose `page_rank()` and `akar-json` expose `json_extract()` as first-class SQL functions.