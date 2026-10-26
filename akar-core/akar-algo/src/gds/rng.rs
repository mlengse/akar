//! Deterministic LCG for the random-walk algorithms (seeded, no extra deps).

pub(super) struct SimpleRng {
    state: u64,
    last_bound: usize,
    last_limit: u64,
}

impl SimpleRng {
    pub(super) fn new(seed: u64) -> Self {
        Self {
            state: seed,
            last_bound: 0,
            last_limit: 0,
        }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.state >> 32) as u32
    }

    /// Uniform index in `0..bound` — rejection sampling removes the modulo bias
    /// of the naive `next_u32() % bound`.
    pub(super) fn gen_range(&mut self, bound: usize) -> usize {
        assert!(bound > 0, "gen_range bound must be > 0");
        let bound_u32 = u32::try_from(bound).unwrap_or(u32::MAX);
        if bound_u32 == 1 {
            return 0;
        }
        // Bolt ⚡ Performance Optimization:
        // Caching `last_bound` and `last_limit` avoids repeating 64-bit division/modulo (`n % bound`)
        // during millions of sequential calls with identical bounds (e.g., negative sampling or walk steps).
        let limit = if bound == self.last_bound {
            self.last_limit
        } else {
            let n = 1_u64 << 32;
            let l = n - (n % u64::from(bound_u32));
            self.last_bound = bound;
            self.last_limit = l;
            l
        };
        loop {
            let x = u64::from(self.next_u32());
            if x < limit {
                return (x % u64::from(bound_u32)) as usize;
            }
        }
    }

    /// Uniform float in `[0.0, 1.0]`.
    pub(super) fn gen_float(&mut self) -> f64 {
        (self.next_u32() as f64) / (u32::MAX as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_range_stays_in_bounds_and_is_deterministic() {
        let mut a = SimpleRng::new(42);
        let mut b = SimpleRng::new(42);
        for bound in [1usize, 2, 3, 7, 16, 100] {
            assert_eq!(a.gen_range(1), 0);
            for _ in 0..10_000 {
                let x = a.gen_range(bound);
                assert!(x < bound);
                assert_eq!(x, b.gen_range(bound));
            }
        }
    }

    #[test]
    fn gen_range_unbiased_for_non_power_of_two() {
        let mut rng = SimpleRng::new(7);
        let bound = 10usize;
        let draws = 1_000_000usize;
        let mut counts = vec![0usize; bound];
        for _ in 0..draws {
            counts[rng.gen_range(bound)] += 1;
        }
        let min = *counts.iter().min().unwrap();
        let max = *counts.iter().max().unwrap();
        // Uniform buckets deviate by sampling noise only; the biased
        // `x % 10` variant would skew two buckets by ~7% relative.
        assert!(max - min < draws / 250, "bucket spread too wide: {min}..{max}");
    }
}
