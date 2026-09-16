//! Vectorized distance kernels for Akar.
//!
//! Runtime-dispatched SIMD implementations (SSE2 / AVX on x86_64, NEON on
//! aarch64) with a portable scalar fallback. Every public entry point
//! validates operand shape and selects the fastest kernel safe for the
//! current CPU via [`std::arch::is_x86_feature_detected`]. Windows and other
//! targets degrade to the scalar path automatically — no build-time flags.
//!
//! # Thresholds
//!
//! - [`MIN_DIM_SIMD`] — minimum dimension for the SSE2 / NEON (128-bit) kernels.
//! - [`MIN_DIM_AVX`] — minimum dimension for the AVX (256-bit) kernel.
//!
//! Below the SIMD threshold the scalar kernel is used, avoiding loop/branch
//! setup overhead on tiny vectors.

/// Minimum vector dimension for SSE2 / NEON (128-bit) kernels.
pub const MIN_DIM_SIMD: usize = 16;

/// Minimum vector dimension for the AVX (256-bit) kernel.
pub const MIN_DIM_AVX: usize = 32;

// ---------------------------------------------------------------------------
// Scalar kernels (portable baseline)
// ---------------------------------------------------------------------------

/// Single-pass dot product and squared norms.
///
/// Returns `(dot, sq_norm_a, sq_norm_b)`. Operands with unequal length yield
/// `(0.0, 0.0, 0.0)`.
#[inline]
pub fn dot_and_sq_norms_scalar(a: &[f64], b: &[f64]) -> (f64, f64, f64) {
    let mut dot = 0.0;
    let mut sq_a = 0.0;
    let mut sq_b = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        sq_a += x * x;
        sq_b += y * y;
    }
    (dot, sq_a, sq_b)
}

/// Squared L2 distance — sum of squared elementwise differences.
#[inline]
pub fn l2_squared_scalar(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum()
}

/// L1 distance — sum of absolute elementwise differences.
#[inline]
pub fn l1_distance_scalar(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| (x - y).abs()).sum()
}

// ---------------------------------------------------------------------------
// x86_64 kernels (SSE2 + AVX, runtime-dispatched)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
mod sse2 {
    use std::arch::x86_64::*;

    /// Horizontal sum of a 2-lane PD vector.
    #[inline]
    pub(crate) unsafe fn hsum_pd(v: __m128d) -> f64 {
        unsafe {
            let hi = _mm_unpackhi_pd(v, v);
            _mm_cvtsd_f64(_mm_add_sd(v, hi))
        }
    }

    /// SSE2 single-pass dot + squared norms.
    ///
    /// # Safety
    ///
    /// The CPU must support SSE2 and `a`/`b` must have equal, non-empty
    /// length.
    #[target_feature(enable = "sse2")]
    pub(crate) unsafe fn dot_and_sq_norms(a: &[f64], b: &[f64]) -> (f64, f64, f64) {
        let chunks = a.len() / 2;
        let mut dot = _mm_setzero_pd();
        let mut sq_a = _mm_setzero_pd();
        let mut sq_b = _mm_setzero_pd();
        for i in 0..chunks {
            let x = unsafe { _mm_loadu_pd(a.as_ptr().add(i * 2)) };
            let y = unsafe { _mm_loadu_pd(b.as_ptr().add(i * 2)) };
            dot = _mm_add_pd(dot, _mm_mul_pd(x, y));
            sq_a = _mm_add_pd(sq_a, _mm_mul_pd(x, x));
            sq_b = _mm_add_pd(sq_b, _mm_mul_pd(y, y));
        }
        let mut acc = (unsafe { hsum_pd(dot) }, unsafe { hsum_pd(sq_a) }, unsafe {
            hsum_pd(sq_b)
        });
        for (x, y) in a[chunks * 2..].iter().zip(b[chunks * 2..].iter()) {
            acc.0 += x * y;
            acc.1 += x * x;
            acc.2 += y * y;
        }
        acc
    }

    /// SSE2 squared L2 distance.
    ///
    /// # Safety
    ///
    /// The CPU must support SSE2 and `a`/`b` must have equal, non-empty
    /// length.
    #[target_feature(enable = "sse2")]
    pub(crate) unsafe fn l2_squared(a: &[f64], b: &[f64]) -> f64 {
        let chunks = a.len() / 2;
        let mut acc = _mm_setzero_pd();
        for i in 0..chunks {
            let x = unsafe { _mm_loadu_pd(a.as_ptr().add(i * 2)) };
            let y = unsafe { _mm_loadu_pd(b.as_ptr().add(i * 2)) };
            let d = _mm_sub_pd(x, y);
            acc = _mm_add_pd(acc, _mm_mul_pd(d, d));
        }
        let mut sum = unsafe { hsum_pd(acc) };
        for (x, y) in a[chunks * 2..].iter().zip(b[chunks * 2..].iter()) {
            let d = x - y;
            sum += d * d;
        }
        sum
    }

    /// SSE2 L1 distance.
    ///
    /// # Safety
    ///
    /// The CPU must support SSE2 and `a`/`b` must have equal, non-empty
    /// length.
    #[target_feature(enable = "sse2")]
    pub(crate) unsafe fn l1_distance(a: &[f64], b: &[f64]) -> f64 {
        let chunks = a.len() / 2;
        let mut acc = _mm_setzero_pd();
        let sign = _mm_set1_pd(-0.0);
        for i in 0..chunks {
            let x = unsafe { _mm_loadu_pd(a.as_ptr().add(i * 2)) };
            let y = unsafe { _mm_loadu_pd(b.as_ptr().add(i * 2)) };
            let d = _mm_sub_pd(x, y);
            acc = _mm_add_pd(acc, _mm_andnot_pd(sign, d));
        }
        let mut sum = unsafe { hsum_pd(acc) };
        for (x, y) in a[chunks * 2..].iter().zip(b[chunks * 2..].iter()) {
            sum += (x - y).abs();
        }
        sum
    }
}

#[cfg(target_arch = "x86_64")]
mod avx {
    use super::*;
    use std::arch::x86_64::*;

    /// Horizontal sum of a 4-lane PD vector: reduce to 128-bit then use the
    /// SSE2 horizontal sum.
    #[inline]
    pub(crate) unsafe fn hsum256_pd(v: __m256d) -> f64 {
        unsafe {
            let lo = _mm256_castpd256_pd128(v);
            let hi = _mm256_extractf128_pd(v, 1);
            sse2::hsum_pd(_mm_add_pd(lo, hi))
        }
    }

    /// AVX single-pass dot + squared norms. Scalar path handles the ≤3-lane
    /// tail.
    ///
    /// # Safety
    ///
    /// The CPU must support AVX and `a`/`b` must have equal, non-empty
    /// length.
    #[target_feature(enable = "avx")]
    pub(crate) unsafe fn dot_and_sq_norms(a: &[f64], b: &[f64]) -> (f64, f64, f64) {
        let chunks = a.len() / 4;
        let mut dot = _mm256_setzero_pd();
        let mut sq_a = _mm256_setzero_pd();
        let mut sq_b = _mm256_setzero_pd();
        for i in 0..chunks {
            let x = unsafe { _mm256_loadu_pd(a.as_ptr().add(i * 4)) };
            let y = unsafe { _mm256_loadu_pd(b.as_ptr().add(i * 4)) };
            dot = _mm256_add_pd(dot, _mm256_mul_pd(x, y));
            sq_a = _mm256_add_pd(sq_a, _mm256_mul_pd(x, x));
            sq_b = _mm256_add_pd(sq_b, _mm256_mul_pd(y, y));
        }
        let mut acc = (unsafe { hsum256_pd(dot) }, unsafe { hsum256_pd(sq_a) }, unsafe {
            hsum256_pd(sq_b)
        });
        for (x, y) in a[chunks * 4..].iter().zip(b[chunks * 4..].iter()) {
            acc.0 += x * y;
            acc.1 += x * x;
            acc.2 += y * y;
        }
        acc
    }

    /// AVX squared L2 distance.
    ///
    /// # Safety
    ///
    /// The CPU must support AVX and `a`/`b` must have equal, non-empty
    /// length.
    #[target_feature(enable = "avx")]
    pub(crate) unsafe fn l2_squared(a: &[f64], b: &[f64]) -> f64 {
        let chunks = a.len() / 4;
        let mut acc = _mm256_setzero_pd();
        for i in 0..chunks {
            let x = unsafe { _mm256_loadu_pd(a.as_ptr().add(i * 4)) };
            let y = unsafe { _mm256_loadu_pd(b.as_ptr().add(i * 4)) };
            let d = _mm256_sub_pd(x, y);
            acc = _mm256_add_pd(acc, _mm256_mul_pd(d, d));
        }
        let mut sum = unsafe { hsum256_pd(acc) };
        for (x, y) in a[chunks * 4..].iter().zip(b[chunks * 4..].iter()) {
            let d = x - y;
            sum += d * d;
        }
        sum
    }

    /// AVX L1 distance (absolute differences via sign-masking).
    ///
    /// # Safety
    ///
    /// The CPU must support AVX and `a`/`b` must have equal, non-empty
    /// length.
    #[target_feature(enable = "avx")]
    pub(crate) unsafe fn l1_distance(a: &[f64], b: &[f64]) -> f64 {
        let chunks = a.len() / 4;
        let mut acc = _mm256_setzero_pd();
        let sign = _mm256_set1_pd(-0.0);
        for i in 0..chunks {
            let x = unsafe { _mm256_loadu_pd(a.as_ptr().add(i * 4)) };
            let y = unsafe { _mm256_loadu_pd(b.as_ptr().add(i * 4)) };
            let d = _mm256_sub_pd(x, y);
            acc = _mm256_add_pd(acc, _mm256_andnot_pd(sign, d));
        }
        let mut sum = unsafe { hsum256_pd(acc) };
        for (x, y) in a[chunks * 4..].iter().zip(b[chunks * 4..].iter()) {
            sum += (x - y).abs();
        }
        sum
    }
}

// ---------------------------------------------------------------------------
// aarch64 kernels (NEON; baseline on armv8-A)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
mod neon {
    use super::*;
    use std::arch::aarch64::*;

    /// NEON single-pass dot + squared norms (2 × f64 per vector).
    pub(crate) fn dot_and_sq_norms(a: &[f64], b: &[f64]) -> (f64, f64, f64) {
        let chunks = a.len() / 2;
        let mut dot = vdupq_n_f64(0.0);
        let mut sq_a = vdupq_n_f64(0.0);
        let mut sq_b = vdupq_n_f64(0.0);
        for i in 0..chunks {
            let x = unsafe { vld1q_f64(a.as_ptr().add(i * 2)) };
            let y = unsafe { vld1q_f64(b.as_ptr().add(i * 2)) };
            dot = vaddq_f64(dot, vmulq_f64(x, y));
            sq_a = vaddq_f64(sq_a, vmulq_f64(x, x));
            sq_b = vaddq_f64(sq_b, vmulq_f64(y, y));
        }
        let mut acc = (vaddvq_f64(dot), vaddvq_f64(sq_a), vaddvq_f64(sq_b));
        for (x, y) in a[chunks * 2..].iter().zip(&b[chunks * 2..]) {
            acc.0 += x * y;
            acc.1 += x * x;
            acc.2 += y * y;
        }
        acc
    }

    /// NEON squared L2 distance.
    pub(crate) fn l2_squared(a: &[f64], b: &[f64]) -> f64 {
        let chunks = a.len() / 2;
        let mut acc = vdupq_n_f64(0.0);
        for i in 0..chunks {
            let x = unsafe { vld1q_f64(a.as_ptr().add(i * 2)) };
            let y = unsafe { vld1q_f64(b.as_ptr().add(i * 2)) };
            let d = vsubq_f64(x, y);
            acc = vaddq_f64(acc, vmulq_f64(d, d));
        }
        let mut sum = vaddvq_f64(acc);
        for (x, y) in a[chunks * 2..].iter().zip(&b[chunks * 2..]) {
            let d = x - y;
            sum += d * d;
        }
        sum
    }

    /// NEON L1 distance (absolute differences via `vabsq_f64`).
    pub(crate) fn l1_distance(a: &[f64], b: &[f64]) -> f64 {
        let chunks = a.len() / 2;
        let mut acc = vdupq_n_f64(0.0);
        for i in 0..chunks {
            let x = unsafe { vld1q_f64(a.as_ptr().add(i * 2)) };
            let y = unsafe { vld1q_f64(b.as_ptr().add(i * 2)) };
            acc = vaddq_f64(acc, vabsq_f64(vsubq_f64(x, y)));
        }
        let mut sum = vaddvq_f64(acc);
        for (x, y) in a[chunks * 2..].iter().zip(&b[chunks * 2..]) {
            sum += (x - y).abs();
        }
        sum
    }
}

// ---------------------------------------------------------------------------
// Runtime dispatch
// ---------------------------------------------------------------------------

/// Dispatched single-pass dot + squared norms.
///
/// Picks AVX (when ≥[`MIN_DIM_AVX`]), else SSE2/NEON (when ≥[`MIN_DIM_SIMD`]),
/// else the scalar kernel. Unequal/empty operands return `(0.0, 0.0, 0.0)`.
#[inline]
pub fn dot_and_sq_norms(a: &[f64], b: &[f64]) -> (f64, f64, f64) {
    if a.len() != b.len() || a.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let dim = a.len();
    #[cfg(target_arch = "x86_64")]
    {
        if dim >= MIN_DIM_AVX && is_x86_feature_detected!("avx") {
            // SAFETY: AVX support runtime-verified; operands validated above.
            return unsafe { avx::dot_and_sq_norms(a, b) };
        }
        if dim >= MIN_DIM_SIMD && is_x86_feature_detected!("sse2") {
            // SAFETY: SSE2 is the x86_64 baseline and runtime-verified anyway.
            return unsafe { sse2::dot_and_sq_norms(a, b) };
        }
    }
    #[cfg(target_arch = "aarch64")]
    if dim >= MIN_DIM_SIMD {
        return neon::dot_and_sq_norms(a, b);
    }
    dot_and_sq_norms_scalar(a, b)
}

/// Dispatched squared L2 distance. Unequal/empty operands return `0.0`.
#[inline]
pub fn l2_squared(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dim = a.len();
    #[cfg(target_arch = "x86_64")]
    {
        if dim >= MIN_DIM_AVX && is_x86_feature_detected!("avx") {
            // SAFETY: AVX support runtime-verified; operands validated above.
            return unsafe { avx::l2_squared(a, b) };
        }
        if dim >= MIN_DIM_SIMD && is_x86_feature_detected!("sse2") {
            // SAFETY: SSE2 is the x86_64 baseline and runtime-verified anyway.
            return unsafe { sse2::l2_squared(a, b) };
        }
    }
    #[cfg(target_arch = "aarch64")]
    if dim >= MIN_DIM_SIMD {
        return neon::l2_squared(a, b);
    }
    l2_squared_scalar(a, b)
}

/// Dispatched L1 distance. Unequal/empty operands return `0.0`.
#[inline]
pub fn l1_distance(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dim = a.len();
    #[cfg(target_arch = "x86_64")]
    {
        if dim >= MIN_DIM_AVX && is_x86_feature_detected!("avx") {
            // SAFETY: AVX support runtime-verified; operands validated above.
            return unsafe { avx::l1_distance(a, b) };
        }
        if dim >= MIN_DIM_SIMD && is_x86_feature_detected!("sse2") {
            // SAFETY: SSE2 is the x86_64 baseline and runtime-verified anyway.
            return unsafe { sse2::l1_distance(a, b) };
        }
    }
    #[cfg(target_arch = "aarch64")]
    if dim >= MIN_DIM_SIMD {
        return neon::l1_distance(a, b);
    }
    l1_distance_scalar(a, b)
}

// ---------------------------------------------------------------------------
// Public distance surface
// ---------------------------------------------------------------------------

/// Cosine similarity — `dot / (||a|| * ||b||)`. Returns `0.0` for zero-norm
/// or shape-mismatched operands.
#[inline]
pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    let (dot, sq_a, sq_b) = dot_and_sq_norms(a, b);
    if sq_a == 0.0 || sq_b == 0.0 {
        return 0.0;
    }
    dot / (sq_a.sqrt() * sq_b.sqrt())
}

/// Cosine distance `1 - cos(a, b)` — the "smaller = more similar" convention
/// used by [`crate::hnsw::DistanceMetric::Cosine`].
#[inline]
pub fn cosine_distance(a: &[f64], b: &[f64]) -> f64 {
    1.0 - cosine_similarity(a, b)
}

/// Dot product of two vectors.
#[inline]
pub fn dot_product(a: &[f64], b: &[f64]) -> f64 {
    dot_and_sq_norms(a, b).0
}

/// Euclidean distance (sqrt of [`l2_squared`]).
#[inline]
pub fn euclidean_distance(a: &[f64], b: &[f64]) -> f64 {
    l2_squared(a, b).sqrt()
}

/// Squared norm (dot of a vector with itself).
#[inline]
pub fn precompute_sq_norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum()
}

// ---------------------------------------------------------------------------
// Batch primitives (G4)
// ---------------------------------------------------------------------------

/// Batch dot + squared-norm for one query against many keys.
///
/// Returns one `(dot, sq_norm_key)` per key; shape-mismatched keys contribute
/// `(0.0, 0.0)`.
pub fn batch_dot_and_sq_norms(query: &[f64], keys: &[&[f64]]) -> Vec<(f64, f64)> {
    keys.iter()
        .map(|k| {
            let (dot, _, sq) = dot_and_sq_norms(query, k);
            (dot, sq)
        })
        .collect()
}

/// Batch cosine similarity of one query against many keys.
///
/// Computes the query norm once, then reuses the dispatched [`dot_and_sq_norms`]
/// kernel per key. Zero-norm or shape-mismatched keys score `0.0`.
pub fn batch_cosine_similarities(query: &[f64], keys: &[&[f64]]) -> Vec<f64> {
    let norm_q = precompute_sq_norm(query);
    if norm_q == 0.0 {
        return vec![0.0; keys.len()];
    }
    // Pre-compute query magnitude once outside the loop to avoid redundant sqrt calls.
    let sqrt_norm_q = norm_q.sqrt();
    keys.iter()
        .map(|k| {
            let (dot, _, sq_k) = dot_and_sq_norms(query, k);
            if sq_k == 0.0 {
                0.0
            } else {
                dot / (sqrt_norm_q * sq_k.sqrt())
            }
        })
        .collect()
}

/// Return the indices of the top-`k` scores, descending, ties broken by
/// lower index for determinism. `k == 0` or an empty slice yields `[]`.
pub fn top_k_by_score(scores: &[f64], k: usize) -> Vec<usize> {
    if k == 0 {
        return Vec::new();
    }
    let mut idx: Vec<usize> = (0..scores.len()).collect();
    idx.sort_by(|&i, &j| scores[j].total_cmp(&scores[i]).then_with(|| i.cmp(&j)));
    idx.truncate(k);
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(a: f64, b: f64) {
        assert!((a - b).abs() <= 1e-9, "expected {a} to be within 1e-9 of {b}");
    }

    fn assert_triple_close(a: (f64, f64, f64), b: (f64, f64, f64)) {
        assert_close(a.0, b.0);
        assert_close(a.1, b.1);
        assert_close(a.2, b.2);
    }

    #[test]
    fn test_scalar_dot_and_sq_norms_known() {
        let a = [1.0, 2.0, 3.0];
        let b = [4.0, 5.0, 6.0];
        assert_triple_close(dot_and_sq_norms_scalar(&a, &b), (32.0, 14.0, 77.0));
    }

    #[test]
    fn test_kernels_match_scalar() {
        let len = 40;
        let a: Vec<f64> = (0..len).map(|i| (i as f64) * 0.5 - 3.0).collect();
        let b: Vec<f64> = (0..len).map(|i| (i as f64) % 7.0 - 2.0).collect();

        assert_triple_close(dot_and_sq_norms(&a, &b), dot_and_sq_norms_scalar(&a, &b));
        assert_close(l2_squared(&a, &b), l2_squared_scalar(&a, &b));
        assert_close(l1_distance(&a, &b), l1_distance_scalar(&a, &b));

        #[cfg(target_arch = "x86_64")]
        if is_x86_feature_detected!("avx") {
            let avx = unsafe { avx::dot_and_sq_norms(&a, &b) };
            assert_triple_close(avx, dot_and_sq_norms_scalar(&a, &b));
            assert_close(unsafe { avx::l2_squared(&a, &b) }, l2_squared_scalar(&a, &b));
            assert_close(unsafe { avx::l1_distance(&a, &b) }, l1_distance_scalar(&a, &b));
        }

        #[cfg(target_arch = "x86_64")]
        {
            let sse = unsafe { sse2::dot_and_sq_norms(&a, &b) };
            assert_triple_close(sse, dot_and_sq_norms_scalar(&a, &b));
            assert_close(unsafe { sse2::l2_squared(&a, &b) }, l2_squared_scalar(&a, &b));
            assert_close(unsafe { sse2::l1_distance(&a, &b) }, l1_distance_scalar(&a, &b));
        }
    }

    #[test]
    fn test_short_vectors_use_scalar_path() {
        // Below MIN_DIM_SIMD the dispatch must still agree with the scalar path.
        for len in 1..MIN_DIM_SIMD {
            let a: Vec<f64> = (0..len).map(|i| i as f64).collect();
            let b: Vec<f64> = (0..len).map(|i| (i * 3) as f64 % 5.0).collect();
            assert_triple_close(dot_and_sq_norms(&a, &b), dot_and_sq_norms_scalar(&a, &b));
            assert_close(l2_squared(&a, &b), l2_squared_scalar(&a, &b));
            assert_close(l1_distance(&a, &b), l1_distance_scalar(&a, &b));
        }
    }

    #[test]
    fn test_shape_mismatch_is_zero() {
        let a = [1.0, 2.0, 3.0];
        let b = [1.0, 2.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
        assert_eq!(l2_squared(&a, &b), 0.0);
        assert_eq!(l1_distance(&a, &b), 0.0);
        assert_triple_close(dot_and_sq_norms(&a, &b), (0.0, 0.0, 0.0));
    }

    #[test]
    fn test_cosine_known_values() {
        // Orthogonal unit vectors → similarity 0, distance 1.
        let x_axis = [1.0, 0.0, 0.0];
        let y_axis = [0.0, 1.0, 0.0];
        assert_close(cosine_similarity(&x_axis, &y_axis), 0.0);
        assert_close(cosine_distance(&x_axis, &y_axis), 1.0);

        // Parallel vectors → similarity 1, distance 0.
        let a = [1.0, 2.0, 3.0];
        let b = [2.0, 4.0, 6.0];
        assert_close(cosine_similarity(&a, &b), 1.0);
        assert_close(cosine_distance(&a, &b), 0.0);

        // Zero-norm vector → similarity 0.
        assert_close(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn test_dot_and_euclidean() {
        let a = [1.0, 2.0, 3.0];
        let b = [2.0, 3.0, 4.0];
        assert_close(dot_product(&a, &b), 20.0);
        // ||b - a|| = sqrt(1 + 1 + 1) = sqrt(3)
        assert_close(euclidean_distance(&a, &b), 3.0_f64.sqrt());
    }

    #[test]
    fn test_batch_matches_per_item() {
        let query = vec![1.0, 0.5, -0.5, 2.0];
        let keys: Vec<Vec<f64>> = vec![
            vec![1.0, 1.0, 1.0, 1.0],
            vec![-1.0, 0.0, 0.0, 1.0],
            vec![2.0, 1.0, -1.0, 4.0],
            vec![0.0, 0.0, 0.0, 0.0], // zero-norm key
        ];
        let key_refs: Vec<&[f64]> = keys.iter().map(|v| v.as_slice()).collect();

        let batch = batch_cosine_similarities(&query, &key_refs);
        for (i, got) in batch.iter().enumerate() {
            assert_close(*got, cosine_similarity(&query, key_refs[i]));
        }
        assert_close(batch[2], 1.0); // scalar multiple of query
        assert_close(batch[3], 0.0); // zero-norm key

        let dots = batch_dot_and_sq_norms(&query, &key_refs);
        assert_eq!(dot_product(&query, key_refs[0]), dots[0].0);
        assert_close(dots[2].1, precompute_sq_norm(&keys[2]));
    }

    #[test]
    fn test_top_k_by_score() {
        let scores = [0.5, 0.9, 0.9, 0.1, 0.7];
        let top = top_k_by_score(&scores, 3);
        // indices 1 and 2 tie at 0.9 → 1 first (lower index).
        assert_eq!(top, vec![1, 2, 4]);
        assert_eq!(top_k_by_score(&scores, 0), Vec::<usize>::new());
        assert_eq!(top_k_by_score(&[], 3), Vec::<usize>::new());
    }

    #[test]
    fn test_hnsw_cosine_distance_parity() {
        // Mirror of DistanceMetric::Cosine semantics (older scalar impl).
        let a = [1.0, 2.0, 3.0];
        let b = [4.0, 5.0, 6.0];
        let mut dot: f64 = 0.0;
        let mut n: f64 = 0.0;
        let mut m: f64 = 0.0;
        for (x, y) in a.iter().zip(b.iter()) {
            dot += x * y;
            n += x * x;
            m += y * y;
        }
        let expect = 1.0 - dot / (n.sqrt() * m.sqrt());
        assert_close(cosine_distance(&a, &b), expect);
    }
}
