//! Shared utility functions used across scalar function categories.

use super::{REGEX_CACHE, RNG_STATE};

/// Get a compiled regex from the cache, or compile and cache it.
pub(crate) fn get_cached_regex(pattern: &str) -> Result<regex::Regex, String> {
    let mut cache = REGEX_CACHE.lock().map_err(|e| format!("Regex cache lock error: {e}"))?;
    if let Some(re) = cache.get(pattern) {
        return Ok(re.clone());
    }
    let re = regex::Regex::new(pattern).map_err(|e| format!("Regex error: {e}"))?;
    cache.insert(pattern.to_string(), re.clone());
    Ok(re)
}

/// Get next random f64 in [0, 1) from the thread-local LCG.
pub(crate) fn rng_next() -> f64 {
    RNG_STATE.with(|state| {
        let old = state.get();
        let new = old.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        state.set(new);
        (new >> 11) as f64 / (1u64 << 53) as f64
    })
}

/// Log-Gamma function using Lanczos approximation.
pub(crate) fn log_gamma(x: f64) -> f64 {
    if x < 0.5 {
        let pi = std::f64::consts::PI;
        let reflection = pi / (pi * x).sin();
        reflection.abs().ln() - log_gamma(1.0 - x)
    } else {
        let xm1 = x - 1.0;
        let g = 7.0;
        let c = [
            0.999_999_999_999_809_9,
            676.5203681218851,
            -1259.1392167224028,
            771.323_428_777_653_1,
            -176.615_029_162_140_6,
            12.507343278686905,
            -0.13857109526572012,
            9.984_369_578_019_572e-6,
            1.5056327351493116e-7,
        ];
        let t = xm1 + g + 0.5;
        let mut s = c[0];
        for (i, &ci) in c[1..].iter().enumerate() {
            s += ci / (xm1 + (i as f64) + 1.0);
        }
        let sqrt_2pi = (2.0 * std::f64::consts::PI).sqrt();
        (sqrt_2pi * s).ln() + (xm1 + 0.5) * t.ln() - t
    }
}

/// Lanczos approximation for Gamma(x) — computed via exp(log_gamma(x)).
pub(crate) fn gamma_func(x: f64) -> f64 {
    log_gamma(x).exp()
}

/// Set the thread-local RNG seed.
pub fn set_rng_seed(seed: u64) {
    RNG_STATE.with(|state| state.set(seed));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_rng_seed_determinism() {
        set_rng_seed(12345);
        let val1 = rng_next();
        let val2 = rng_next();

        set_rng_seed(12345);
        let val1_again = rng_next();
        let val2_again = rng_next();

        assert_eq!(val1, val1_again);
        assert_eq!(val2, val2_again);
    }

    #[test]
    fn test_set_rng_seed_different_seeds() {
        set_rng_seed(12345);
        let val1 = rng_next();

        set_rng_seed(54321);
        let val2 = rng_next();

        assert_ne!(val1, val2);
    }

    #[test]
    fn test_set_rng_seed_boundary_values() {
        set_rng_seed(0);
        let val_zero = rng_next();
        assert!((0.0..1.0).contains(&val_zero));

        set_rng_seed(0);
        assert_eq!(rng_next(), val_zero);

        set_rng_seed(u64::MAX);
        let val_max = rng_next();
        assert!((0.0..1.0).contains(&val_max));

        set_rng_seed(u64::MAX);
        assert_eq!(rng_next(), val_max);
    }

    #[test]
    fn test_get_cached_regex() {
        let re1 = get_cached_regex(r"\d+").unwrap();
        let re2 = get_cached_regex(r"\d+").unwrap();
        assert!(re1.is_match("123"));
        assert!(re2.is_match("123"));

        let err = get_cached_regex(r"[").unwrap_err();
        assert!(err.contains("Regex error"));
    }
}
