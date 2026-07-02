//! Function registry — manages lookup and registration of all built-in functions.
//!
//! Three categories:
//! - Scalar functions (1-to-1 mapping of inputs to outputs)
//! - Aggregate functions (N-to-1 reduction)
//! - Table functions (produce a table of rows)

use hashbrown::HashMap;
use kuzu_common::types::Value;
use kuzu_common::vector::DataChunk;
use std::sync::Arc;

// ==================== Scalar Function Types ====================

/// All built-in scalar function variants.
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub enum ScalarFunction {
    Arithmetic {
        op: ArithmeticOp,
    },
    Comparison {
        op: ComparisonOp,
    },
    String {
        op: StringOp,
    },
    Cast {
        target_type: CastTarget,
    },
    Date {
        op: DateOp,
    },
    List {
        op: ListOp,
    },
    Map {
        op: MapOp,
    },
    Struct {
        op: StructOp,
    },
    Boolean {
        op: BooleanOp,
    },
    Utility {
        op: UtilityOp,
    },
    Schema {
        op: SchemaOp,
    },
    Array {
        op: ArrayOp,
    },
    /// Path functions — operate on recursive rel / path values.
    Path {
        op: PathOp,
    },
    /// UUID function — generates random UUIDs.
    Uuid,
    /// Extension-provided scalar function with a callback closure.
    /// The closure receives input values and returns an output value.
    CustomScalar {
        name: String,
        execute: Arc<dyn Fn(&[Value]) -> Result<Value, String> + Send + Sync>,
    },
    /// Sequence operation (nextval/currval) — requires catalog access.
    /// Resolved at the connection/processor level where the catalog is available.
    SequenceOp {
        is_nextval: bool,
    },

}

impl std::fmt::Debug for ScalarFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Arithmetic { op } => f.debug_struct("Arithmetic").field("op", op).finish(),
            Self::Comparison { op } => f.debug_struct("Comparison").field("op", op).finish(),
            Self::String { op } => f.debug_struct("String").field("op", op).finish(),
            Self::Cast { target_type } => f.debug_struct("Cast").field("target_type", target_type).finish(),
            Self::Date { op } => f.debug_struct("Date").field("op", op).finish(),
            Self::List { op } => f.debug_struct("List").field("op", op).finish(),
            Self::Map { op } => f.debug_struct("Map").field("op", op).finish(),
            Self::Struct { op } => f.debug_struct("Struct").field("op", op).finish(),
            Self::Boolean { op } => f.debug_struct("Boolean").field("op", op).finish(),
            Self::Utility { op } => f.debug_struct("Utility").field("op", op).finish(),
            Self::Schema { op } => f.debug_struct("Schema").field("op", op).finish(),
            Self::Array { op } => f.debug_struct("Array").field("op", op).finish(),
            Self::Path { op } => f.debug_struct("Path").field("op", op).finish(),
            Self::Uuid => f.debug_struct("Uuid").finish(),
            Self::CustomScalar { name, .. } => f.debug_struct("CustomScalar").field("name", name).finish(),
            Self::SequenceOp { is_nextval } => f.debug_struct("SequenceOp").field("is_nextval", is_nextval).finish(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ArithmeticOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Abs,
    Ceil,
    Floor,
    Round,
    Negate,
    Power,
    Sqrt,
    Log,
    Exp,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Degrees,
    Radians,
    Sign,
    Pi,
    Rand,
    /// Math functions (f64-based, single argument)
    Cbrt,
    Cot,
    Log2,
    Even,
    /// Bitwise operations (int64-only, matching C++ hardcoded int64_t)
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    BitShiftLeft,
    BitShiftRight,
}

#[derive(Debug, Clone, Copy)]
pub enum ComparisonOp {
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone, Copy)]
pub enum StringOp {
    Concat,
    Contains,
    StartsWith,
    EndsWith,
    ToUpper,
    ToLower,
    Trim,
    LTrim,
    RTrim,
    Length,
    Reverse,
    Repeat,
    Replace,
    Substring,
    RegexMatches,
    RegexReplace,
    Split,
    Head,
    Tail,
    Left,
    Right,
    Lpad,
    Rpad,
}

#[derive(Debug, Clone, Copy)]
pub enum CastTarget {
    String,
    Int64,
    Int32,
    Double,
    Float,
    Bool,
    Date,
    Timestamp,
    Interval,
}

#[derive(Debug, Clone, Copy)]
pub enum DateOp {
    DatePart,
    DateTrunc,
    DateDiff,
    DateAdd,
    CurrentDate,
    CurrentTimestamp,
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    DayName,
    MonthName,
    LastDay,
    MakeDate,
}

#[derive(Debug, Clone, Copy)]
pub enum ListOp {
    Creation,
    Extract,
    Concat,
    Len,
    Sort,
    Reverse,
    Contains,
    Append,
    Prepend,
    Slice,
}

#[derive(Debug, Clone, Copy)]
pub enum MapOp {
    Creation,
    Extract,
    Keys,
    Values,
    Contains,
}

#[derive(Debug, Clone, Copy)]
pub enum StructOp {
    Creation,
    Extract,
}

#[derive(Debug, Clone, Copy)]
pub enum BooleanOp {
    And,
    Or,
    Xor,
    Not,
}

#[derive(Debug, Clone, Copy)]
pub enum UtilityOp {
    Coalesce,
    IfNull,
    TypeOf,
}

/// Schema functions — access metadata about nodes, relationships, and values.
///
/// These correspond to the C++ schema functions in `function/schema/`:
/// - `OFFSET(v)` → returns the internal offset of a node/rel ID (INT64)
/// - `ID(v)` → returns the internal ID (offset + table_id) as InternalID
/// - `START_NODE(r)` → returns the source node of a relationship
/// - `END_NODE(r)` → returns the target node of a relationship
/// - `LABEL(v)` → returns the table name as a string
#[derive(Debug, Clone, Copy)]
pub enum SchemaOp {
    Offset,
    Id,
    StartNode,
    EndNode,
    Label,
}

/// Path functions — operate on recursive rel / path values (NODES, RELS, LENGTH).
///
/// Ported from C++ `function/path/`:
/// - `NODES(path)` → returns the list of nodes in the path
/// - `RELS(path)` / `RELATIONSHIPS(path)` → returns the list of rels in the path
/// - `LENGTH(path)` → returns the number of rels in the path
#[derive(Debug, Clone, Copy)]
pub enum PathOp {
    Nodes,
    Rels,
    Length,
}

/// Array math functions — element-wise operations on numeric lists.
///
/// These correspond to the C++ array functions in `function/array/`:
/// - `array_cosine_similarity(a, b)` → cosine similarity between two arrays
/// - `array_distance(a, b)` → Euclidean distance between two arrays
/// - `array_inner_product(a, b)` → dot/inner product of two arrays
/// - `array_cross_product(a, b)` → 3D cross product of two arrays
/// - `array_squared_distance(a, b)` → squared Euclidean distance
#[derive(Debug, Clone, Copy)]
pub enum ArrayOp {
    CosineSimilarity,
    Distance,
    InnerProduct,
    CrossProduct,
    SquaredDistance,
}

// ==================== Aggregate Function Types ====================

/// All built-in aggregate functions.
#[derive(Debug, Clone)]
pub enum AggregateFunction {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    Collect,
    CountStar,
    StdDev,
    Variance,
}

// ==================== Table Function Types ====================

/// All built-in table functions.
#[derive(Clone)]
pub enum TableFunction {
    ScanCsv {
        path: String,
    },
    ScanParquet {
        path: String,
    },
    ScanJson {
        path: String,
    },
    ListTables,
    ShowColumns {
        table_name: String,
    },
    CurrentSetting {
        key: String,
    },
    /// Extension-specific custom table function (tag-based, no callback).
    /// The `name` field identifies which custom function to execute.
    Custom {
        name: String,
    },
    /// Extension-provided table function with a callback closure.
    /// The closure receives input args and fills a mutable DataChunk.
    #[allow(clippy::type_complexity)]
    CustomTable {
        name: String,
        execute: Arc<dyn Fn(&[Value], &mut DataChunk) -> Result<(), String> + Send + Sync>,
    },
}

impl std::fmt::Debug for TableFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ScanCsv { path } => f.debug_struct("ScanCsv").field("path", path).finish(),
            Self::ScanParquet { path } => f.debug_struct("ScanParquet").field("path", path).finish(),
            Self::ScanJson { path } => f.debug_struct("ScanJson").field("path", path).finish(),
            Self::ListTables => write!(f, "ListTables"),
            Self::ShowColumns { table_name } => f.debug_struct("ShowColumns").field("table_name", table_name).finish(),
            Self::CurrentSetting { key } => f.debug_struct("CurrentSetting").field("key", key).finish(),
            Self::Custom { name } => f.debug_struct("Custom").field("name", name).finish(),
            Self::CustomTable { name, .. } => f.debug_struct("CustomTable").field("name", name).finish(),
        }
    }
}

/// A resolved function with its variant.
#[derive(Debug, Clone)]
pub enum ResolvedFunction {
    Scalar(ScalarFunction),
    Aggregate(AggregateFunction),
    Table(TableFunction),
}

// ==================== Registry ====================

/// Registry of all built-in functions (scalar, aggregate, table).
#[derive(Default)]
pub struct FunctionRegistry {
    scalar_functions: HashMap<String, ScalarFunction>,
    aggregate_functions: HashMap<String, AggregateFunction>,
    table_functions: HashMap<String, TableFunction>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        let mut reg = Self::default();
        reg.register_builtins();
        reg
    }

    fn register_builtins(&mut self) {
        // --- Arithmetic ---
        self.register_scalar("+", ScalarFunction::Arithmetic { op: ArithmeticOp::Add });
        self.register_scalar("-", ScalarFunction::Arithmetic { op: ArithmeticOp::Sub });
        self.register_scalar("*", ScalarFunction::Arithmetic { op: ArithmeticOp::Mul });
        self.register_scalar("/", ScalarFunction::Arithmetic { op: ArithmeticOp::Div });
        self.register_scalar("%", ScalarFunction::Arithmetic { op: ArithmeticOp::Mod });
        self.register_scalar("abs", ScalarFunction::Arithmetic { op: ArithmeticOp::Abs });
        self.register_scalar("ceil", ScalarFunction::Arithmetic { op: ArithmeticOp::Ceil });
        // Standard SQL alias
        self.register_scalar("ceiling", ScalarFunction::Arithmetic { op: ArithmeticOp::Ceil });
        self.register_scalar(
            "floor",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::Floor,
            },
        );
        self.register_scalar(
            "round",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::Round,
            },
        );
        self.register_scalar(
            "^",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::Power,
            },
        );
        self.register_scalar("sqrt", ScalarFunction::Arithmetic { op: ArithmeticOp::Sqrt });
        // Math aliases
        self.register_scalar("cbrt", ScalarFunction::Arithmetic { op: ArithmeticOp::Cbrt });
        self.register_scalar("cot", ScalarFunction::Arithmetic { op: ArithmeticOp::Cot });
        self.register_scalar("log", ScalarFunction::Arithmetic { op: ArithmeticOp::Log });
        self.register_scalar("ln", ScalarFunction::Arithmetic { op: ArithmeticOp::Log });
        self.register_scalar("log2", ScalarFunction::Arithmetic { op: ArithmeticOp::Log2 });
        self.register_scalar("even", ScalarFunction::Arithmetic { op: ArithmeticOp::Even });
        self.register_scalar("exp", ScalarFunction::Arithmetic { op: ArithmeticOp::Exp });
        self.register_scalar("sin", ScalarFunction::Arithmetic { op: ArithmeticOp::Sin });
        self.register_scalar("cos", ScalarFunction::Arithmetic { op: ArithmeticOp::Cos });
        self.register_scalar("tan", ScalarFunction::Arithmetic { op: ArithmeticOp::Tan });
        self.register_scalar("asin", ScalarFunction::Arithmetic { op: ArithmeticOp::Asin });
        self.register_scalar("acos", ScalarFunction::Arithmetic { op: ArithmeticOp::Acos });
        self.register_scalar("atan", ScalarFunction::Arithmetic { op: ArithmeticOp::Atan });
        self.register_scalar("atan2", ScalarFunction::Arithmetic { op: ArithmeticOp::Atan2 });
        self.register_scalar("degrees", ScalarFunction::Arithmetic { op: ArithmeticOp::Degrees });
        self.register_scalar("radians", ScalarFunction::Arithmetic { op: ArithmeticOp::Radians });
        self.register_scalar("sign", ScalarFunction::Arithmetic { op: ArithmeticOp::Sign });
        self.register_scalar("pi", ScalarFunction::Arithmetic { op: ArithmeticOp::Pi });
        self.register_scalar("rand", ScalarFunction::Arithmetic { op: ArithmeticOp::Rand });

        // --- Bitwise ---
        self.register_scalar(
            "bitwise_and",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::BitwiseAnd,
            },
        );
        self.register_scalar(
            "&",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::BitwiseAnd,
            },
        );
        self.register_scalar(
            "bitwise_or",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::BitwiseOr,
            },
        );
        self.register_scalar(
            "|",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::BitwiseOr,
            },
        );
        self.register_scalar(
            "bitwise_xor",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::BitwiseXor,
            },
        );
        self.register_scalar(
            "#",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::BitwiseXor,
            },
        );
        self.register_scalar(
            "bit_shift_left",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::BitShiftLeft,
            },
        );
        self.register_scalar(
            "<<",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::BitShiftLeft,
            },
        );
        self.register_scalar(
            "bit_shift_right",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::BitShiftRight,
            },
        );
        self.register_scalar(
            ">>",
            ScalarFunction::Arithmetic {
                op: ArithmeticOp::BitShiftRight,
            },
        );

        // --- Comparison ---
        self.register_scalar("=", ScalarFunction::Comparison { op: ComparisonOp::Eq });
        self.register_scalar(
            "<>",
            ScalarFunction::Comparison {
                op: ComparisonOp::NotEq,
            },
        );
        self.register_scalar("<", ScalarFunction::Comparison { op: ComparisonOp::Lt });
        self.register_scalar("<=", ScalarFunction::Comparison { op: ComparisonOp::Lte });
        self.register_scalar(">", ScalarFunction::Comparison { op: ComparisonOp::Gt });
        self.register_scalar(">=", ScalarFunction::Comparison { op: ComparisonOp::Gte });
        self.register_scalar(
            "IS NULL",
            ScalarFunction::Comparison {
                op: ComparisonOp::IsNull,
            },
        );
        self.register_scalar(
            "IS NOT NULL",
            ScalarFunction::Comparison {
                op: ComparisonOp::IsNotNull,
            },
        );

        // --- String ---
        self.register_scalar("concat", ScalarFunction::String { op: StringOp::Concat });
        self.register_scalar("contains", ScalarFunction::String { op: StringOp::Contains });
        self.register_scalar(
            "starts_with",
            ScalarFunction::String {
                op: StringOp::StartsWith,
            },
        );
        self.register_scalar("ends_with", ScalarFunction::String { op: StringOp::EndsWith });
        self.register_scalar("to_upper", ScalarFunction::String { op: StringOp::ToUpper });
        self.register_scalar("to_lower", ScalarFunction::String { op: StringOp::ToLower });
        // Standard SQL aliases
        self.register_scalar("upper", ScalarFunction::String { op: StringOp::ToUpper });
        self.register_scalar("lower", ScalarFunction::String { op: StringOp::ToLower });
        self.register_scalar("ucase", ScalarFunction::String { op: StringOp::ToUpper });
        self.register_scalar("lcase", ScalarFunction::String { op: StringOp::ToLower });
        self.register_scalar("trim", ScalarFunction::String { op: StringOp::Trim });
        self.register_scalar("ltrim", ScalarFunction::String { op: StringOp::LTrim });
        self.register_scalar("rtrim", ScalarFunction::String { op: StringOp::RTrim });
        self.register_scalar("length", ScalarFunction::String { op: StringOp::Length });
        self.register_scalar("reverse", ScalarFunction::String { op: StringOp::Reverse });
        self.register_scalar("repeat", ScalarFunction::String { op: StringOp::Repeat });
        self.register_scalar("replace", ScalarFunction::String { op: StringOp::Replace });
        self.register_scalar(
            "substring",
            ScalarFunction::String {
                op: StringOp::Substring,
            },
        );
        self.register_scalar(
            "regex_matches",
            ScalarFunction::String {
                op: StringOp::RegexMatches,
            },
        );
        self.register_scalar(
            "regex_replace",
            ScalarFunction::String {
                op: StringOp::RegexReplace,
            },
        );
        self.register_scalar(
            "split",
            ScalarFunction::String {
                op: StringOp::Split,
            },
        );
        self.register_scalar(
            "head",
            ScalarFunction::String {
                op: StringOp::Head,
            },
        );
        self.register_scalar(
            "tail",
            ScalarFunction::String {
                op: StringOp::Tail,
            },
        );
        self.register_scalar(
            "left",
            ScalarFunction::String {
                op: StringOp::Left,
            },
        );
        self.register_scalar(
            "right",
            ScalarFunction::String {
                op: StringOp::Right,
            },
        );
        self.register_scalar(
            "lpad",
            ScalarFunction::String {
                op: StringOp::Lpad,
            },
        );
        self.register_scalar(
            "rpad",
            ScalarFunction::String {
                op: StringOp::Rpad,
            },
        );

        // --- Date/Time ---
        self.register_scalar("date_part", ScalarFunction::Date { op: DateOp::DatePart });
        self.register_scalar("date_trunc", ScalarFunction::Date { op: DateOp::DateTrunc });
        self.register_scalar("date_diff", ScalarFunction::Date { op: DateOp::DateDiff });
        self.register_scalar("date_add", ScalarFunction::Date { op: DateOp::DateAdd });
        self.register_scalar(
            "current_date",
            ScalarFunction::Date {
                op: DateOp::CurrentDate,
            },
        );
        self.register_scalar(
            "current_timestamp",
            ScalarFunction::Date {
                op: DateOp::CurrentTimestamp,
            },
        );
        self.register_scalar("year", ScalarFunction::Date { op: DateOp::Year });
        self.register_scalar("month", ScalarFunction::Date { op: DateOp::Month });

        // Sequence functions (require catalog access at connection level)
        self.register_scalar("nextval", ScalarFunction::SequenceOp { is_nextval: true });
        self.register_scalar("currval", ScalarFunction::SequenceOp { is_nextval: false });
        self.register_scalar("day", ScalarFunction::Date { op: DateOp::Day });
        self.register_scalar("hour", ScalarFunction::Date { op: DateOp::Hour });
        self.register_scalar("minute", ScalarFunction::Date { op: DateOp::Minute });
        self.register_scalar("second", ScalarFunction::Date { op: DateOp::Second });
        self.register_scalar("dayname", ScalarFunction::Date { op: DateOp::DayName });
        self.register_scalar("monthname", ScalarFunction::Date { op: DateOp::MonthName });
        self.register_scalar("last_day", ScalarFunction::Date { op: DateOp::LastDay });
        self.register_scalar("make_date", ScalarFunction::Date { op: DateOp::MakeDate });

        // --- Cast ---
        self.register_scalar(
            "CAST",
            ScalarFunction::Cast {
                target_type: CastTarget::String,
            },
        );
        self.register_scalar(
            "cast_string",
            ScalarFunction::Cast {
                target_type: CastTarget::String,
            },
        );
        self.register_scalar(
            "cast_int64",
            ScalarFunction::Cast {
                target_type: CastTarget::Int64,
            },
        );
        self.register_scalar(
            "cast_double",
            ScalarFunction::Cast {
                target_type: CastTarget::Double,
            },
        );
        self.register_scalar(
            "cast_bool",
            ScalarFunction::Cast {
                target_type: CastTarget::Bool,
            },
        );
        // Cast function name aliases (Cypher/SQL standard)
        self.register_scalar("date", ScalarFunction::Cast { target_type: CastTarget::Date });
        self.register_scalar("timestamp", ScalarFunction::Cast { target_type: CastTarget::Timestamp });
        self.register_scalar("float", ScalarFunction::Cast { target_type: CastTarget::Double });
        self.register_scalar("double", ScalarFunction::Cast { target_type: CastTarget::Double });
        self.register_scalar("int64", ScalarFunction::Cast { target_type: CastTarget::Int64 });
        self.register_scalar("int", ScalarFunction::Cast { target_type: CastTarget::Int64 });
        self.register_scalar("bool", ScalarFunction::Cast { target_type: CastTarget::Bool });
        self.register_scalar("boolean", ScalarFunction::Cast { target_type: CastTarget::Bool });
        self.register_scalar("string", ScalarFunction::Cast { target_type: CastTarget::String });
        self.register_scalar("blob", ScalarFunction::Cast { target_type: CastTarget::String });

        // --- List ---
        self.register_scalar("list_creation", ScalarFunction::List { op: ListOp::Creation });
        self.register_scalar("list_extract", ScalarFunction::List { op: ListOp::Extract });
        self.register_scalar("list_concat", ScalarFunction::List { op: ListOp::Concat });
        self.register_scalar("list_len", ScalarFunction::List { op: ListOp::Len });
        self.register_scalar("list_sort", ScalarFunction::List { op: ListOp::Sort });
        self.register_scalar("list_reverse", ScalarFunction::List { op: ListOp::Reverse });
        self.register_scalar("list_contains", ScalarFunction::List { op: ListOp::Contains });
        self.register_scalar("list_append", ScalarFunction::List { op: ListOp::Append });
        self.register_scalar("list_prepend", ScalarFunction::List { op: ListOp::Prepend });
        self.register_scalar("list_slice", ScalarFunction::List { op: ListOp::Slice });

        // --- Map ---
        self.register_scalar("map_creation", ScalarFunction::Map { op: MapOp::Creation });
        self.register_scalar("map_extract", ScalarFunction::Map { op: MapOp::Extract });
        self.register_scalar("map_keys", ScalarFunction::Map { op: MapOp::Keys });
        self.register_scalar("map_values", ScalarFunction::Map { op: MapOp::Values });

        // --- Struct ---
        self.register_scalar("struct_creation", ScalarFunction::Struct { op: StructOp::Creation });
        self.register_scalar("struct_extract", ScalarFunction::Struct { op: StructOp::Extract });

        // --- Boolean ---
        self.register_scalar("AND", ScalarFunction::Boolean { op: BooleanOp::And });
        self.register_scalar("OR", ScalarFunction::Boolean { op: BooleanOp::Or });
        self.register_scalar("XOR", ScalarFunction::Boolean { op: BooleanOp::Xor });
        self.register_scalar("NOT", ScalarFunction::Boolean { op: BooleanOp::Not });

        // --- Utility ---
        self.register_scalar(
            "coalesce",
            ScalarFunction::Utility {
                op: UtilityOp::Coalesce,
            },
        );
        self.register_scalar("ifnull", ScalarFunction::Utility { op: UtilityOp::IfNull });
        self.register_scalar("typeof", ScalarFunction::Utility { op: UtilityOp::TypeOf });

        // --- Schema ---
        self.register_scalar("OFFSET", ScalarFunction::Schema { op: SchemaOp::Offset });
        self.register_scalar("ID", ScalarFunction::Schema { op: SchemaOp::Id });
        self.register_scalar("START_NODE", ScalarFunction::Schema { op: SchemaOp::StartNode });
        self.register_scalar("END_NODE", ScalarFunction::Schema { op: SchemaOp::EndNode });
        self.register_scalar("LABEL", ScalarFunction::Schema { op: SchemaOp::Label });

        // --- Array ---
        self.register_scalar("array_cosine_similarity", ScalarFunction::Array { op: ArrayOp::CosineSimilarity });
        self.register_scalar("array_distance", ScalarFunction::Array { op: ArrayOp::Distance });
        self.register_scalar("array_inner_product", ScalarFunction::Array { op: ArrayOp::InnerProduct });
        self.register_scalar("array_cross_product", ScalarFunction::Array { op: ArrayOp::CrossProduct });
        self.register_scalar("array_squared_distance", ScalarFunction::Array { op: ArrayOp::SquaredDistance });

        // --- Path ---
        self.register_scalar("nodes", ScalarFunction::Path { op: PathOp::Nodes });
        self.register_scalar("rels", ScalarFunction::Path { op: PathOp::Rels });
        self.register_scalar("relationships", ScalarFunction::Path { op: PathOp::Rels });

        // --- UUID ---
        self.register_scalar("gen_random_uuid", ScalarFunction::Uuid);

        // --- Array utility aliases (delegate to list functions) ---
        self.register_scalar("array_concat", ScalarFunction::List { op: ListOp::Concat });
        self.register_scalar("array_cat", ScalarFunction::List { op: ListOp::Concat });
        self.register_scalar("array_append", ScalarFunction::List { op: ListOp::Append });
        self.register_scalar("array_push_back", ScalarFunction::List { op: ListOp::Append });
        self.register_scalar("array_prepend", ScalarFunction::List { op: ListOp::Prepend });
        self.register_scalar("array_push_front", ScalarFunction::List { op: ListOp::Prepend });
        self.register_scalar("array_contains", ScalarFunction::List { op: ListOp::Contains });
        self.register_scalar("array_has", ScalarFunction::List { op: ListOp::Contains });
        self.register_scalar("array_slice", ScalarFunction::List { op: ListOp::Slice });
        self.register_scalar("array_value", ScalarFunction::List { op: ListOp::Creation });

        // --- Aggregate ---
        self.register_aggregate("COUNT", AggregateFunction::Count);
        self.register_aggregate("COUNT(*)", AggregateFunction::CountStar);
        self.register_aggregate("SUM", AggregateFunction::Sum);
        self.register_aggregate("AVG", AggregateFunction::Avg);
        self.register_aggregate("MIN", AggregateFunction::Min);
        self.register_aggregate("MAX", AggregateFunction::Max);
        self.register_aggregate("COLLECT", AggregateFunction::Collect);
        self.register_aggregate("STDDEV", AggregateFunction::StdDev);
        self.register_aggregate("VARIANCE", AggregateFunction::Variance);

        // --- Table ---
        self.register_table("list_tables", TableFunction::ListTables);
    }

    // --- Registration ---

    pub fn register_scalar(&mut self, name: &str, func: ScalarFunction) {
        self.scalar_functions.insert(name.to_lowercase(), func);
    }

    pub fn register_aggregate(&mut self, name: &str, func: AggregateFunction) {
        self.aggregate_functions.insert(name.to_lowercase(), func);
    }

    pub fn register_table(&mut self, name: &str, func: TableFunction) {
        self.table_functions.insert(name.to_lowercase(), func);
    }

    // --- Lookup ---

    pub fn resolve(&self, name: &str) -> Option<ResolvedFunction> {
        let lower = name.to_lowercase();
        if let Some(f) = self.scalar_functions.get(&lower) {
            return Some(ResolvedFunction::Scalar(f.clone()));
        }
        if let Some(f) = self.aggregate_functions.get(&lower) {
            return Some(ResolvedFunction::Aggregate(f.clone()));
        }
        if let Some(f) = self.table_functions.get(&lower) {
            return Some(ResolvedFunction::Table(f.clone()));
        }
        None
    }

    pub fn get_scalar(&self, name: &str) -> Option<&ScalarFunction> {
        self.scalar_functions.get(&name.to_lowercase())
    }

    pub fn get_aggregate(&self, name: &str) -> Option<&AggregateFunction> {
        self.aggregate_functions.get(&name.to_lowercase())
    }

    pub fn get_table(&self, name: &str) -> Option<&TableFunction> {
        self.table_functions.get(&name.to_lowercase())
    }

    pub fn contains(&self, name: &str) -> bool {
        let lower = name.to_lowercase();
        self.scalar_functions.contains_key(&lower)
            || self.aggregate_functions.contains_key(&lower)
            || self.table_functions.contains_key(&lower)
    }

    /// Number of registered scalar functions.
    pub fn scalar_count(&self) -> usize {
        self.scalar_functions.len()
    }

    /// Number of registered aggregate functions.
    pub fn aggregate_count(&self) -> usize {
        self.aggregate_functions.len()
    }

    /// Number of registered table functions.
    pub fn table_count(&self) -> usize {
        self.table_functions.len()
    }

    /// Total number of registered functions.
    pub fn total_count(&self) -> usize {
        self.scalar_count() + self.aggregate_count() + self.table_count()
    }

    /// Execute a table function by name with the given pre-evaluated arguments.
    ///
    /// Returns a `Vec<Vec<Value>>` representing rows of results.
    /// Each inner vec is one row with one or more column values.
    pub fn execute_table_function(
        &self,
        name: &str,
        args: &[Value],
    ) -> Result<Vec<Vec<Value>>, String> {
        use kuzu_common::vector::DataChunk;

        let func = self
            .get_table(name)
            .ok_or_else(|| format!("Table function '{}' not found", name))?;

        match func {
            TableFunction::ListTables => {
                Err("ListTables requires catalog access — handled at connection level".into())
            }
            TableFunction::ShowColumns { .. } => {
                Err("ShowColumns requires catalog access — handled at connection level".into())
            }
            TableFunction::Custom { name: custom_name } => {
                Err(format!("Custom table function '{}' has no callback registered", custom_name))
            }
            TableFunction::CustomTable { name: _, execute } => {
                let mut chunk = DataChunk {
                    fields: Vec::new(),
                    size: 0,
                    field_names: vec![],
                };
                execute(args, &mut chunk).map(|_| {
                    let mut rows = Vec::new();
                    for row in 0..chunk.size {
                        let mut row_vals = Vec::new();
                        for field in &chunk.fields {
                            row_vals.push(field.get_value(row).unwrap_or(Value::Null));
                        }
                        rows.push(row_vals);
                    }
                    rows
                })
            }
            TableFunction::ScanCsv { .. }
            | TableFunction::ScanParquet { .. }
            | TableFunction::ScanJson { .. }
            | TableFunction::CurrentSetting { .. } => {
                Err(format!(
                    "Table function '{}' requires file/catalog context — use COPY FROM instead",
                    name
                ))
            }
        }
    }
}
