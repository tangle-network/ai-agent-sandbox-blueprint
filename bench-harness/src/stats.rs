//! Descriptive statistics for a vector of sample measurements.
//!
//! Criterion already performs bootstrap resampling, outlier detection, and
//! Student's t-test internally and exposes the results in its `estimates.json`
//! files. This module provides a parallel, self-contained implementation used
//! when we need to compute stats over Criterion's iteration vector directly
//! (e.g. for the run manifest) or when comparing two vectors of samples from
//! different runs.

use serde::{Deserialize, Serialize};

/// Full statistical summary of a sample vector.
///
/// All time fields are nanoseconds. `confidence_level` is the fraction of
/// bootstrap iterations whose mean fell inside `[ci_lower_ns, ci_upper_ns]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    pub n: usize,
    pub min_ns: f64,
    pub max_ns: f64,
    pub mean_ns: f64,
    pub median_ns: f64,
    pub stddev_ns: f64,
    pub variance_ns2: f64,
    /// Median absolute deviation — robust alternative to stddev.
    pub mad_ns: f64,
    pub p50_ns: f64,
    pub p90_ns: f64,
    pub p95_ns: f64,
    pub p99_ns: f64,
    pub p999_ns: f64,
    /// 95% confidence interval for the mean, via basic bootstrap.
    pub ci_lower_ns: f64,
    pub ci_upper_ns: f64,
    pub confidence_level: f64,
    /// Tukey outlier count using 1.5*IQR rule on the high side.
    pub outliers_high: usize,
    pub outliers_low: usize,
    /// Throughput estimate: ops per second derived from the mean.
    pub throughput_ops_per_sec: f64,
}

impl Summary {
    /// Compute a full statistical summary from a non-empty sample vector.
    ///
    /// # Panics
    /// Does not panic — returns a `Summary` with all-NaN values if the input
    /// is empty. Callers should guard against empty input at the type level.
    pub fn from_samples(samples: &[f64]) -> Self {
        if samples.is_empty() {
            return Self::empty();
        }
        let mut sorted: Vec<f64> = samples.iter().copied().filter(|v| v.is_finite()).collect();
        if sorted.is_empty() {
            return Self::empty();
        }
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let n = sorted.len();
        let min_ns = sorted[0];
        let max_ns = sorted[n - 1];
        let sum: f64 = sorted.iter().sum();
        let mean_ns = sum / n as f64;

        let variance_ns2 = if n > 1 {
            sorted.iter().map(|v| (v - mean_ns).powi(2)).sum::<f64>() / (n as f64 - 1.0)
        } else {
            0.0
        };
        let stddev_ns = variance_ns2.sqrt();

        let median_ns = percentile(&sorted, 0.50);
        let p50_ns = median_ns;
        let p90_ns = percentile(&sorted, 0.90);
        let p95_ns = percentile(&sorted, 0.95);
        let p99_ns = percentile(&sorted, 0.99);
        let p999_ns = percentile(&sorted, 0.999);

        // Median absolute deviation
        let mut abs_dev: Vec<f64> = sorted.iter().map(|v| (v - median_ns).abs()).collect();
        abs_dev.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mad_ns = percentile(&abs_dev, 0.50);

        // Tukey outlier fences (1.5 * IQR)
        let q1 = percentile(&sorted, 0.25);
        let q3 = percentile(&sorted, 0.75);
        let iqr = q3 - q1;
        let low_fence = q1 - 1.5 * iqr;
        let high_fence = q3 + 1.5 * iqr;
        let outliers_low = sorted.iter().filter(|&&v| v < low_fence).count();
        let outliers_high = sorted.iter().filter(|&&v| v > high_fence).count();

        // Bootstrap 95% CI for the mean using deterministic percentile method.
        // We use a simple seeded LCG so results are reproducible across runs.
        let (ci_lower_ns, ci_upper_ns) = bootstrap_mean_ci(&sorted, 1000, 0.95);

        let throughput_ops_per_sec = if mean_ns > 0.0 {
            1e9 / mean_ns
        } else {
            f64::INFINITY
        };

        Summary {
            n,
            min_ns,
            max_ns,
            mean_ns,
            median_ns,
            stddev_ns,
            variance_ns2,
            mad_ns,
            p50_ns,
            p90_ns,
            p95_ns,
            p99_ns,
            p999_ns,
            ci_lower_ns,
            ci_upper_ns,
            confidence_level: 0.95,
            outliers_high,
            outliers_low,
            throughput_ops_per_sec,
        }
    }

    fn empty() -> Self {
        Summary {
            n: 0,
            min_ns: f64::NAN,
            max_ns: f64::NAN,
            mean_ns: f64::NAN,
            median_ns: f64::NAN,
            stddev_ns: f64::NAN,
            variance_ns2: f64::NAN,
            mad_ns: f64::NAN,
            p50_ns: f64::NAN,
            p90_ns: f64::NAN,
            p95_ns: f64::NAN,
            p99_ns: f64::NAN,
            p999_ns: f64::NAN,
            ci_lower_ns: f64::NAN,
            ci_upper_ns: f64::NAN,
            confidence_level: 0.95,
            outliers_high: 0,
            outliers_low: 0,
            throughput_ops_per_sec: f64::NAN,
        }
    }
}

/// Linear-interpolation percentile. `sorted` must be ascending and non-empty.
/// `p` is in [0, 1].
fn percentile(sorted: &[f64], p: f64) -> f64 {
    debug_assert!(!sorted.is_empty());
    let n = sorted.len();
    if n == 1 {
        return sorted[0];
    }
    let rank = p * (n as f64 - 1.0);
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let frac = rank - lo as f64;
        sorted[lo] * (1.0 - frac) + sorted[hi] * frac
    }
}

/// Basic bootstrap confidence interval for the mean.
///
/// Uses a deterministic LCG seeded from the input length so CI outputs are
/// reproducible across runs given the same sample vector.
fn bootstrap_mean_ci(sorted: &[f64], iterations: usize, confidence: f64) -> (f64, f64) {
    let n = sorted.len();
    if n < 2 {
        let m = sorted.first().copied().unwrap_or(f64::NAN);
        return (m, m);
    }

    // Deterministic LCG (Numerical Recipes)
    let mut state: u64 = (n as u64)
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut means: Vec<f64> = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        let mut sum = 0.0;
        for _ in 0..n {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let idx = (state >> 33) as usize % n;
            sum += sorted[idx];
        }
        means.push(sum / n as f64);
    }

    means.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let alpha = 1.0 - confidence;
    let lower = percentile(&means, alpha / 2.0);
    let upper = percentile(&means, 1.0 - alpha / 2.0);
    (lower, upper)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_produces_nan_summary() {
        let s = Summary::from_samples(&[]);
        assert_eq!(s.n, 0);
        assert!(s.mean_ns.is_nan());
    }

    #[test]
    fn single_sample_produces_zero_variance() {
        let s = Summary::from_samples(&[100.0]);
        assert_eq!(s.n, 1);
        assert_eq!(s.mean_ns, 100.0);
        assert_eq!(s.min_ns, 100.0);
        assert_eq!(s.max_ns, 100.0);
        assert_eq!(s.variance_ns2, 0.0);
        assert_eq!(s.stddev_ns, 0.0);
    }

    #[test]
    fn percentile_interpolation_is_correct() {
        // Known values: for [1, 2, 3, 4, 5], median = 3, p90 = 4.6
        let s = Summary::from_samples(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert!((s.median_ns - 3.0).abs() < 1e-9);
        assert!((s.p90_ns - 4.6).abs() < 1e-9);
    }

    #[test]
    fn mean_stddev_are_correct_for_known_data() {
        // [2, 4, 4, 4, 5, 5, 7, 9] → mean=5, variance=4, stddev=2 (sample)
        let s = Summary::from_samples(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
        assert!((s.mean_ns - 5.0).abs() < 1e-9);
        // Sample stddev (n-1): sqrt((9+1+1+1+0+0+4+16)/7) = sqrt(32/7) ≈ 2.138
        assert!((s.stddev_ns - (32.0f64 / 7.0).sqrt()).abs() < 1e-9);
    }

    #[test]
    fn throughput_is_reciprocal_of_mean() {
        // 1ms mean → 1000 ops/sec
        let s = Summary::from_samples(&[1_000_000.0; 10]);
        assert!((s.throughput_ops_per_sec - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn outliers_detected_via_tukey_fence() {
        // [10 values clustered around 100] + one extreme outlier
        let mut samples = vec![100.0; 20];
        samples.push(10_000.0);
        let s = Summary::from_samples(&samples);
        assert!(s.outliers_high >= 1, "should flag extreme high outlier");
    }

    #[test]
    fn confidence_interval_brackets_the_mean() {
        let samples: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let s = Summary::from_samples(&samples);
        assert!(s.ci_lower_ns <= s.mean_ns);
        assert!(s.ci_upper_ns >= s.mean_ns);
    }

    #[test]
    fn filters_non_finite_samples() {
        let samples = vec![1.0, f64::NAN, 2.0, f64::INFINITY, 3.0];
        let s = Summary::from_samples(&samples);
        assert_eq!(s.n, 3, "NaN and inf should be filtered out");
        assert!((s.mean_ns - 2.0).abs() < 1e-9);
    }

    #[test]
    fn mad_is_robust_to_outliers() {
        // Dataset with one massive outlier — MAD should be small while stddev is huge
        let mut samples = vec![100.0; 50];
        samples.push(1_000_000.0);
        let s = Summary::from_samples(&samples);
        assert!(
            s.mad_ns < 10.0,
            "MAD should be small despite outlier: {}",
            s.mad_ns
        );
        assert!(
            s.stddev_ns > 10_000.0,
            "stddev should be inflated by outlier: {}",
            s.stddev_ns
        );
    }
}
