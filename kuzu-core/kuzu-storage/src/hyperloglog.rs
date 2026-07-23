//! HyperLogLog cardinality estimation.
//!
//! Implements the HyperLogLog algorithm as described in:
//!   Flajolet et al., "HyperLogLog: the analysis of a near-optimal
//!   cardinality estimation algorithm", 2007.
//!
//! Uses precision P=6 (M=64 registers) with 8-bit registers,
//! suitable for estimating up to ~2^32 distinct values.
//! This matches LadybugDB's configuration.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Precision parameter — determines the number of registers (M = 2^P).
const P: u32 = 6;
/// Number of registers = 64.
const M: usize = 1 << P;
/// Q = 64 - P = 58 bits used for the hash prefix.
const Q: u32 = 64 - P;
/// Bias correction constant: α = 1 / (M * ∫₀^∞ (log₂((2+u)/(1+u)))^M du) ≈ 0.709
/// The standard approximation for M=64 is m * α_m where α_64 ≈ 0.709.
const ALPHA_MM: f64 = 0.709;

/// Linear counting threshold — use LC when raw estimate < threshold.
const LINEAR_COUNTING_THRESHOLD: f64 = M as f64 * 2.5;

/// HyperLogLog cardinality estimator.
#[derive(Debug, Clone)]
pub struct HyperLogLog {
    /// Register array — each entry is the maximum number of leading zeros + 1 seen.
    registers: [u8; M],
}

impl Default for HyperLogLog {
    fn default() -> Self {
        Self::new()
    }
}

impl HyperLogLog {
    pub fn new() -> Self {
        Self {
            registers: [0u8; M],
        }
    }

    /// Hash a value and update the HLL registers.
    pub fn insert<T: Hash>(&mut self, value: &T) {
        let hash = Self::hash_value(value);
        let idx = (hash & ((1 << P) - 1)) as usize; // lower P bits = register index
        let w = (hash >> P) | (1u64 << Q); // upper Q bits with implicit leading 1
        let leading_zeros = (w.trailing_zeros() + 1) as u8; // ρ(w) = position of leftmost 1
        if leading_zeros > self.registers[idx] {
            self.registers[idx] = leading_zeros;
        }
    }

    /// Insert a pre-computed hash value directly.
    pub fn insert_hash(&mut self, hash: u64) {
        let idx = (hash & ((1 << P) - 1)) as usize;
        let w = (hash >> P) | (1u64 << Q);
        let leading_zeros = (w.trailing_zeros() + 1) as u8;
        if leading_zeros > self.registers[idx] {
            self.registers[idx] = leading_zeros;
        }
    }

    /// Merge another HLL into this one (union operation).
    pub fn merge(&mut self, other: &HyperLogLog) {
        for i in 0..M {
            if other.registers[i] > self.registers[i] {
                self.registers[i] = other.registers[i];
            }
        }
    }

    /// Estimate the cardinality (number of distinct values seen).
    pub fn count(&self) -> u64 {
        // Raw HyperLogLog estimate
        let z_inv: f64 = self.registers.iter()
            .map(|&r| 2.0f64.powi(-(r as i32)))
            .sum();
        let raw_estimate = ALPHA_MM * (M as f64).powi(2) / z_inv;

        // Small range correction: use Linear Counting if estimate is small
        if raw_estimate <= LINEAR_COUNTING_THRESHOLD {
            let zero_regs = self.registers.iter().filter(|&&r| r == 0).count() as f64;
            if zero_regs > 0.0 {
                return (M as f64 * (M as f64 / zero_regs).ln()).round() as u64;
            }
        }

        // Large range correction: no correction needed for M=64 (simple HLL)
        raw_estimate.round() as u64
    }

    /// Number of non-zero registers.
    pub fn non_zero_registers(&self) -> usize {
        self.registers.iter().filter(|&&r| r > 0).count()
    }

    /// Reset all registers to zero.
    pub fn clear(&mut self) {
        self.registers = [0u8; M];
    }

    fn hash_value<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_hll() {
        let hll = HyperLogLog::new();
        assert_eq!(hll.count(), 0);
    }

    #[test]
    fn test_single_element() {
        let mut hll = HyperLogLog::new();
        hll.insert(&42i64);
        let count = hll.count();
        assert!((1..=5).contains(&count), "Expected ~1, got {}", count);
    }

    #[test]
    fn test_small_set() {
        let mut hll = HyperLogLog::new();
        for i in 0..100i64 {
            hll.insert(&i);
        }
        let count = hll.count();
        let error = ((count as f64 - 100.0) / 100.0).abs();
        assert!(error < 0.30, "Expected ~100, got {} (error: {:.1}%)", count, error * 100.0);
    }

    #[test]
    fn test_large_set() {
        let mut hll = HyperLogLog::new();
        for i in 0..10_000i64 {
            hll.insert(&i);
        }
        let count = hll.count();
        let error = ((count as f64 - 10_000.0) / 10_000.0).abs();
        assert!(error < 0.15, "Expected ~10000, got {} (error: {:.1}%)", count, error * 100.0);
    }

    #[test]
    fn test_merge() {
        let mut hll1 = HyperLogLog::new();
        let mut hll2 = HyperLogLog::new();
        for i in 0..500i64 {
            hll1.insert(&i);
        }
        for i in 250..750i64 {
            hll2.insert(&i);
        }
        hll1.merge(&hll2);
        let count = hll1.count();
        let error = ((count as f64 - 750.0) / 750.0).abs();
        assert!(error < 0.35, "Expected ~750, got {} (error: {:.1}%)", count, error * 100.0);
    }

    #[test]
    fn test_clear() {
        let mut hll = HyperLogLog::new();
        for i in 0..1_000i64 {
            hll.insert(&i);
        }
        assert!(hll.count() > 0);
        hll.clear();
        assert_eq!(hll.count(), 0);
    }
}
