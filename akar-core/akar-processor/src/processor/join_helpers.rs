use akar_common::vector::DataChunk;
use akar_parser::ast::Expression;

/// Resolved join key binding for one equality condition: which build/probe
/// columns participate, and whether the column holds a map/struct from which a
/// key must be extracted (e.g. `UNWIND $rows AS row MATCH (m:Memory
/// {id: row.id})` binds the probe column `row` with map key `id`, P53.26).
#[derive(Debug, Clone)]
pub struct JoinKeyBinding {
    pub build_col: u32,
    pub probe_col: u32,
    /// Map key to extract from the build-side column value (None = raw cell).
    pub build_map_key: Option<String>,
    /// Map key to extract from the probe-side column value (None = raw cell).
    pub probe_map_key: Option<String>,
}

/// Resolve the build/probe column indices for a join condition. Returns one
/// binding per `Equal` key.
///
/// Column resolution tries, in order:
/// 1. the fully-qualified property (`m.id` / `row.id`),
/// 2. the bare property name (`id` — scan columns use plain column names),
/// 3. a variable column holding map/struct values, with the key to extract
///    (`row` + key `id`).
///
/// The cross-side fallback (try the opposite side's expression) preserves the
/// legacy behaviour when chunk field names are asymmetric.
pub fn derive_join_bindings(
    join_keys: &[Expression],
    build_chunks: &[DataChunk],
    probe_chunks: &[DataChunk],
) -> Vec<JoinKeyBinding> {
    let build_names: Vec<&str> = build_chunks
        .first()
        .map(|c| c.field_names.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    let probe_names: Vec<&str> = probe_chunks
        .first()
        .map(|c| c.field_names.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();

    let mut bindings = Vec::new();
    for key in join_keys {
        if let Expression::BinaryOp(akar_parser::ast::BinaryOp::Equal, left, right) = key {
            let (lp, lvar, lprop) = split_prop(left);
            let (rp, rvar, rprop) = split_prop(right);

            let (build_col, build_map_key) = resolve_side(&lp, &lvar, &lprop, &build_names)
                .or_else(|| resolve_side(&rp, &rvar, &rprop, &build_names))
                .unwrap_or((0, None));
            let (probe_col, probe_map_key) = resolve_side(&rp, &rvar, &rprop, &probe_names)
                .or_else(|| resolve_side(&lp, &lvar, &lprop, &probe_names))
                .unwrap_or((0, None));

            bindings.push(JoinKeyBinding {
                build_col: build_col as u32,
                probe_col: probe_col as u32,
                build_map_key,
                probe_map_key,
            });
        }
    }

    if bindings.is_empty() {
        bindings.push(JoinKeyBinding {
            build_col: 0,
            probe_col: 0,
            build_map_key: None,
            probe_map_key: None,
        });
    }
    bindings
}

/// Split a join-key expression into `(full_prop, base_variable, key)`.
/// A plain variable `iid` yields `("iid", "iid", "iid")`; a property access
/// `row.id` yields `("row.id", "row", "id")`.
fn split_prop(expr: &Expression) -> (String, String, String) {
    match expr {
        Expression::PropertyAccess(obj, prop) => {
            if let Expression::Variable(var) = &**obj {
                (format!("{var}.{prop}"), var.clone(), prop.clone())
            } else {
                (prop.clone(), String::new(), prop.clone())
            }
        }
        Expression::Variable(name) => (name.clone(), name.clone(), name.clone()),
        _ => (String::new(), String::new(), String::new()),
    }
}

/// Resolve one side of a join key against the side's chunk field names.
/// Returns `(column_index, map_key_to_extract)`.
fn resolve_side(full: &str, var: &str, key: &str, names: &[&str]) -> Option<(usize, Option<String>)> {
    if let Some(i) = names.iter().position(|n| *n == full) {
        return Some((i, None));
    }
    if let Some(i) = names.iter().position(|n| *n == key) {
        return Some((i, None));
    }
    if !var.is_empty() && let Some(i) = names.iter().position(|n| *n == var) {
        return Some((i, Some(key.to_string())));
    }
    None
}
