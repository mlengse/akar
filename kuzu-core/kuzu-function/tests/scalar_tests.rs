use kuzu_function::aggregate::{evaluate_aggregate, AggValueState};
use kuzu_function::registry::*;
use kuzu_function::scalar::evaluate_scalar;
use kuzu_common::types::{Date, Interval, Timestamp, Value};

#[test]
fn test_add() {
    let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Add };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(3), Value::Int64(4)]).unwrap(),
        Value::Int64(7)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::Double(1.5), Value::Double(2.5)]).unwrap(),
        Value::Double(4.0)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("a".into()), Value::String("b".into())]).unwrap(),
        Value::String("ab".into())
    );
}

#[test]
fn test_sub() {
    let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Sub };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(10), Value::Int64(3)]).unwrap(),
        Value::Int64(7)
    );
}

#[test]
fn test_mul() {
    let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Mul };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(5), Value::Int64(6)]).unwrap(),
        Value::Int64(30)
    );
}

#[test]
fn test_div() {
    let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Div };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(10), Value::Int64(3)]).unwrap(),
        Value::Int64(3)
    );
}

#[test]
fn test_div_by_zero() {
    let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Div };
    assert!(evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(0)]).is_err());
}

#[test]
fn test_mod() {
    let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Mod };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(10), Value::Int64(3)]).unwrap(),
        Value::Int64(1)
    );
}

#[test]
fn test_abs() {
    let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Abs };
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(-5)]).unwrap(), Value::Int64(5));
}

#[test]
fn test_negate() {
    let func = ScalarFunction::Arithmetic {
        op: ArithmeticOp::Negate,
    };
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(42)]).unwrap(), Value::Int64(-42));
}

// --- Light Math tests ---

#[test]
fn test_cbrt() {
    let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Cbrt };
    let get_f64 = |v: Value| -> f64 {
        if let Value::Double(d) = v {
            d
        } else {
            panic!("Expected Double")
        }
    };
    assert!((get_f64(evaluate_scalar(&func, &[Value::Double(27.0)]).unwrap()) - 3.0).abs() < 1e-10);
    assert!((get_f64(evaluate_scalar(&func, &[Value::Double(8.0)]).unwrap()) - 2.0).abs() < 1e-10);
    assert!((get_f64(evaluate_scalar(&func, &[Value::Int64(27)]).unwrap()) - 3.0).abs() < 1e-10);
}

#[test]
fn test_cot() {
    let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Cot };
    // cot(pi/4) = 1
    let pi_4 = std::f64::consts::PI / 4.0;
    let result = evaluate_scalar(&func, &[Value::Double(pi_4)]).unwrap();
    if let Value::Double(v) = result {
        assert!((v - 1.0).abs() < 1e-10);
    } else {
        panic!("Expected Double");
    }
}

#[test]
fn test_log2() {
    let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Log2 };
    let get_f64 = |v: Value| -> f64 {
        if let Value::Double(d) = v {
            d
        } else {
            panic!("Expected Double")
        }
    };
    assert!((get_f64(evaluate_scalar(&func, &[Value::Double(8.0)]).unwrap()) - 3.0).abs() < 1e-10);
    assert!((get_f64(evaluate_scalar(&func, &[Value::Int64(16)]).unwrap()) - 4.0).abs() < 1e-10);
}

#[test]
fn test_even() {
    let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Even };
    // Int64: even numbers unchanged, odd rounded up
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(4)]).unwrap(), Value::Int64(4));
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(5)]).unwrap(), Value::Int64(6));
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(-2)]).unwrap(), Value::Int64(-2));
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(-3)]).unwrap(), Value::Int64(-2));
    // Double
    assert_eq!(evaluate_scalar(&func, &[Value::Double(2.3)]).unwrap(), Value::Int64(4));
    assert_eq!(evaluate_scalar(&func, &[Value::Double(3.8)]).unwrap(), Value::Int64(4));
}

// --- Heavy Math tests ---

#[test]
fn test_factorial() {
    let func = ScalarFunction::Arithmetic {
        op: ArithmeticOp::Factorial,
    };
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(0)]).unwrap(), Value::Int64(1));
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(1)]).unwrap(), Value::Int64(1));
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(5)]).unwrap(), Value::Int64(120));
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(10)]).unwrap(),
        Value::Int64(3628800)
    );
    // Negative input
    assert!(evaluate_scalar(&func, &[Value::Int64(-1)]).is_err());
}

#[test]
fn test_gamma() {
    let func = ScalarFunction::Arithmetic {
        op: ArithmeticOp::Gamma,
    };
    let get_f64 = |v: Value| -> f64 {
        if let Value::Double(d) = v {
            d
        } else {
            panic!("Expected Double")
        }
    };
    // Gamma(1) = 1
    assert!((get_f64(evaluate_scalar(&func, &[Value::Double(1.0)]).unwrap()) - 1.0).abs() < 1e-10);
    // Gamma(2) = 1
    assert!((get_f64(evaluate_scalar(&func, &[Value::Double(2.0)]).unwrap()) - 1.0).abs() < 1e-10);
    // Gamma(3) = 2! = 2
    assert!((get_f64(evaluate_scalar(&func, &[Value::Double(3.0)]).unwrap()) - 2.0).abs() < 1e-8);
    // Non-positive integer → infinity
    assert_eq!(
        evaluate_scalar(&func, &[Value::Double(0.0)]).unwrap(),
        Value::Double(f64::INFINITY)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::Double(-1.0)]).unwrap(),
        Value::Double(f64::INFINITY)
    );
}

#[test]
fn test_lgamma() {
    let func = ScalarFunction::Arithmetic {
        op: ArithmeticOp::Lgamma,
    };
    let get_f64 = |v: Value| -> f64 {
        if let Value::Double(d) = v {
            d
        } else {
            panic!("Expected Double")
        }
    };
    // ln(Gamma(1)) = ln(1) = 0
    assert!((get_f64(evaluate_scalar(&func, &[Value::Double(1.0)]).unwrap()) - 0.0).abs() < 1e-10);
    // ln(Gamma(2)) = ln(1) = 0
    assert!((get_f64(evaluate_scalar(&func, &[Value::Double(2.0)]).unwrap()) - 0.0).abs() < 1e-10);
    // Non-positive integer → infinity
    assert_eq!(
        evaluate_scalar(&func, &[Value::Double(0.0)]).unwrap(),
        Value::Double(f64::INFINITY)
    );
}

#[test]
fn test_set_seed() {
    let set_seed = ScalarFunction::Arithmetic {
        op: ArithmeticOp::SetSeed,
    };
    let rand_func = ScalarFunction::Arithmetic { op: ArithmeticOp::Rand };
    // Set seed to known value
    assert_eq!(
        evaluate_scalar(&set_seed, &[Value::Double(0.5)]).unwrap(),
        Value::Int32(0)
    );
    let first = evaluate_scalar(&rand_func, &[]).unwrap();
    // Same seed should produce same sequence
    assert_eq!(
        evaluate_scalar(&set_seed, &[Value::Double(0.5)]).unwrap(),
        Value::Int32(0)
    );
    let second = evaluate_scalar(&rand_func, &[]).unwrap();
    assert_eq!(first, second);
}

// --- Hash function tests ---

#[test]
fn test_md5() {
    let func = ScalarFunction::Hash { op: HashOp::Md5 };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap(),
        Value::String("5d41402abc4b2a76b9719d911017c592".into())
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("".into())]).unwrap(),
        Value::String("d41d8cd98f00b204e9800998ecf8427e".into())
    );
}

#[test]
fn test_sha256() {
    let func = ScalarFunction::Hash { op: HashOp::Sha256 };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap(),
        Value::String("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into())
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("".into())]).unwrap(),
        Value::String("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into())
    );
}

#[test]
fn test_hash_generic() {
    let func = ScalarFunction::Hash { op: HashOp::Hash };
    let h1 = evaluate_scalar(&func, &[Value::Int64(42)]).unwrap();
    let h2 = evaluate_scalar(&func, &[Value::Int64(42)]).unwrap();
    assert_eq!(h1, h2);
    let h3 = evaluate_scalar(&func, &[Value::Int64(43)]).unwrap();
    assert_ne!(h1, h3);
    let hs = evaluate_scalar(&func, &[Value::String("test".into())]).unwrap();
    assert!(matches!(hs, Value::Int64(_)));
}

// --- Regex string function tests ---

#[test]
fn test_regexp_full_match() {
    let func = ScalarFunction::String {
        op: StringOp::RegexpFullMatch,
    };
    // Full match
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into()), Value::String("hello".into())]).unwrap(),
        Value::Bool(true)
    );
    // Partial match should be false
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello123".into()), Value::String(r"\d+".into())]).unwrap(),
        Value::Bool(false)
    );
    // Full match with pattern
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("123".into()), Value::String(r"\d+".into())]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn test_regexp_extract() {
    let func = ScalarFunction::String {
        op: StringOp::RegexpExtract,
    };
    // Extract first digit sequence
    assert_eq!(
        evaluate_scalar(
            &func,
            &[Value::String("abc123def".into()), Value::String(r"\d+".into())]
        )
        .unwrap(),
        Value::String("123".into())
    );
    // No match
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("abcdef".into()), Value::String(r"\d+".into())]).unwrap(),
        Value::String("".into())
    );
    // With capture group (0-based: group 0 = full match)
    assert_eq!(
        evaluate_scalar(
            &func,
            &[
                Value::String("hello@example.com".into()),
                Value::String(r"(\w+)@(\w+\.\w+)".into()),
                Value::Int64(1),
            ]
        )
        .unwrap(),
        Value::String("hello".into())
    );
    assert_eq!(
        evaluate_scalar(
            &func,
            &[
                Value::String("hello@example.com".into()),
                Value::String(r"(\w+)@(\w+\.\w+)".into()),
                Value::Int64(2),
            ]
        )
        .unwrap(),
        Value::String("example.com".into())
    );
}

#[test]
fn test_regexp_extract_all() {
    let func = ScalarFunction::String {
        op: StringOp::RegexpExtractAll,
    };
    // Extract all digits
    let result = evaluate_scalar(&func, &[Value::String("a1b2c3".into()), Value::String(r"\d+".into())]).unwrap();
    assert_eq!(
        result,
        Value::List(vec![
            Value::String("1".into()),
            Value::String("2".into()),
            Value::String("3".into()),
        ])
    );
    // With group
    let result = evaluate_scalar(
        &func,
        &[
            Value::String("a1b2c3".into()),
            Value::String(r"(\d)".into()),
            Value::Int64(1),
        ],
    )
    .unwrap();
    assert_eq!(
        result,
        Value::List(vec![
            Value::String("1".into()),
            Value::String("2".into()),
            Value::String("3".into()),
        ])
    );
    // No matches
    let result = evaluate_scalar(&func, &[Value::String("abc".into()), Value::String(r"\d+".into())]).unwrap();
    assert_eq!(result, Value::List(vec![]));
}

#[test]
fn test_regexp_split_to_array() {
    let func = ScalarFunction::String {
        op: StringOp::RegexpSplitToArray,
    };
    // Split on digits
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("a1b2c".into()), Value::String(r"\d".into())]).unwrap(),
        Value::List(vec![
            Value::String("a".into()),
            Value::String("b".into()),
            Value::String("c".into()),
        ])
    );
    // No match: single element
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("abc".into()), Value::String(r"\d+".into())]).unwrap(),
        Value::List(vec![Value::String("abc".into())])
    );
}

#[test]
fn test_levenshtein() {
    let func = ScalarFunction::String {
        op: StringOp::Levenshtein,
    };
    // Same strings
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into()), Value::String("hello".into())]).unwrap(),
        Value::Int64(0)
    );
    // One substitution
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("kitten".into()), Value::String("sitten".into())]).unwrap(),
        Value::Int64(1)
    );
    // Known distance
    assert_eq!(
        evaluate_scalar(
            &func,
            &[Value::String("kitten".into()), Value::String("sitting".into())]
        )
        .unwrap(),
        Value::Int64(3)
    );
    // Empty string
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("".into()), Value::String("abc".into())]).unwrap(),
        Value::Int64(3)
    );
}

// --- Bitwise tests ---

#[test]
fn test_bitwise_and() {
    let func = ScalarFunction::Arithmetic {
        op: ArithmeticOp::BitwiseAnd,
    };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(6), Value::Int64(3)]).unwrap(),
        Value::Int64(2)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(0xFF), Value::Int64(0x0F)]).unwrap(),
        Value::Int64(0x0F)
    );
    // Error: non-integer
    assert!(evaluate_scalar(&func, &[Value::Double(1.0), Value::Int64(2)]).is_err());
    // Error: wrong number of args
    assert!(evaluate_scalar(&func, &[Value::Int64(1)]).is_err());
}

#[test]
fn test_bitwise_or() {
    let func = ScalarFunction::Arithmetic {
        op: ArithmeticOp::BitwiseOr,
    };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(6), Value::Int64(3)]).unwrap(),
        Value::Int64(7)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(0xF0), Value::Int64(0x0F)]).unwrap(),
        Value::Int64(0xFF)
    );
    assert!(evaluate_scalar(&func, &[Value::Int64(1)]).is_err());
}

#[test]
fn test_bitwise_xor() {
    let func = ScalarFunction::Arithmetic {
        op: ArithmeticOp::BitwiseXor,
    };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(6), Value::Int64(3)]).unwrap(),
        Value::Int64(5)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(0xFF), Value::Int64(0x0F)]).unwrap(),
        Value::Int64(0xF0)
    );
    assert!(evaluate_scalar(&func, &[Value::Int64(1)]).is_err());
}

#[test]
fn test_bit_shift_left() {
    let func = ScalarFunction::Arithmetic {
        op: ArithmeticOp::BitShiftLeft,
    };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(3)]).unwrap(),
        Value::Int64(8)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(0xFF), Value::Int64(4)]).unwrap(),
        Value::Int64(0xFF0)
    );
    assert!(evaluate_scalar(&func, &[Value::Int64(1)]).is_err());
}

#[test]
fn test_bit_shift_right() {
    let func = ScalarFunction::Arithmetic {
        op: ArithmeticOp::BitShiftRight,
    };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(8), Value::Int64(3)]).unwrap(),
        Value::Int64(1)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(0xFF0), Value::Int64(4)]).unwrap(),
        Value::Int64(0xFF)
    );
    assert!(evaluate_scalar(&func, &[Value::Int64(1)]).is_err());
}

#[test]
fn test_comparison_eq() {
    let func = ScalarFunction::Comparison { op: ComparisonOp::Eq };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(1)]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(2)]).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn test_comparison_gt() {
    let func = ScalarFunction::Comparison { op: ComparisonOp::Gt };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(5), Value::Int64(3)]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::Int64(3), Value::Int64(5)]).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn test_string_concat() {
    let func = ScalarFunction::String { op: StringOp::Concat };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello ".into()), Value::String("world".into())]).unwrap(),
        Value::String("hello world".into())
    );
}

#[test]
fn test_string_to_upper() {
    let func = ScalarFunction::String { op: StringOp::ToUpper };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap(),
        Value::String("HELLO".into())
    );
}

#[test]
fn test_string_to_lower() {
    let func = ScalarFunction::String { op: StringOp::ToLower };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("HELLO".into())]).unwrap(),
        Value::String("hello".into())
    );
}

#[test]
fn test_string_trim() {
    let func = ScalarFunction::String { op: StringOp::Trim };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("  hello  ".into())]).unwrap(),
        Value::String("hello".into())
    );
}

#[test]
fn test_string_length() {
    let func = ScalarFunction::String { op: StringOp::Length };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap(),
        Value::Int64(5)
    );
}

#[test]
fn test_string_contains() {
    let func = ScalarFunction::String { op: StringOp::Contains };
    assert_eq!(
        evaluate_scalar(
            &func,
            &[Value::String("hello world".into()), Value::String("world".into())]
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn test_boolean_and() {
    let func = ScalarFunction::Boolean { op: BooleanOp::And };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Bool(true), Value::Bool(true)]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::Bool(true), Value::Bool(false)]).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn test_boolean_or() {
    let func = ScalarFunction::Boolean { op: BooleanOp::Or };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Bool(true), Value::Bool(false)]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn test_boolean_not() {
    let func = ScalarFunction::Boolean { op: BooleanOp::Not };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Bool(true)]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::Bool(false)]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn test_is_null() {
    let func = ScalarFunction::Comparison {
        op: ComparisonOp::IsNull,
    };
    assert_eq!(evaluate_scalar(&func, &[Value::Null]).unwrap(), Value::Bool(true));
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(5)]).unwrap(), Value::Bool(false));
}

#[test]
fn test_coalesce() {
    let func = ScalarFunction::Utility {
        op: UtilityOp::Coalesce,
    };
    assert_eq!(
        evaluate_scalar(&func, &[Value::Null, Value::Int64(42)]).unwrap(),
        Value::Int64(42)
    );
}

#[test]
fn test_cast_int64() {
    let func = ScalarFunction::Cast {
        target_type: CastTarget::Int64,
    };
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(42)]).unwrap(), Value::Int64(42));
    assert_eq!(evaluate_scalar(&func, &[Value::Double(3.14)]).unwrap(), Value::Int64(3));
}

#[test]
fn test_cast_string() {
    let func = ScalarFunction::Cast {
        target_type: CastTarget::String,
    };
    let result = evaluate_scalar(&func, &[Value::Int64(42)]).unwrap();
    assert!(matches!(result, Value::String(_)));
}

#[test]
fn test_list_len() {
    let func = ScalarFunction::List { op: ListOp::Len };
    assert_eq!(
        evaluate_scalar(&func, &[Value::List(vec![Value::Int64(1), Value::Int64(2)])]).unwrap(),
        Value::Int64(2)
    );
}

#[test]
fn test_list_contains() {
    let func = ScalarFunction::List { op: ListOp::Contains };
    let list = Value::List(vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)]);
    assert_eq!(
        evaluate_scalar(&func, &[list.clone(), Value::Int64(2)]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_scalar(&func, &[list, Value::Int64(99)]).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn test_list_append() {
    let func = ScalarFunction::List { op: ListOp::Append };
    let result = evaluate_scalar(&func, &[Value::List(vec![Value::Int64(1)]), Value::Int64(2)]).unwrap();
    match result {
        Value::List(items) => assert_eq!(items.len(), 2),
        _ => panic!("Expected list"),
    }
}

#[test]
fn test_typeof() {
    let func = ScalarFunction::Utility { op: UtilityOp::TypeOf };
    let result = evaluate_scalar(&func, &[Value::Int64(42)]).unwrap();
    assert!(matches!(result, Value::String(_)));
}

#[test]
fn test_regex_matches() {
    let func = ScalarFunction::String {
        op: StringOp::RegexMatches,
    };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello123".into()), Value::String(r"\d+".into())]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into()), Value::String(r"\d+".into())]).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn test_regex_replace() {
    let func = ScalarFunction::String {
        op: StringOp::RegexReplace,
    };
    assert_eq!(
        evaluate_scalar(
            &func,
            &[
                Value::String("hello 123 world".into()),
                Value::String(r"\d+".into()),
                Value::String("NUM".into())
            ]
        )
        .unwrap(),
        Value::String("hello NUM world".into())
    );
}

#[test]
fn test_function_registry_lookup() {
    let reg = FunctionRegistry::new();
    assert!(reg.contains("+"));
    assert!(reg.contains("COUNT"));
    assert!(reg.contains("trim"));
    assert!(reg.contains("list_tables"));
    assert!(reg.scalar_count() > 30);
    assert!(reg.aggregate_count() >= 7);
    assert!(reg.total_count() >= 40);
}

// ==================== New Fase 1 Tests ====================

// --- String function tests ---
#[test]
fn test_string_reverse() {
    let func = ScalarFunction::String { op: StringOp::Reverse };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap(),
        Value::String("olleh".into())
    );
}

#[test]
fn test_string_repeat() {
    let func = ScalarFunction::String { op: StringOp::Repeat };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("ab".into()), Value::Int64(3)]).unwrap(),
        Value::String("ababab".into())
    );
}

#[test]
fn test_string_replace() {
    let func = ScalarFunction::String { op: StringOp::Replace };
    assert_eq!(
        evaluate_scalar(
            &func,
            &[
                Value::String("hello world".into()),
                Value::String("world".into()),
                Value::String("there".into())
            ]
        )
        .unwrap(),
        Value::String("hello there".into())
    );
}

#[test]
fn test_string_substring() {
    let func = ScalarFunction::String {
        op: StringOp::Substring,
    };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into()), Value::Int64(2)]).unwrap(),
        Value::String("ello".into())
    );
    assert_eq!(
        evaluate_scalar(
            &func,
            &[Value::String("hello".into()), Value::Int64(1), Value::Int64(3)]
        )
        .unwrap(),
        Value::String("hel".into())
    );
}

#[test]
fn test_string_starts_ends_with() {
    let starts = ScalarFunction::String {
        op: StringOp::StartsWith,
    };
    let ends = ScalarFunction::String { op: StringOp::EndsWith };
    assert_eq!(
        evaluate_scalar(&starts, &[Value::String("hello".into()), Value::String("he".into())]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_scalar(&ends, &[Value::String("hello".into()), Value::String("lo".into())]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn test_string_trim_variants() {
    let ltrim = ScalarFunction::String { op: StringOp::LTrim };
    let rtrim = ScalarFunction::String { op: StringOp::RTrim };
    assert_eq!(
        evaluate_scalar(&ltrim, &[Value::String("  hello".into())]).unwrap(),
        Value::String("hello".into())
    );
    assert_eq!(
        evaluate_scalar(&rtrim, &[Value::String("hello  ".into())]).unwrap(),
        Value::String("hello".into())
    );
}

// --- String basic tests (C++ port) ---

#[test]
fn test_initcap() {
    let func = ScalarFunction::String { op: StringOp::InitCap };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello world".into())]).unwrap(),
        Value::String("Hello world".into())
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("HELLO".into())]).unwrap(),
        Value::String("Hello".into())
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("".into())]).unwrap(),
        Value::String("".into())
    );
}

#[test]
fn test_concat_ws() {
    let func = ScalarFunction::String { op: StringOp::ConcatWs };
    // Basic concat
    assert_eq!(
        evaluate_scalar(
            &func,
            &[
                Value::String(",".into()),
                Value::String("a".into()),
                Value::String("b".into()),
                Value::String("c".into()),
            ]
        )
        .unwrap(),
        Value::String("a,b,c".into())
    );
    // Skip NULL
    assert_eq!(
        evaluate_scalar(
            &func,
            &[
                Value::String("-".into()),
                Value::String("a".into()),
                Value::Null,
                Value::String("b".into()),
            ]
        )
        .unwrap(),
        Value::String("a-b".into())
    );
    // Single element (no separator)
    assert_eq!(
        evaluate_scalar(&func, &[Value::String(",".into()), Value::String("only".into()),]).unwrap(),
        Value::String("only".into())
    );
}

#[test]
fn test_split_part() {
    let func = ScalarFunction::String {
        op: StringOp::SplitPart,
    };
    // Normal case
    assert_eq!(
        evaluate_scalar(
            &func,
            &[
                Value::String("a,b,c".into()),
                Value::String(",".into()),
                Value::Int64(2),
            ]
        )
        .unwrap(),
        Value::String("b".into())
    );
    // Out of range (too high)
    assert_eq!(
        evaluate_scalar(
            &func,
            &[Value::String("a,b".into()), Value::String(",".into()), Value::Int64(5),]
        )
        .unwrap(),
        Value::String("".into())
    );
    // Index <= 0
    assert_eq!(
        evaluate_scalar(
            &func,
            &[Value::String("a,b".into()), Value::String(",".into()), Value::Int64(0),]
        )
        .unwrap(),
        Value::String("".into())
    );
}

#[test]
fn test_array_extract() {
    let func = ScalarFunction::String {
        op: StringOp::ArrayExtract,
    };
    // Positive 1-based index
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into()), Value::Int64(1),]).unwrap(),
        Value::String("h".into())
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into()), Value::Int64(5),]).unwrap(),
        Value::String("o".into())
    );
    // Index 0 returns empty
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into()), Value::Int64(0),]).unwrap(),
        Value::String("".into())
    );
    // Negative index (from end)
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into()), Value::Int64(-1),]).unwrap(),
        Value::String("o".into())
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("hello".into()), Value::Int64(-2),]).unwrap(),
        Value::String("l".into())
    );
}

// --- Date function tests ---
#[test]
fn test_date_current() {
    let cur_date = ScalarFunction::Date {
        op: DateOp::CurrentDate,
    };
    let cur_ts = ScalarFunction::Date {
        op: DateOp::CurrentTimestamp,
    };
    let d = evaluate_scalar(&cur_date, &[]).unwrap();
    let ts = evaluate_scalar(&cur_ts, &[]).unwrap();
    assert!(matches!(d, Value::Date(_)));
    assert!(matches!(ts, Value::Timestamp(_)));
}

#[test]
fn test_date_year_month_day() {
    // Use a known date: 2023-06-15 = days since epoch ~ 19523
    let date_val = Value::Date(Date(19523)); // approx 2023-06-15
    let year = ScalarFunction::Date { op: DateOp::Year };
    let month = ScalarFunction::Date { op: DateOp::Month };
    let day = ScalarFunction::Date { op: DateOp::Day };
    assert_eq!(evaluate_scalar(&year, &[date_val.clone()]).unwrap(), Value::Int64(2023));
    assert_eq!(evaluate_scalar(&month, &[date_val.clone()]).unwrap(), Value::Int64(6));
    assert_eq!(evaluate_scalar(&day, &[date_val]).unwrap(), Value::Int64(15));
}

#[test]
fn test_date_part() {
    let date_val = Value::Date(Date(19523)); // 2023-06-15
    let dp = ScalarFunction::Date { op: DateOp::DatePart };
    assert_eq!(
        evaluate_scalar(&dp, &[Value::String("year".into()), date_val.clone()]).unwrap(),
        Value::Int64(2023)
    );
    assert_eq!(
        evaluate_scalar(&dp, &[Value::String("month".into()), date_val.clone()]).unwrap(),
        Value::Int64(6)
    );
    assert_eq!(
        evaluate_scalar(&dp, &[Value::String("day".into()), date_val]).unwrap(),
        Value::Int64(15)
    );
}

#[test]
fn test_date_trunc() {
    let date_val = Value::Date(Date(19600)); // some date in 2023
    let dt = ScalarFunction::Date { op: DateOp::DateTrunc };
    let result = evaluate_scalar(&dt, &[Value::String("year".into()), date_val]).unwrap();
    assert!(matches!(result, Value::Date(_)));
}

#[test]
fn test_date_diff() {
    let d1 = Value::Date(Date(19000));
    let d2 = Value::Date(Date(19500));
    let dd = ScalarFunction::Date { op: DateOp::DateDiff };
    let days = evaluate_scalar(&dd, &[Value::String("day".into()), d1, d2]).unwrap();
    assert_eq!(days, Value::Int64(500));
}

#[test]
fn test_date_add() {
    let date_val = Value::Date(Date(19523)); // 2023-06-15
    let da = ScalarFunction::Date { op: DateOp::DateAdd };
    let result = evaluate_scalar(&da, &[Value::String("day".into()), Value::Int64(7), date_val]).unwrap();
    assert!(matches!(result, Value::Date(_)));
}

// --- Timestamp function tests ---

#[test]
fn test_century() {
    let func = ScalarFunction::Date { op: DateOp::Century };
    // Use a date in the 21st century (e.g., 2023-06-15 = ~19523 days from epoch)
    let date_val = Value::Date(Date(19523));
    assert_eq!(evaluate_scalar(&func, &[date_val]).unwrap(), Value::Int64(21));
    // Year 2000 → century 20 (2000-01-01 = ~10957 days from epoch)
    let y2000 = Value::Date(Date(10957));
    assert_eq!(evaluate_scalar(&func, &[y2000]).unwrap(), Value::Int64(20));
    // Also works with timestamp
    let ts = Value::Timestamp(Timestamp(19523i64 * 86400 * 1_000_000));
    assert_eq!(evaluate_scalar(&func, &[ts]).unwrap(), Value::Int64(21));
}

#[test]
fn test_epoch_ms() {
    let func = ScalarFunction::Date { op: DateOp::EpochMs };
    // 0 ms → epoch
    let result = evaluate_scalar(&func, &[Value::Int64(0)]).unwrap();
    assert_eq!(result, Value::Timestamp(Timestamp(0)));
    // 1000 ms = 1 sec → Timestamp(1_000_000 micros)
    let result = evaluate_scalar(&func, &[Value::Int64(1000)]).unwrap();
    assert_eq!(result, Value::Timestamp(Timestamp(1_000_000)));
}

#[test]
fn test_to_timestamp() {
    let func = ScalarFunction::Date {
        op: DateOp::ToTimestamp,
    };
    // 0 seconds → epoch
    let result = evaluate_scalar(&func, &[Value::Double(0.0)]).unwrap();
    assert_eq!(result, Value::Timestamp(Timestamp(0)));
    // 1 second → 1_000_000 micros
    let result = evaluate_scalar(&func, &[Value::Double(1.0)]).unwrap();
    assert_eq!(result, Value::Timestamp(Timestamp(1_000_000)));
    // Integer input
    let result = evaluate_scalar(&func, &[Value::Int64(0)]).unwrap();
    assert_eq!(result, Value::Timestamp(Timestamp(0)));
}

#[test]
fn test_to_epoch_ms() {
    let func = ScalarFunction::Date { op: DateOp::ToEpochMs };
    // Epoch → 0 ms
    let result = evaluate_scalar(&func, &[Value::Timestamp(Timestamp(0))]).unwrap();
    assert_eq!(result, Value::Int64(0));
    // 1 ms = 1000 micros → 1 ms
    let result = evaluate_scalar(&func, &[Value::Timestamp(Timestamp(1000))]).unwrap();
    assert_eq!(result, Value::Int64(1));
}

// --- Interval constructor function tests ---

#[test]
fn test_to_years() {
    let func = ScalarFunction::Interval {
        op: IntervalOp::ToYears,
    };
    let result = evaluate_scalar(&func, &[Value::Int64(3)]).unwrap();
    assert_eq!(result, Value::Interval(Interval::new(36, 0, 0)));
}

#[test]
fn test_to_months() {
    let func = ScalarFunction::Interval {
        op: IntervalOp::ToMonths,
    };
    let result = evaluate_scalar(&func, &[Value::Int64(5)]).unwrap();
    assert_eq!(result, Value::Interval(Interval::new(5, 0, 0)));
}

#[test]
fn test_to_days() {
    let func = ScalarFunction::Interval { op: IntervalOp::ToDays };
    let result = evaluate_scalar(&func, &[Value::Int64(10)]).unwrap();
    assert_eq!(result, Value::Interval(Interval::new(0, 10, 0)));
}

#[test]
fn test_to_hours() {
    let func = ScalarFunction::Interval {
        op: IntervalOp::ToHours,
    };
    let result = evaluate_scalar(&func, &[Value::Int64(2)]).unwrap();
    assert_eq!(result, Value::Interval(Interval::new(0, 0, 7_200_000_000)));
}

#[test]
fn test_to_minutes() {
    let func = ScalarFunction::Interval {
        op: IntervalOp::ToMinutes,
    };
    let result = evaluate_scalar(&func, &[Value::Int64(30)]).unwrap();
    assert_eq!(result, Value::Interval(Interval::new(0, 0, 1_800_000_000)));
}

#[test]
fn test_to_seconds() {
    let func = ScalarFunction::Interval {
        op: IntervalOp::ToSeconds,
    };
    let result = evaluate_scalar(&func, &[Value::Int64(45)]).unwrap();
    assert_eq!(result, Value::Interval(Interval::new(0, 0, 45_000_000)));
}

#[test]
fn test_to_milliseconds() {
    let func = ScalarFunction::Interval {
        op: IntervalOp::ToMilliseconds,
    };
    let result = evaluate_scalar(&func, &[Value::Int64(500)]).unwrap();
    assert_eq!(result, Value::Interval(Interval::new(0, 0, 500_000)));
}

#[test]
fn test_to_microseconds() {
    let func = ScalarFunction::Interval {
        op: IntervalOp::ToMicroseconds,
    };
    let result = evaluate_scalar(&func, &[Value::Int64(999)]).unwrap();
    assert_eq!(result, Value::Interval(Interval::new(0, 0, 999)));
}

// --- Blob function tests ---

#[test]
fn test_encode() {
    let func = ScalarFunction::Blob { op: BlobOp::Encode };
    let result = evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap();
    assert_eq!(result, Value::Blob(b"hello".to_vec()));
    // Empty string
    let result = evaluate_scalar(&func, &[Value::String("".into())]).unwrap();
    assert_eq!(result, Value::Blob(vec![]));
}

#[test]
fn test_decode() {
    let func = ScalarFunction::Blob { op: BlobOp::Decode };
    let result = evaluate_scalar(&func, &[Value::Blob(b"hello".to_vec())]).unwrap();
    assert_eq!(result, Value::String("hello".into()));
    // Invalid UTF-8 should error
    let result = evaluate_scalar(&func, &[Value::Blob(vec![0xFF, 0xFE])]);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("UTF8"));
}

#[test]
fn test_octet_length() {
    let func = ScalarFunction::Blob {
        op: BlobOp::OctetLength,
    };
    let result = evaluate_scalar(&func, &[Value::Blob(b"hello".to_vec())]).unwrap();
    assert_eq!(result, Value::Int64(5));
    // Empty blob
    let result = evaluate_scalar(&func, &[Value::Blob(vec![])]).unwrap();
    assert_eq!(result, Value::Int64(0));
}

// --- Union function tests ---

#[test]
fn test_union_value() {
    let func = ScalarFunction::Union {
        op: UnionOp::UnionValue,
    };
    let result = evaluate_scalar(&func, &[Value::Int64(42)]).unwrap();
    // Should produce a union wrapping the value
    assert_eq!(
        result,
        Value::Struct(vec![
            ("tag".to_string(), Value::UInt16(0)),
            ("_value".to_string(), Value::Int64(42)),
        ])
    );
}

#[test]
fn test_union_tag() {
    // Create a union with tag=1 (second variant active)
    let union_val = Value::Struct(vec![
        ("tag".to_string(), Value::UInt16(1)),
        ("a".to_string(), Value::Int64(10)),
        ("b".to_string(), Value::String("hello".into())),
    ]);
    let func = ScalarFunction::Union { op: UnionOp::UnionTag };
    let result = evaluate_scalar(&func, &[union_val]).unwrap();
    assert_eq!(result, Value::String("b".into()));
}

#[test]
fn test_union_extract() {
    let union_val = Value::Struct(vec![
        ("tag".to_string(), Value::UInt16(0)),
        ("a".to_string(), Value::Int64(10)),
        ("b".to_string(), Value::String("hello".into())),
    ]);
    let func = ScalarFunction::Union {
        op: UnionOp::UnionExtract,
    };
    // Extract field "a"
    let result = evaluate_scalar(&func, &[union_val.clone(), Value::String("a".into())]).unwrap();
    assert_eq!(result, Value::Int64(10));
    // Extract field "b"
    let result = evaluate_scalar(&func, &[union_val.clone(), Value::String("b".into())]).unwrap();
    assert_eq!(result, Value::String("hello".into()));
    // Non-existent key
    let result = evaluate_scalar(&func, &[union_val, Value::String("c".into())]);
    assert!(result.is_err());
}

// --- List function tests ---
#[test]
fn test_list_creation() {
    let func = ScalarFunction::List { op: ListOp::Creation };
    let result = evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(2), Value::Int64(3)]).unwrap();
    match result {
        Value::List(items) => assert_eq!(items.len(), 3),
        _ => panic!("Expected list"),
    }
}

#[test]
fn test_array_value() {
    // array_value is an alias for ListOp::Creation
    let func = ScalarFunction::List { op: ListOp::Creation };
    let result = evaluate_scalar(
        &func,
        &[Value::Int64(10), Value::Int64(20), Value::Int64(30), Value::Int64(40)],
    )
    .unwrap();
    match result {
        Value::List(items) => {
            assert_eq!(items.len(), 4);
            assert_eq!(items[0], Value::Int64(10));
            assert_eq!(items[2], Value::Int64(30));
        }
        _ => panic!("Expected list"),
    }
}

#[test]
fn test_list_concat() {
    let func = ScalarFunction::List { op: ListOp::Concat };
    let l1 = Value::List(vec![Value::Int64(1), Value::Int64(2)]);
    let l2 = Value::List(vec![Value::Int64(3), Value::Int64(4)]);
    let result = evaluate_scalar(&func, &[l1, l2]).unwrap();
    match result {
        Value::List(items) => {
            assert_eq!(items.len(), 4);
            assert_eq!(items[0], Value::Int64(1));
            assert_eq!(items[3], Value::Int64(4));
        }
        _ => panic!("Expected list"),
    }
}

#[test]
fn test_list_sort() {
    let func = ScalarFunction::List { op: ListOp::Sort };
    let list = Value::List(vec![Value::Int64(3), Value::Int64(1), Value::Int64(2)]);
    let result = evaluate_scalar(&func, &[list]).unwrap();
    match result {
        Value::List(items) => {
            assert_eq!(items[0], Value::Int64(1));
            assert_eq!(items[1], Value::Int64(2));
            assert_eq!(items[2], Value::Int64(3));
        }
        _ => panic!("Expected list"),
    }
}

#[test]
fn test_list_prepend() {
    let func = ScalarFunction::List { op: ListOp::Prepend };
    let result = evaluate_scalar(
        &func,
        &[Value::List(vec![Value::Int64(2), Value::Int64(3)]), Value::Int64(1)],
    )
    .unwrap();
    match result {
        Value::List(items) => {
            assert_eq!(items[0], Value::Int64(1));
            assert_eq!(items.len(), 3);
        }
        _ => panic!("Expected list"),
    }
}

#[test]
fn test_list_reverse() {
    let func = ScalarFunction::List { op: ListOp::Reverse };
    let result = evaluate_scalar(
        &func,
        &[Value::List(vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)])],
    )
    .unwrap();
    match result {
        Value::List(items) => {
            assert_eq!(items[0], Value::Int64(3));
            assert_eq!(items[2], Value::Int64(1));
        }
        _ => panic!("Expected list"),
    }
}

#[test]
fn test_list_extract() {
    let func = ScalarFunction::List { op: ListOp::Extract };
    let list = Value::List(vec![Value::Int64(10), Value::Int64(20), Value::Int64(30)]);
    assert_eq!(
        evaluate_scalar(&func, &[list, Value::Int64(2)]).unwrap(),
        Value::Int64(20)
    );
}

// --- Map function tests ---
#[test]
fn test_map_creation() {
    let func = ScalarFunction::Map { op: MapOp::Creation };
    let result = evaluate_scalar(
        &func,
        &[
            Value::String("a".into()),
            Value::Int64(1),
            Value::String("b".into()),
            Value::Int64(2),
        ],
    )
    .unwrap();
    match result {
        Value::Struct(entries) => {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].0, "a");
            assert_eq!(entries[1].0, "b");
        }
        _ => panic!("Expected struct"),
    }
}

#[test]
fn test_map_extract() {
    let func = ScalarFunction::Map { op: MapOp::Extract };
    let map_val = Value::Struct(vec![
        ("x".into(), Value::Int64(42)),
        ("y".into(), Value::String("hello".into())),
    ]);
    assert_eq!(
        evaluate_scalar(&func, &[map_val.clone(), Value::String("x".into())]).unwrap(),
        Value::Int64(42)
    );
    assert_eq!(
        evaluate_scalar(&func, &[map_val, Value::String("y".into())]).unwrap(),
        Value::String("hello".into())
    );
}

#[test]
fn test_map_contains() {
    let func = ScalarFunction::Map { op: MapOp::Contains };
    let map_val = Value::Struct(vec![("a".into(), Value::Int64(1))]);
    assert_eq!(
        evaluate_scalar(&func, &[map_val.clone(), Value::String("a".into())]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_scalar(&func, &[map_val, Value::String("b".into())]).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn test_map_keys_values() {
    let keys = ScalarFunction::Map { op: MapOp::Keys };
    let values = ScalarFunction::Map { op: MapOp::Values };
    let map_val = Value::Struct(vec![("a".into(), Value::Int64(1)), ("b".into(), Value::Int64(2))]);

    let key_result = evaluate_scalar(&keys, &[map_val.clone()]).unwrap();
    match key_result {
        Value::List(items) => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], Value::String("a".into()));
        }
        _ => panic!("Expected list"),
    }

    let val_result = evaluate_scalar(&values, &[map_val]).unwrap();
    match val_result {
        Value::List(items) => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], Value::Int64(1));
        }
        _ => panic!("Expected list"),
    }
}

// --- Struct function tests ---
#[test]
fn test_struct_creation() {
    let func = ScalarFunction::Struct { op: StructOp::Creation };
    let result = evaluate_scalar(
        &func,
        &[
            Value::String("name".into()),
            Value::String("Alice".into()),
            Value::String("age".into()),
            Value::Int64(30),
        ],
    )
    .unwrap();
    match result {
        Value::Struct(entries) => {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].0, "name");
            assert_eq!(entries[1].0, "age");
        }
        _ => panic!("Expected struct"),
    }
}

#[test]
fn test_struct_extract() {
    let func = ScalarFunction::Struct { op: StructOp::Extract };
    let s = Value::Struct(vec![("name".into(), Value::String("Bob".into()))]);
    assert_eq!(
        evaluate_scalar(&func, &[s, Value::String("name".into())]).unwrap(),
        Value::String("Bob".into())
    );
}

// --- Cast function tests ---
#[test]
fn test_cast_int32() {
    let func = ScalarFunction::Cast {
        target_type: CastTarget::Int32,
    };
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(42)]).unwrap(), Value::Int32(42));
    assert_eq!(evaluate_scalar(&func, &[Value::Double(3.14)]).unwrap(), Value::Int32(3));
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("99".into())]).unwrap(),
        Value::Int32(99)
    );
}

#[test]
fn test_cast_float() {
    let func = ScalarFunction::Cast {
        target_type: CastTarget::Float,
    };
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(42)]).unwrap(), Value::Float(42.0));
    let result = evaluate_scalar(&func, &[Value::String("3.14".into())]).unwrap();
    match result {
        Value::Float(x) => assert!((x - 3.14).abs() < 0.001),
        _ => panic!("Expected float"),
    }
}

#[test]
fn test_cast_bool() {
    let func = ScalarFunction::Cast {
        target_type: CastTarget::Bool,
    };
    assert_eq!(evaluate_scalar(&func, &[Value::Bool(true)]).unwrap(), Value::Bool(true));
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(1)]).unwrap(), Value::Bool(true));
    assert_eq!(evaluate_scalar(&func, &[Value::Int64(0)]).unwrap(), Value::Bool(false));
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("true".into())]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("false".into())]).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn test_cast_date_timestamp() {
    let cast_date = ScalarFunction::Cast {
        target_type: CastTarget::Date,
    };
    let cast_ts = ScalarFunction::Cast {
        target_type: CastTarget::Timestamp,
    };

    let d = Value::Date(Date(100));
    assert_eq!(
        evaluate_scalar(&cast_date, &[d.clone()]).unwrap(),
        Value::Date(Date(100))
    );

    let ts = evaluate_scalar(&cast_ts, &[d]).unwrap();
    assert!(matches!(ts, Value::Timestamp(_)));
}

// --- Aggregate function tests ---
#[test]
fn test_aggregate_count() {
    let result = evaluate_aggregate(
        &AggregateFunction::Count,
        &[Value::Int64(1), Value::Int64(2), Value::Int64(3)],
    );
    assert_eq!(result.unwrap(), Value::Int64(3));
}

#[test]
fn test_aggregate_count_star() {
    let result = evaluate_aggregate(&AggregateFunction::CountStar, &[Value::Null, Value::Int64(1)]);
    assert_eq!(result.unwrap(), Value::Int64(2));
}

#[test]
fn test_aggregate_sum() {
    let result = evaluate_aggregate(
        &AggregateFunction::Sum,
        &[Value::Int64(1), Value::Int64(2), Value::Int64(3)],
    );
    assert_eq!(result.unwrap(), Value::Int64(6));
}

#[test]
fn test_aggregate_avg() {
    let result = evaluate_aggregate(
        &AggregateFunction::Avg,
        &[Value::Int64(1), Value::Int64(2), Value::Int64(3)],
    );
    match result.unwrap() {
        Value::Double(x) => assert!((x - 2.0).abs() < 1e-10),
        _ => panic!("Expected double"),
    }
}

#[test]
fn test_aggregate_min_max() {
    let values = &[Value::Int64(5), Value::Int64(2), Value::Int64(8), Value::Int64(1)];
    assert_eq!(
        evaluate_aggregate(&AggregateFunction::Min, values).unwrap(),
        Value::Int64(1)
    );
    assert_eq!(
        evaluate_aggregate(&AggregateFunction::Max, values).unwrap(),
        Value::Int64(8)
    );
}

#[test]
fn test_aggregate_collect() {
    let result = evaluate_aggregate(&AggregateFunction::Collect, &[Value::Int64(1), Value::Int64(2)]);
    match result.unwrap() {
        Value::List(items) => assert_eq!(items.len(), 2),
        _ => panic!("Expected list"),
    }
}

#[test]
fn test_aggregate_skip_null() {
    let result = evaluate_aggregate(
        &AggregateFunction::Sum,
        &[Value::Int64(1), Value::Null, Value::Int64(2)],
    );
    assert_eq!(result.unwrap(), Value::Int64(3));
}

#[test]
fn test_aggregate_empty() {
    let result = evaluate_aggregate(&AggregateFunction::Count, &[]);
    assert_eq!(result.unwrap(), Value::Int64(0));
}

#[test]
fn test_aggregate_double_sum() {
    let result = evaluate_aggregate(&AggregateFunction::Sum, &[Value::Double(1.5), Value::Double(2.5)]);
    match result.unwrap() {
        Value::Double(x) => assert!((x - 4.0).abs() < 1e-10),
        _ => panic!("Expected double"),
    }
}

// --- AggValueState tests ---
#[test]
fn test_agg_value_state_new() {
    let state = AggValueState::new(&AggregateFunction::Count);
    assert!(matches!(state, AggValueState::Count(0)));
    let state = AggValueState::new(&AggregateFunction::Sum);
    assert!(matches!(state, AggValueState::Sum(Value::Null)));
    let state = AggValueState::new(&AggregateFunction::Collect);
    assert!(matches!(state, AggValueState::Collect(_)));
}

#[test]
fn test_agg_state_stddev() {
    let mut state = AggValueState::new(&AggregateFunction::StdDev);
    state.update(&Value::Double(2.0));
    state.update(&Value::Double(4.0));
    state.update(&Value::Double(6.0));
    let result = state.finalize();
    match result {
        Value::Double(x) => assert!((x - 1.63299).abs() < 0.001),
        _ => panic!("Expected double, got {:?}", result),
    }
}

#[test]
fn test_agg_state_variance() {
    let mut state = AggValueState::new(&AggregateFunction::Variance);
    state.update(&Value::Double(2.0));
    state.update(&Value::Double(4.0));
    state.update(&Value::Double(6.0));
    let result = state.finalize();
    match result {
        Value::Double(x) => assert!((x - 2.66666).abs() < 0.001),
        _ => panic!("Expected double"),
    }
}

#[test]
fn test_agg_state_percentile_disc() {
    let mut state = AggValueState::new(&AggregateFunction::PercentileDisc { percentile: 0.5 });
    state.update(&Value::Double(1.0));
    state.update(&Value::Double(3.0));
    state.update(&Value::Double(7.0));
    state.update(&Value::Double(9.0));
    // 4 values, 0.5 * 4 = 2 → ceil(2) = 2 → index 1 → 3.0
    let result = state.finalize();
    match result {
        Value::Double(x) => assert!((x - 3.0).abs() < 0.001, "Expected median 3.0, got {}", x),
        _ => panic!("Expected double"),
    }
}

#[test]
fn test_agg_state_percentile_disc_90th() {
    let mut state = AggValueState::new(&AggregateFunction::PercentileDisc { percentile: 0.9 });
    for v in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0] {
        state.update(&Value::Double(v));
    }
    // 10 values, 0.9 * 10 = 9 → ceil(9) = 9 → index 8 → 9.0
    let result = state.finalize();
    match result {
        Value::Double(x) => assert!((x - 9.0).abs() < 0.001),
        _ => panic!("Expected double"),
    }
}

#[test]
fn test_agg_state_percentile_skip_null() {
    let mut state = AggValueState::new(&AggregateFunction::PercentileDisc { percentile: 0.5 });
    state.update(&Value::Null);
    state.update(&Value::Double(10.0));
    state.update(&Value::Double(20.0));
    // 2 values: 10, 20. 0.5 * 2 = 1 → ceil(1) = 1 → index 0 → 10.0
    let result = state.finalize();
    match result {
        Value::Double(x) => assert!((x - 10.0).abs() < 0.001),
        _ => panic!("Expected double"),
    }
}

#[test]
fn test_agg_state_percentile_empty() {
    let state = AggValueState::new(&AggregateFunction::PercentileDisc { percentile: 0.5 });
    let result = state.finalize();
    assert_eq!(result, Value::Null);
}

#[test]
fn test_agg_state_percentile_cont() {
    let mut state = AggValueState::new(&AggregateFunction::PercentileCont { percentile: 0.5 });
    state.update(&Value::Double(1.0));
    state.update(&Value::Double(5.0));
    // 2 values, 0.5 * 2 = 1 → ceil(1) = 1 → index 0 → 1.0 (same as disc for small N)
    let result = state.finalize();
    match result {
        Value::Double(x) => assert!((x - 1.0).abs() < 0.001),
        _ => panic!("Expected double"),
    }
}

// --- AggValueState merge tests ---
#[test]
fn test_agg_state_merge_count() {
    let mut a = AggValueState::new(&AggregateFunction::Count);
    let b = AggValueState::Count(5);
    a.update(&Value::Int64(1));
    a.update(&Value::Int64(2));
    a.merge(&b);
    assert_eq!(a.finalize(), Value::Int64(7)); // 2 + 5
}

#[test]
fn test_agg_state_merge_sum() {
    let mut a = AggValueState::new(&AggregateFunction::Sum);
    let mut b = AggValueState::new(&AggregateFunction::Sum);
    a.update(&Value::Int64(10));
    b.update(&Value::Int64(20));
    b.update(&Value::Int64(30));
    a.merge(&b);
    assert_eq!(a.finalize(), Value::Int64(60));
}

#[test]
fn test_agg_state_merge_avg() {
    let mut a = AggValueState::new(&AggregateFunction::Avg);
    let mut b = AggValueState::new(&AggregateFunction::Avg);
    a.update(&Value::Double(10.0));
    b.update(&Value::Double(20.0));
    b.update(&Value::Double(30.0));
    a.merge(&b);
    match a.finalize() {
        Value::Double(x) => assert!((x - 20.0).abs() < 0.001),
        _ => panic!("Expected double"),
    }
}

#[test]
fn test_agg_state_merge_percentile() {
    let mut a = AggValueState::new(&AggregateFunction::PercentileDisc { percentile: 0.5 });
    let mut b = AggValueState::new(&AggregateFunction::PercentileDisc { percentile: 0.5 });
    a.update(&Value::Double(1.0));
    a.update(&Value::Double(3.0));
    b.update(&Value::Double(7.0));
    b.update(&Value::Double(9.0));
    a.merge(&b);
    match a.finalize() {
        Value::Double(x) => assert!((x - 3.0).abs() < 0.001),
        _ => panic!("Expected double"),
    }
}

// --- Schema function tests ---
#[test]
fn test_schema_offset_internal_id() {
    let func = ScalarFunction::Schema { op: SchemaOp::Offset };
    let id = kuzu_common::types::InternalID {
        table_id: 1,
        offset: 42,
    };
    assert_eq!(
        evaluate_scalar(&func, &[Value::InternalID(id)]).unwrap(),
        Value::Int64(42)
    );
}

#[test]
fn test_schema_offset_struct() {
    let func = ScalarFunction::Schema { op: SchemaOp::Offset };
    let id = kuzu_common::types::InternalID {
        table_id: 1,
        offset: 99,
    };
    let node = Value::Struct(vec![("_id".into(), Value::InternalID(id))]);
    assert_eq!(evaluate_scalar(&func, &[node]).unwrap(), Value::Int64(99));
}

#[test]
fn test_schema_offset_error() {
    let func = ScalarFunction::Schema { op: SchemaOp::Offset };
    assert!(evaluate_scalar(&func, &[Value::Int64(42)]).is_err());
}

#[test]
fn test_schema_id_internal_id() {
    let func = ScalarFunction::Schema { op: SchemaOp::Id };
    let id = kuzu_common::types::InternalID {
        table_id: 5,
        offset: 100,
    };
    assert_eq!(
        evaluate_scalar(&func, &[Value::InternalID(id)]).unwrap(),
        Value::InternalID(id)
    );
}

#[test]
fn test_schema_id_from_struct() {
    let func = ScalarFunction::Schema { op: SchemaOp::Id };
    let id = kuzu_common::types::InternalID {
        table_id: 2,
        offset: 77,
    };
    let node = Value::Struct(vec![("_id".into(), Value::InternalID(id))]);
    assert_eq!(evaluate_scalar(&func, &[node]).unwrap(), Value::InternalID(id));
}

#[test]
fn test_schema_start_end_node() {
    let start_func = ScalarFunction::Schema {
        op: SchemaOp::StartNode,
    };
    let end_func = ScalarFunction::Schema { op: SchemaOp::EndNode };
    let src_id = kuzu_common::types::InternalID {
        table_id: 1,
        offset: 10,
    };
    let dst_id = kuzu_common::types::InternalID {
        table_id: 1,
        offset: 20,
    };
    let rel = Value::Struct(vec![
        ("_src".into(), Value::InternalID(src_id)),
        ("_dst".into(), Value::InternalID(dst_id)),
    ]);
    assert_eq!(
        evaluate_scalar(&start_func, &[rel.clone()]).unwrap(),
        Value::InternalID(src_id)
    );
    assert_eq!(evaluate_scalar(&end_func, &[rel]).unwrap(), Value::InternalID(dst_id));
}

#[test]
fn test_schema_label_string() {
    let func = ScalarFunction::Schema { op: SchemaOp::Label };
    assert_eq!(
        evaluate_scalar(&func, &[Value::String("Person".into())]).unwrap(),
        Value::String("Person".into())
    );
}

#[test]
fn test_schema_label_struct() {
    let func = ScalarFunction::Schema { op: SchemaOp::Label };
    let node = Value::Struct(vec![("_label".into(), Value::String("Person".into()))]);
    assert_eq!(evaluate_scalar(&func, &[node]).unwrap(), Value::String("Person".into()));
}

#[test]
fn test_schema_label_internal_id() {
    let func = ScalarFunction::Schema { op: SchemaOp::Label };
    let id = kuzu_common::types::InternalID { table_id: 3, offset: 0 };
    let result = evaluate_scalar(&func, &[Value::InternalID(id)]).unwrap();
    assert!(matches!(result, Value::String(_)));
    if let Value::String(s) = result {
        assert!(s.contains("3"));
    }
}

#[test]
fn test_schema_empty_args() {
    let func = ScalarFunction::Schema { op: SchemaOp::Label };
    assert!(evaluate_scalar(&func, &[]).is_err());
}

#[test]
fn test_schema_registry_contains() {
    let reg = FunctionRegistry::new();
    assert!(reg.contains("OFFSET"));
    assert!(reg.contains("ID"));
    assert!(reg.contains("START_NODE"));
    assert!(reg.contains("END_NODE"));
    assert!(reg.contains("LABEL"));
}

// --- Array function tests ---
#[test]
fn test_array_cosine_similarity() {
    let func = ScalarFunction::Array {
        op: ArrayOp::CosineSimilarity,
    };
    let a = Value::List(vec![Value::Double(1.0), Value::Double(0.0)]);
    let b = Value::List(vec![Value::Double(0.0), Value::Double(1.0)]);
    let result = evaluate_scalar(&func, &[a, b]).unwrap();
    if let Value::Double(x) = result {
        assert!((x - 0.0).abs() < 1e-10);
    } else {
        panic!("Expected Double");
    }
}

#[test]
fn test_array_cosine_identical() {
    let func = ScalarFunction::Array {
        op: ArrayOp::CosineSimilarity,
    };
    let a = Value::List(vec![Value::Double(3.0), Value::Double(4.0)]);
    let b = Value::List(vec![Value::Double(3.0), Value::Double(4.0)]);
    let result = evaluate_scalar(&func, &[a, b]).unwrap();
    if let Value::Double(x) = result {
        assert!((x - 1.0).abs() < 1e-10);
    } else {
        panic!("Expected Double");
    }
}

#[test]
fn test_array_distance() {
    let func = ScalarFunction::Array { op: ArrayOp::Distance };
    let a = Value::List(vec![Value::Double(0.0), Value::Double(0.0)]);
    let b = Value::List(vec![Value::Double(3.0), Value::Double(4.0)]);
    let result = evaluate_scalar(&func, &[a, b]).unwrap();
    if let Value::Double(x) = result {
        assert!((x - 5.0).abs() < 1e-10);
    } else {
        panic!("Expected Double");
    }
}

#[test]
fn test_array_inner_product() {
    let func = ScalarFunction::Array {
        op: ArrayOp::InnerProduct,
    };
    let a = Value::List(vec![Value::Double(1.0), Value::Double(2.0)]);
    let b = Value::List(vec![Value::Double(3.0), Value::Double(4.0)]);
    let result = evaluate_scalar(&func, &[a, b]).unwrap();
    assert_eq!(result, Value::Double(11.0));
}

#[test]
fn test_array_cross_product() {
    let func = ScalarFunction::Array {
        op: ArrayOp::CrossProduct,
    };
    let a = Value::List(vec![Value::Double(1.0), Value::Double(0.0), Value::Double(0.0)]);
    let b = Value::List(vec![Value::Double(0.0), Value::Double(1.0), Value::Double(0.0)]);
    let result = evaluate_scalar(&func, &[a, b]).unwrap();
    match result {
        Value::List(items) => {
            assert_eq!(items.len(), 3);
            if let Value::Double(z) = &items[2] {
                assert!((z - 1.0).abs() < 1e-10);
            } else {
                panic!("Expected Double for z-component");
            }
        }
        _ => panic!("Expected List"),
    }
}

#[test]
fn test_array_cross_product_wrong_dim() {
    let func = ScalarFunction::Array {
        op: ArrayOp::CrossProduct,
    };
    let a = Value::List(vec![Value::Double(1.0), Value::Double(2.0)]);
    let b = Value::List(vec![Value::Double(3.0), Value::Double(4.0)]);
    assert!(evaluate_scalar(&func, &[a, b]).is_err());
}

#[test]
fn test_array_squared_distance() {
    let func = ScalarFunction::Array {
        op: ArrayOp::SquaredDistance,
    };
    let a = Value::List(vec![Value::Double(0.0), Value::Double(0.0)]);
    let b = Value::List(vec![Value::Double(3.0), Value::Double(4.0)]);
    let result = evaluate_scalar(&func, &[a, b]).unwrap();
    assert_eq!(result, Value::Double(25.0));
}

#[test]
fn test_array_diff_length() {
    let func = ScalarFunction::Array { op: ArrayOp::Distance };
    let a = Value::List(vec![Value::Double(1.0)]);
    let b = Value::List(vec![Value::Double(1.0), Value::Double(2.0)]);
    assert!(evaluate_scalar(&func, &[a, b]).is_err());
}

#[test]
fn test_list_slice() {
    let func = ScalarFunction::List { op: ListOp::Slice };
    let list = Value::List(vec![
        Value::Int64(10),
        Value::Int64(20),
        Value::Int64(30),
        Value::Int64(40),
        Value::Int64(50),
    ]);
    let result = evaluate_scalar(&func, &[list, Value::Int64(2), Value::Int64(4)]).unwrap();
    match result {
        Value::List(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], Value::Int64(20));
            assert_eq!(items[2], Value::Int64(40));
        }
        _ => panic!("Expected List"),
    }
}

#[test]
fn test_list_slice_single_arg() {
    let func = ScalarFunction::List { op: ListOp::Slice };
    let list = Value::List(vec![Value::Int64(10), Value::Int64(20), Value::Int64(30)]);
    let result = evaluate_scalar(&func, &[list, Value::Int64(2)]).unwrap();
    match result {
        Value::List(items) => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], Value::Int64(20));
            assert_eq!(items[1], Value::Int64(30));
        }
        _ => panic!("Expected List"),
    }
}

#[test]
fn test_array_registry_contains() {
    let reg = FunctionRegistry::new();
    assert!(reg.contains("array_cosine_similarity"));
    assert!(reg.contains("array_distance"));
    assert!(reg.contains("array_inner_product"));
    assert!(reg.contains("array_cross_product"));
    assert!(reg.contains("array_squared_distance"));
    assert!(reg.contains("list_slice"));
    assert!(reg.contains("list_prepend"));
    // Array utility aliases
    assert!(reg.contains("array_concat"), "array_concat should be registered");
    assert!(reg.contains("array_cat"), "array_cat should be registered");
    assert!(reg.contains("array_append"), "array_append should be registered");
    assert!(reg.contains("array_push_back"), "array_push_back should be registered");
    assert!(reg.contains("array_prepend"), "array_prepend should be registered");
    assert!(
        reg.contains("array_push_front"),
        "array_push_front should be registered"
    );
    assert!(reg.contains("array_contains"), "array_contains should be registered");
    assert!(reg.contains("array_has"), "array_has should be registered");
    assert!(reg.contains("array_slice"), "array_slice should be registered");
}

// --- List functions (C++ port) tests ---

#[test]
fn test_range() {
    let func = ScalarFunction::List { op: ListOp::Range };
    // 1-arg: range(end) → [0, 1, ..., end]
    let result = evaluate_scalar(&func, &[Value::Int64(3)]).unwrap();
    assert_eq!(
        result,
        Value::List(vec![Value::Int64(0), Value::Int64(1), Value::Int64(2), Value::Int64(3),])
    );
    // 2-arg: range(start, end)
    let result = evaluate_scalar(&func, &[Value::Int64(2), Value::Int64(5)]).unwrap();
    assert_eq!(
        result,
        Value::List(vec![Value::Int64(2), Value::Int64(3), Value::Int64(4), Value::Int64(5),])
    );
    // 3-arg: range(start, end, step)
    let result = evaluate_scalar(&func, &[Value::Int64(0), Value::Int64(6), Value::Int64(2)]).unwrap();
    assert_eq!(
        result,
        Value::List(vec![Value::Int64(0), Value::Int64(2), Value::Int64(4), Value::Int64(6),])
    );
    // Zero step → error
    assert!(evaluate_scalar(&func, &[Value::Int64(0), Value::Int64(5), Value::Int64(0)]).is_err());
}

#[test]
fn test_list_distinct() {
    let func = ScalarFunction::List { op: ListOp::Distinct };
    let result = evaluate_scalar(
        &func,
        &[Value::List(vec![
            Value::Int64(1),
            Value::Int64(2),
            Value::Int64(1),
            Value::Int64(3),
        ])],
    )
    .unwrap();
    if let Value::List(items) = result {
        assert_eq!(items.len(), 3);
        assert!(items.contains(&Value::Int64(1)));
        assert!(items.contains(&Value::Int64(2)));
        assert!(items.contains(&Value::Int64(3)));
    } else {
        panic!("Expected list");
    }
}

#[test]
fn test_list_unique() {
    let func = ScalarFunction::List { op: ListOp::Unique };
    // All unique → count = 3
    let result = evaluate_scalar(
        &func,
        &[Value::List(vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)])],
    )
    .unwrap();
    assert_eq!(result, Value::Int64(3));
    // Duplicates → count = 2
    let result = evaluate_scalar(
        &func,
        &[Value::List(vec![Value::Int64(1), Value::Int64(2), Value::Int64(1)])],
    )
    .unwrap();
    assert_eq!(result, Value::Int64(2));
}

#[test]
fn test_list_sum() {
    let func = ScalarFunction::List { op: ListOp::Sum };
    let result = evaluate_scalar(
        &func,
        &[Value::List(vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)])],
    )
    .unwrap();
    assert_eq!(result, Value::Int64(6));
}

#[test]
fn test_list_product() {
    let func = ScalarFunction::List { op: ListOp::Product };
    let result = evaluate_scalar(
        &func,
        &[Value::List(vec![Value::Int64(2), Value::Int64(3), Value::Int64(4)])],
    )
    .unwrap();
    assert_eq!(result, Value::Int64(24));
}

#[test]
fn test_list_any_value() {
    let func = ScalarFunction::List { op: ListOp::AnyValue };
    let result = evaluate_scalar(&func, &[Value::List(vec![Value::Int64(42), Value::Int64(100)])]).unwrap();
    assert_eq!(result, Value::Int64(42));
}

#[test]
fn test_list_to_string() {
    let func = ScalarFunction::List { op: ListOp::ToString };
    let result = evaluate_scalar(
        &func,
        &[
            Value::String(",".into()),
            Value::List(vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)]),
        ],
    )
    .unwrap();
    assert_eq!(result, Value::String("Int64(1),Int64(2),Int64(3)".into()));
}

#[test]
fn test_list_position() {
    let func = ScalarFunction::List { op: ListOp::Position };
    let list = Value::List(vec![
        Value::String("a".into()),
        Value::String("b".into()),
        Value::String("c".into()),
    ]);
    // Found → 1-based index
    let result = evaluate_scalar(&func, &[list.clone(), Value::String("b".into())]).unwrap();
    assert_eq!(result, Value::Int64(2));
    // Not found → 0
    let result = evaluate_scalar(&func, &[list.clone(), Value::String("z".into())]).unwrap();
    assert_eq!(result, Value::Int64(0));
}

#[test]
fn test_list_has_all() {
    let func = ScalarFunction::List { op: ListOp::HasAll };
    let left = Value::List(vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)]);
    let right_yes = Value::List(vec![Value::Int64(1), Value::Int64(3)]);
    let right_no = Value::List(vec![Value::Int64(1), Value::Int64(99)]);
    assert_eq!(
        evaluate_scalar(&func, &[left.clone(), right_yes]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(evaluate_scalar(&func, &[left, right_no]).unwrap(), Value::Bool(false));
}

#[test]
fn test_list_reverse_sort() {
    let func = ScalarFunction::List {
        op: ListOp::ReverseSort,
    };
    let result = evaluate_scalar(
        &func,
        &[Value::List(vec![Value::Int64(3), Value::Int64(1), Value::Int64(2)])],
    )
    .unwrap();
    assert_eq!(
        result,
        Value::List(vec![Value::Int64(3), Value::Int64(2), Value::Int64(1),])
    );
}

// --- List predicate function tests ---

#[test]
fn test_list_any() {
    let func = ScalarFunction::List { op: ListOp::Any };
    assert_eq!(
        evaluate_scalar(&func, &[Value::List(vec![Value::Bool(false), Value::Bool(true),])]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::List(vec![Value::Bool(false), Value::Bool(false),])]).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn test_list_all() {
    let func = ScalarFunction::List { op: ListOp::All };
    assert_eq!(
        evaluate_scalar(&func, &[Value::List(vec![Value::Bool(true), Value::Int64(1),])]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::List(vec![Value::Bool(true), Value::Int64(0),])]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::List(vec![])]).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn test_list_none() {
    let func = ScalarFunction::List { op: ListOp::None };
    assert_eq!(
        evaluate_scalar(&func, &[Value::List(vec![Value::Bool(false), Value::Int64(0),])]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::List(vec![Value::Bool(false), Value::Int64(1),])]).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn test_list_single() {
    let func = ScalarFunction::List { op: ListOp::Single };
    assert_eq!(
        evaluate_scalar(
            &func,
            &[Value::List(vec![
                Value::Bool(false),
                Value::Bool(true),
                Value::Int64(0),
            ])]
        )
        .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::List(vec![Value::Bool(false), Value::Int64(0),])]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate_scalar(&func, &[Value::List(vec![Value::Bool(true), Value::Int64(1),])]).unwrap(),
        Value::Bool(false)
    );
}
