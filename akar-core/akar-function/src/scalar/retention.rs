//! Ebbinghaus-style retention scoring — the cognitive decay primitive (P119).
//!
//! `retention_score` answers "how much of this memory is still there?", as a
//! number in `(0, 1]`, from the observables an agent memory actually carries:
//! age, access history, salience, and how many distinct actors have used it.
//!
//! # Formula
//!
//! ```text
//! stability_days = BASE
//!                * (1 + ACCESS_BOOST      * ln(1 + access_count))
//!                * (1 + AGE_CONSOLIDATION * ln(1 + age_days))
//!                * (SALIENCE_FLOOR + (1 - SALIENCE_FLOOR) * salience)
//!                * (1 + breadth_weight    * ln(1 + distinct_actors))
//!
//! retention = exp(-days_since_access / stability_days)
//! ```
//!
//! This is Ebbinghaus forgetting (`R = e^{-t/S}`) with a **dynamic** stability
//! `S`: every factor is non-decreasing in its input, so the curve flattens —
//! a memory that has been recalled often, has been alive a long time, matters
//! (high salience), or is shared across many actors decays more slowly.
//!
//! Every factor is 1.0 at its neutral value and above 1.0 only for observables
//! that genuinely extend stability, so the whole formula degenerates to plain
//! one-day Ebbinghaus (`e^{-t}`) when a memory is maximally salient and nothing
//! else is known. Salience scales between `SALIENCE_FLOOR` (salience 0) and 1.0
//! (salience 1) rather than adding on top of it, so no single observable can
//! silently inflate stability by an arbitrary factor.
//!
//! # Why `age_days` *and* `days_since_access`
//!
//! They are different signals and both are needed. `days_since_access` is the
//! forgetting interval `t` — the only quantity in the exponent.
//! `age_days` is consolidation age: a memory that has survived a year of
//! graph churn is better established than one created yesterday, even if
//! neither has been touched since creation. Feeding `age_days` into the
//! exponent instead would conflate "old" with "forgotten", which is exactly
//! the confusion that makes flat linear decay wrong.

use crate::registry::RetentionOp;
use akar_common::types::Value;

/// Stability (in days) of a memory with no accesses, no age, and no salience.
/// One day reproduces textbook one-day Ebbinghaus forgetting.
const BASE_STABILITY_DAYS: f64 = 1.0;

/// How much each recall extends stability, on a log scale. `ln(1 + n)`
/// saturates, so the 100th recall matters far less than the 1st — matching
/// spaced-repetition behaviour rather than letting a hot loop dominate.
const ACCESS_BOOST: f64 = 0.5;

/// How much consolidation age extends stability, on the same log scale.
const AGE_CONSOLIDATION: f64 = 0.1;

/// Stability multiplier at zero salience. Salience scales linearly from this
/// floor up to 1.0, so a maximally salient memory gets no extra stability from
/// salience and an unsalient one keeps half — never less than half, because
/// salience is a weighting signal, not evidence of forgetting.
const SALIENCE_FLOOR: f64 = 0.5;

/// Default breadth weight when the caller omits it.
const DEFAULT_BREADTH_WEIGHT: f64 = 1.0;

/// Evaluate a retention-score function.
///
/// Accepts 5 or 6 arguments:
/// `(age_days, access_count, days_since_access, salience, distinct_actors[, breadth_weight])`.
///
/// # Errors
/// Returns `Err` for the wrong number of arguments or a non-numeric argument
/// that is not `NULL`. `NULL` is not an error: it contributes 0 to its own
/// factor (and, for `days_since_access`, means "accessed just now"), because
/// callers score rows where these fields are simply absent.
pub fn evaluate_retention(op: RetentionOp, args: &[Value]) -> Result<Value, String> {
    match op {
        RetentionOp::Score => {
            if args.len() < 5 || args.len() > 6 {
                return Err(format!(
                    "retention_score expects 5 or 6 arguments \
                     (age_days, access_count, days_since_access, salience, distinct_actors[, breadth_weight]), got {}",
                    args.len()
                ));
            }
            let age_days = number_arg(&args[0], "age_days")?.max(0.0);
            let access_count = number_arg(&args[1], "access_count")?.max(0.0);
            let days_since_access = number_arg(&args[2], "days_since_access")?.max(0.0);
            let salience = number_arg(&args[3], "salience")?.clamp(0.0, 1.0);
            let distinct_actors = number_arg(&args[4], "distinct_actors")?.max(0.0);
            let breadth_weight = match args.get(5) {
                Some(v) => number_arg(v, "breadth_weight")?.max(0.0),
                None => DEFAULT_BREADTH_WEIGHT,
            };

            Ok(Value::Double(retention_score(
                age_days,
                access_count,
                days_since_access,
                salience,
                distinct_actors,
                breadth_weight,
            )))
        }
    }
}

/// The retention formula itself, as a plain function.
///
/// Exposed separately from argument decoding so the dream engine (P119.2) and
/// other in-process consumers score memories directly, without building
/// `Value`s per row. All inputs are taken as already clamped to their valid
/// ranges; the result is always in `(0, 1]`.
pub fn retention_score(
    age_days: f64,
    access_count: f64,
    days_since_access: f64,
    salience: f64,
    distinct_actors: f64,
    breadth_weight: f64,
) -> f64 {
    let stability = BASE_STABILITY_DAYS
        * (1.0 + ACCESS_BOOST * (1.0 + access_count).ln())
        * (1.0 + AGE_CONSOLIDATION * (1.0 + age_days).ln())
        * (SALIENCE_FLOOR + (1.0 - SALIENCE_FLOOR) * salience.clamp(0.0, 1.0))
        * (1.0 + breadth_weight * (1.0 + distinct_actors).ln());

    if !stability.is_finite() || stability <= 0.0 {
        // Degenerate inputs (NaN/inf from a non-finite caller value) must not
        // leak a NaN score into ranking; treat them as fully forgotten.
        return 0.0;
    }

    (-days_since_access.max(0.0) / stability).exp().clamp(0.0, 1.0)
}

/// Decode one argument as `f64`, treating `NULL` as 0.
fn number_arg(value: &Value, name: &str) -> Result<f64, String> {
    match value {
        Value::Null => Ok(0.0),
        Value::Int64(v) => Ok(*v as f64),
        Value::Int32(v) => Ok(f64::from(*v)),
        Value::Int16(v) => Ok(f64::from(*v)),
        Value::Int8(v) => Ok(f64::from(*v)),
        Value::UInt64(v) => Ok(*v as f64),
        Value::UInt32(v) => Ok(f64::from(*v)),
        Value::UInt16(v) => Ok(f64::from(*v)),
        Value::UInt8(v) => Ok(f64::from(*v)),
        Value::Double(v) => Ok(*v),
        Value::Float(v) => Ok(f64::from(*v)),
        other => Err(format!(
            "retention_score: argument '{name}' must be numeric, got {other:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(args: &[Value]) -> f64 {
        match evaluate_retention(RetentionOp::Score, args).unwrap() {
            Value::Double(v) => v,
            other => panic!("expected Double, got {other:?}"),
        }
    }

    fn baseline() -> Vec<Value> {
        vec![
            Value::Int64(30),   // age_days
            Value::Int64(3),    // access_count
            Value::Int64(2),    // days_since_access
            Value::Double(0.5), // salience
            Value::Int64(1),    // distinct_actors
        ]
    }

    #[test]
    fn fresh_access_retains_everything() {
        let mut args = baseline();
        args[2] = Value::Int64(0);
        assert!(
            (score(&args) - 1.0).abs() < 1e-12,
            "a just-recalled memory must score 1.0"
        );
    }

    #[test]
    fn retention_decays_with_days_since_access() {
        let mut previous = f64::INFINITY;
        for days in [0, 1, 2, 5, 10, 30, 100] {
            let mut args = baseline();
            args[2] = Value::Int64(days);
            let s = score(&args);
            assert!(s < previous, "retention must strictly decrease with age of access");
            assert!((0.0..=1.0).contains(&s), "retention out of range: {s}");
            previous = s;
        }
    }

    #[test]
    fn more_accesses_slow_the_decay() {
        let mut low = baseline();
        low[1] = Value::Int64(1);
        let mut high = baseline();
        high[1] = Value::Int64(50);
        assert!(score(&high) > score(&low), "recall history must extend retention");
    }

    #[test]
    fn older_memories_are_better_consolidated() {
        let mut young = baseline();
        young[0] = Value::Int64(0);
        let mut old = baseline();
        old[0] = Value::Int64(365);
        assert!(score(&old) > score(&young), "consolidation age must extend retention");
    }

    #[test]
    fn salience_and_actor_breadth_extend_retention() {
        let mut low_sal = baseline();
        low_sal[3] = Value::Double(0.0);
        let mut high_sal = baseline();
        high_sal[3] = Value::Double(1.0);
        assert!(score(&high_sal) > score(&low_sal), "salience must extend retention");

        let mut narrow = baseline();
        narrow[4] = Value::Int64(1);
        let mut broad = baseline();
        broad[4] = Value::Int64(25);
        assert!(score(&broad) > score(&narrow), "actor breadth must extend retention");
    }

    #[test]
    fn breadth_weight_scales_the_actor_term() {
        let mut args = baseline();
        args[4] = Value::Int64(10);
        let without = score(&args);
        args.push(Value::Double(5.0));
        let with_weight = score(&args);
        assert!(
            with_weight > without,
            "an explicit breadth weight must strengthen the actor term"
        );

        // A zero breadth weight removes the factor entirely.
        let mut zero = baseline();
        zero[4] = Value::Int64(10);
        zero.push(Value::Double(0.0));
        let mut no_actors = baseline();
        no_actors[4] = Value::Int64(0);
        assert!((score(&zero) - score(&no_actors)).abs() < 1e-12);
    }

    #[test]
    fn nulls_are_treated_as_absent_not_as_errors() {
        let args = vec![Value::Null, Value::Null, Value::Null, Value::Null, Value::Null];
        let s = score(&args);
        assert!((0.0..=1.0).contains(&s), "null-only row must still score: {s}");
        // days_since_access NULL → "accessed now" → full retention.
        assert!((s - 1.0).abs() < 1e-12);
    }

    #[test]
    fn wrong_arity_is_rejected() {
        let four = vec![Value::Int64(1); 4];
        let err = evaluate_retention(RetentionOp::Score, &four).unwrap_err();
        assert!(err.contains("5 or 6 arguments"), "unhelpful error: {err}");

        let seven = vec![Value::Int64(1); 7];
        assert!(evaluate_retention(RetentionOp::Score, &seven).is_err());
    }

    #[test]
    fn non_numeric_arguments_are_rejected() {
        let mut args = baseline();
        args[0] = Value::String("yesterday".into());
        let err = evaluate_retention(RetentionOp::Score, &args).unwrap_err();
        assert!(err.contains("age_days"), "error should name the argument: {err}");
    }

    #[test]
    fn negative_inputs_are_clamped_not_propagated() {
        let args = vec![
            Value::Int64(-5),
            Value::Int64(-2),
            Value::Int64(-1),
            Value::Double(-0.5),
            Value::Int64(-3),
        ];
        let s = score(&args);
        assert!((0.0..=1.0).contains(&s), "clamping must keep the score in range: {s}");
        // Everything clamped to the floor → plain one-day Ebbinghaus at t=0.
        assert!((s - 1.0).abs() < 1e-12);
    }

    #[test]
    fn salience_above_one_is_clamped() {
        let mut high = baseline();
        high[3] = Value::Double(50.0);
        let mut at_one = baseline();
        at_one[3] = Value::Double(1.0);
        assert!((score(&high) - score(&at_one)).abs() < 1e-12);
    }

    #[test]
    fn pure_formula_matches_the_documented_shape() {
        let t = 3.0;
        // Maximally salient with no history: every factor is neutral, so the
        // curve is plain one-day Ebbinghaus — the anchor the formula is built on.
        let expected = (-t / BASE_STABILITY_DAYS).exp();
        assert!((retention_score(0.0, 0.0, t, 1.0, 0.0, 0.0) - expected).abs() < 1e-12);

        // Zero salience halves stability, never more than that.
        let expected_unsalient = (-t / (BASE_STABILITY_DAYS * SALIENCE_FLOOR)).exp();
        assert!((retention_score(0.0, 0.0, t, 0.0, 0.0, 0.0) - expected_unsalient).abs() < 1e-12);
    }

    #[test]
    fn salience_interpolates_between_the_floor_and_neutral() {
        let t = 10.0;
        let at_floor = retention_score(0.0, 0.0, t, 0.0, 0.0, 0.0);
        let mid = retention_score(0.0, 0.0, t, 0.5, 0.0, 0.0);
        let neutral = retention_score(0.0, 0.0, t, 1.0, 0.0, 0.0);
        assert!(at_floor < mid && mid < neutral, "salience must be monotone");
        assert!(
            (neutral - (-t / BASE_STABILITY_DAYS).exp()).abs() < 1e-12,
            "max salience must be neutral, not a bonus"
        );
    }

    #[test]
    fn non_finite_input_does_not_produce_nan() {
        let s = retention_score(f64::NAN, 0.0, 1.0, 0.0, 0.0, 0.0);
        assert!(!s.is_nan(), "a NaN score must never escape");
    }
}
