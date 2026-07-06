//! Scalar function evaluation.
//!
//! Each function takes input `Value` slices and produces an output `Value`.

#![allow(
    clippy::unnecessary_cast,
    clippy::approx_constant,
    clippy::manual_is_multiple_of,
    clippy::clone_on_ref_ptr,
    clippy::collapsible_if,
    clippy::never_loop
)]

use crate::registry::*;
use kuzu_common::types::Value;

pub mod arithmetic;
pub mod comparison;
pub mod string;
pub mod date;
pub mod list;
pub mod map_struct;
pub mod boolean;
pub mod path;
pub mod hash;
pub mod interval;
pub mod union_funcs;
pub mod blob;
pub mod cast;
pub mod utility;
pub mod schema;
pub mod array;
pub mod utils;

pub(crate) use arithmetic::{evaluate_arithmetic, numeric_to_f64};
pub(crate) use comparison::evaluate_comparison;
pub(crate) use string::{evaluate_string, get_string};
pub(crate) use date::evaluate_date;
pub(crate) use list::evaluate_list;
pub(crate) use map_struct::{evaluate_map, evaluate_struct};
pub(crate) use boolean::evaluate_boolean;
pub(crate) use path::{evaluate_path, evaluate_uuid};
pub(crate) use hash::evaluate_hash;
pub(crate) use interval::evaluate_interval;
pub(crate) use union_funcs::evaluate_union;
pub(crate) use blob::evaluate_blob;
pub(crate) use cast::evaluate_cast;
pub(crate) use utility::evaluate_utility;
pub(crate) use schema::evaluate_schema;
pub(crate) use array::evaluate_array;
pub(crate) use utils::*;
pub use utils::set_rng_seed;


// ==================== Module-level utilities ====================

thread_local! {
    pub(crate) static RNG_STATE: std::cell::Cell<u64> = std::cell::Cell::new(
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(12345)
    );
}

/// Global regex cache — avoids recompiling regex patterns on every row.
///
/// Regex compilation is expensive (~10-50µs per pattern). For queries like
/// `WHERE regex_matches(text, 'pattern')`, this saves the compilation cost
/// for every row beyond the first.
pub(crate) static REGEX_CACHE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, regex::Regex>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

pub fn evaluate_scalar(func: &ScalarFunction, args: &[Value]) -> Result<Value, String> {
    match func {
        ScalarFunction::Arithmetic { op } => evaluate_arithmetic(*op, args),
        ScalarFunction::Comparison { op } => evaluate_comparison(*op, args),
        ScalarFunction::String { op } => evaluate_string(*op, args),
        ScalarFunction::Cast { target_type } => evaluate_cast(*target_type, args),
        ScalarFunction::Date { op } => evaluate_date(*op, args),
        ScalarFunction::List { op } => evaluate_list(*op, args),
        ScalarFunction::Map { op } => evaluate_map(*op, args),
        ScalarFunction::Struct { op } => evaluate_struct(*op, args),
        ScalarFunction::Boolean { op } => evaluate_boolean(*op, args),
        ScalarFunction::Utility { op } => evaluate_utility(*op, args),
        ScalarFunction::Schema { op } => evaluate_schema(*op, args),
        ScalarFunction::Array { op } => evaluate_array(*op, args),
        ScalarFunction::Path { op } => evaluate_path(*op, args),
        ScalarFunction::Hash { op } => evaluate_hash(*op, args),
        ScalarFunction::Interval { op } => evaluate_interval(*op, args),
        ScalarFunction::Blob { op } => evaluate_blob(*op, args),
        ScalarFunction::Union { op } => evaluate_union(*op, args),
        ScalarFunction::Uuid => evaluate_uuid(args),
        ScalarFunction::CustomScalar { execute, .. } => (execute)(args),
        ScalarFunction::SequenceOp { .. } => Err(
            "Sequence operations (nextval/currval) require catalog access — handle at connection/processor level"
                .into(),
        ),
    }
}
