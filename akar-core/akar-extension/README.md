# Akar Extension Framework

Extension trait (CustomScalar/CustomTable) and registry for loading and dispatching third-party extensions.

**Features:**
- `Extension` trait with `name()`, `initialize()` lifecycle
- `ExtensionRegistry` for loading/managing extensions
- `CustomScalar` — closure-based scalar functions (`Arc<dyn Fn(&[Value]) -> Result<Value, String>>`)
- `CustomTable` — closure-based table functions (`Arc<dyn Fn(&[Value], &mut DataChunk) -> Result<(), String>>`)
- `ExtensionContext` — shared context with function registry access

**Tests:** 15 (extension framework registry)
