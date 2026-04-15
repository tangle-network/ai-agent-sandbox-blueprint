//! Read Criterion's machine-readable output from `target/criterion/`.
//!
//! Criterion writes per-benchmark directories containing `new/estimates.json`
//! (the latest run) and `base/estimates.json` (previous baseline). Each file
//! is a JSON object with `mean`, `median`, `std_dev`, `slope`, etc., each
//! carrying `point_estimate`, `confidence_interval.lower_bound`,
//! `confidence_interval.upper_bound`, and `confidence_interval.confidence_level`.
//!
//! There is also a `new/sample.json` with the raw iteration vector; we ingest
//! both and use whichever the caller asks for.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::stats::Summary;

/// One benchmark's aggregated data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchRecord {
    /// Full dotted path: "group/function/parameter"
    pub id: String,
    /// Path to the estimates.json file (for traceability)
    pub source: String,
    /// Derived statistics from the raw sample vector.
    pub summary: Summary,
    /// Criterion's reported mean (ns) — included verbatim for comparison.
    pub criterion_mean_ns: f64,
    pub criterion_median_ns: f64,
    pub criterion_stddev_ns: f64,
    pub criterion_ci_lower_ns: f64,
    pub criterion_ci_upper_ns: f64,
    pub criterion_confidence_level: f64,
    /// Change relative to Criterion's previous baseline, when available.
    pub criterion_change_pct: Option<f64>,
}

/// Criterion's `estimates.json` schema (just the fields we use).
#[derive(Debug, Deserialize)]
struct Estimate {
    point_estimate: f64,
    confidence_interval: ConfidenceInterval,
}

#[derive(Debug, Deserialize)]
struct ConfidenceInterval {
    lower_bound: f64,
    upper_bound: f64,
    confidence_level: f64,
}

#[derive(Debug, Deserialize)]
struct Estimates {
    mean: Estimate,
    median: Estimate,
    std_dev: Estimate,
}

/// Criterion's `change/estimates.json` is similar but with a `mean` field
/// whose `point_estimate` is the relative change (fraction).
#[derive(Debug, Deserialize)]
struct ChangeEstimates {
    mean: Estimate,
}

/// Criterion's `sample.json` — raw iteration counts and timings.
#[derive(Debug, Deserialize)]
struct Sample {
    iters: Vec<f64>,
    times: Vec<f64>,
}

/// Walk `target/criterion/` and collect every benchmark with a `new/` subdir.
pub fn collect_all(criterion_dir: &Path) -> Result<Vec<BenchRecord>> {
    if !criterion_dir.exists() {
        return Ok(Vec::new());
    }

    let pattern = criterion_dir.join("**/new/estimates.json");
    let pattern_str = pattern.to_string_lossy().to_string();

    let mut records = Vec::new();
    for entry in glob::glob(&pattern_str).context("invalid glob pattern")? {
        let estimates_path = match entry {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(error = %err, "skipping unreadable criterion entry");
                continue;
            }
        };

        match ingest_bench(&estimates_path, criterion_dir) {
            Ok(record) => records.push(record),
            Err(err) => {
                tracing::warn!(
                    path = %estimates_path.display(),
                    error = %err,
                    "failed to ingest benchmark; skipping"
                );
            }
        }
    }

    // Stable order so manifest diffs are deterministic
    records.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(records)
}

fn ingest_bench(estimates_path: &Path, criterion_root: &Path) -> Result<BenchRecord> {
    let estimates_text = fs::read_to_string(estimates_path)
        .with_context(|| format!("reading {}", estimates_path.display()))?;
    let estimates: Estimates = serde_json::from_str(&estimates_text)
        .with_context(|| format!("parsing {}", estimates_path.display()))?;

    // Derive the benchmark id from the path relative to criterion/
    let new_dir = estimates_path
        .parent()
        .context("estimates.json has no parent")?;
    let bench_dir = new_dir.parent().context("new/ has no parent")?;
    let id = bench_dir
        .strip_prefix(criterion_root)
        .unwrap_or(bench_dir)
        .to_string_lossy()
        .replace('\\', "/");

    // Raw samples
    let sample_path = new_dir.join("sample.json");
    let summary = if sample_path.exists() {
        let sample_text = fs::read_to_string(&sample_path)
            .with_context(|| format!("reading {}", sample_path.display()))?;
        let sample: Sample = serde_json::from_str(&sample_text)
            .with_context(|| format!("parsing {}", sample_path.display()))?;
        let per_iter: Vec<f64> = sample
            .times
            .iter()
            .zip(sample.iters.iter())
            .filter_map(|(t, i)| if *i > 0.0 { Some(t / i) } else { None })
            .collect();
        Summary::from_samples(&per_iter)
    } else {
        // Fall back to a sparse summary derived from Criterion's estimates alone
        Summary {
            n: 0,
            min_ns: f64::NAN,
            max_ns: f64::NAN,
            mean_ns: estimates.mean.point_estimate,
            median_ns: estimates.median.point_estimate,
            stddev_ns: estimates.std_dev.point_estimate,
            variance_ns2: estimates.std_dev.point_estimate.powi(2),
            mad_ns: f64::NAN,
            p50_ns: estimates.median.point_estimate,
            p90_ns: f64::NAN,
            p95_ns: f64::NAN,
            p99_ns: f64::NAN,
            p999_ns: f64::NAN,
            ci_lower_ns: estimates.mean.confidence_interval.lower_bound,
            ci_upper_ns: estimates.mean.confidence_interval.upper_bound,
            confidence_level: estimates.mean.confidence_interval.confidence_level,
            outliers_high: 0,
            outliers_low: 0,
            throughput_ops_per_sec: if estimates.mean.point_estimate > 0.0 {
                1e9 / estimates.mean.point_estimate
            } else {
                f64::INFINITY
            },
        }
    };

    // Optional: Criterion's own change estimate (from previous baseline)
    let change_path = bench_dir.join("change/estimates.json");
    let criterion_change_pct = if change_path.exists() {
        match fs::read_to_string(&change_path)
            .ok()
            .and_then(|t| serde_json::from_str::<ChangeEstimates>(&t).ok())
        {
            Some(ce) => Some(ce.mean.point_estimate * 100.0),
            None => None,
        }
    } else {
        None
    };

    Ok(BenchRecord {
        id,
        source: estimates_path.to_string_lossy().to_string(),
        summary,
        criterion_mean_ns: estimates.mean.point_estimate,
        criterion_median_ns: estimates.median.point_estimate,
        criterion_stddev_ns: estimates.std_dev.point_estimate,
        criterion_ci_lower_ns: estimates.mean.confidence_interval.lower_bound,
        criterion_ci_upper_ns: estimates.mean.confidence_interval.upper_bound,
        criterion_confidence_level: estimates.mean.confidence_interval.confidence_level,
        criterion_change_pct,
    })
}

/// Default Criterion output directory within a Cargo workspace.
pub fn default_criterion_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("target/criterion")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = fs::File::create(path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
    }

    fn estimates_json(mean: f64, median: f64, stddev: f64) -> String {
        format!(
            r#"{{
                "mean": {{
                    "confidence_interval": {{ "confidence_level": 0.95, "lower_bound": {m_lo}, "upper_bound": {m_hi} }},
                    "point_estimate": {mean},
                    "standard_error": 0.0
                }},
                "median": {{
                    "confidence_interval": {{ "confidence_level": 0.95, "lower_bound": {md_lo}, "upper_bound": {md_hi} }},
                    "point_estimate": {median},
                    "standard_error": 0.0
                }},
                "median_abs_dev": {{
                    "confidence_interval": {{ "confidence_level": 0.95, "lower_bound": 0, "upper_bound": 0 }},
                    "point_estimate": 0,
                    "standard_error": 0
                }},
                "slope": {{
                    "confidence_interval": {{ "confidence_level": 0.95, "lower_bound": {mean}, "upper_bound": {mean} }},
                    "point_estimate": {mean},
                    "standard_error": 0
                }},
                "std_dev": {{
                    "confidence_interval": {{ "confidence_level": 0.95, "lower_bound": 0, "upper_bound": 0 }},
                    "point_estimate": {stddev},
                    "standard_error": 0
                }}
            }}"#,
            mean = mean,
            median = median,
            stddev = stddev,
            m_lo = mean * 0.95,
            m_hi = mean * 1.05,
            md_lo = median * 0.95,
            md_hi = median * 1.05,
        )
    }

    fn sample_json(samples_per_iter: &[f64]) -> String {
        // 1 iter per sample so that time/iter equals the sample value directly
        let times: Vec<String> = samples_per_iter.iter().map(|v| v.to_string()).collect();
        let iters: Vec<String> = samples_per_iter.iter().map(|_| "1".to_string()).collect();
        format!(
            r#"{{"iters":[{}],"times":[{}]}}"#,
            iters.join(","),
            times.join(",")
        )
    }

    #[test]
    fn collects_empty_when_no_criterion_dir() {
        let dir = TempDir::new().unwrap();
        let records = collect_all(&dir.path().join("missing")).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn ingest_single_benchmark_with_samples() {
        let dir = TempDir::new().unwrap();
        let crit = dir.path().join("criterion");
        let bench = crit.join("group1/fn1/new");
        write(
            &bench.join("estimates.json"),
            &estimates_json(100.0, 98.0, 5.0),
        );
        write(
            &bench.join("sample.json"),
            &sample_json(&[95.0, 100.0, 105.0, 98.0, 102.0]),
        );

        let records = collect_all(&crit).unwrap();
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.id, "group1/fn1");
        assert_eq!(r.criterion_mean_ns, 100.0);
        assert!(r.summary.n > 0, "should have samples");
        assert!(r.summary.mean_ns > 90.0 && r.summary.mean_ns < 110.0);
    }

    #[test]
    fn ingest_multiple_benchmarks_sorted() {
        let dir = TempDir::new().unwrap();
        let crit = dir.path().join("criterion");
        for name in ["zzz", "aaa", "mmm"] {
            let bench = crit.join(format!("{name}/fn/new"));
            write(
                &bench.join("estimates.json"),
                &estimates_json(100.0, 100.0, 1.0),
            );
            write(&bench.join("sample.json"), &sample_json(&[100.0]));
        }

        let records = collect_all(&crit).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].id, "aaa/fn");
        assert_eq!(records[1].id, "mmm/fn");
        assert_eq!(records[2].id, "zzz/fn");
    }

    #[test]
    fn ingests_change_estimates_when_present() {
        let dir = TempDir::new().unwrap();
        let crit = dir.path().join("criterion");
        let bench_dir = crit.join("grp/fn");
        write(
            &bench_dir.join("new/estimates.json"),
            &estimates_json(100.0, 100.0, 1.0),
        );
        write(&bench_dir.join("new/sample.json"), &sample_json(&[100.0]));
        // Criterion encodes change as a fraction; 0.05 → +5%
        write(
            &bench_dir.join("change/estimates.json"),
            &estimates_json(0.05, 0.05, 0.0),
        );

        let records = collect_all(&crit).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].criterion_change_pct, Some(5.0));
    }

    #[test]
    fn skips_malformed_estimates() {
        let dir = TempDir::new().unwrap();
        let crit = dir.path().join("criterion");
        // One good benchmark
        let good = crit.join("good/fn/new");
        write(
            &good.join("estimates.json"),
            &estimates_json(100.0, 100.0, 1.0),
        );
        write(&good.join("sample.json"), &sample_json(&[100.0]));
        // One malformed benchmark
        let bad = crit.join("bad/fn/new");
        write(&bad.join("estimates.json"), "{not valid json");
        write(&bad.join("sample.json"), &sample_json(&[1.0]));

        let records = collect_all(&crit).unwrap();
        assert_eq!(records.len(), 1, "should skip malformed and keep good");
        assert_eq!(records[0].id, "good/fn");
    }
}
