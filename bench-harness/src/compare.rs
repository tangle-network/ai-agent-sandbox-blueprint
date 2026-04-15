//! Cross-run regression comparator.
//!
//! Given a baseline manifest and a current manifest, flag benchmarks whose
//! mean or p99 regressed beyond a configured threshold using confidence-interval
//! comparison, not raw point estimates. This is the mathematically honest way:
//! a "regression" is only real when the current-run's mean exceeds the baseline's
//! upper confidence bound AND the relative change exceeds the threshold.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::manifest::RunManifest;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonReport {
    pub baseline_run_id: String,
    pub current_run_id: String,
    pub threshold: Threshold,
    pub results: Vec<BenchComparison>,
    /// Count of benchmarks exceeding the regression threshold.
    pub regressions: usize,
    /// Count of benchmarks showing significant improvement.
    pub improvements: usize,
    /// Benchmarks in one run but not the other (id listed).
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Threshold {
    /// Mean regression threshold as a fraction (0.10 = 10%).
    pub mean_pct: f64,
    /// P99 regression threshold as a fraction.
    pub p99_pct: f64,
    /// Require CI-bound proof of regression (current lower > baseline upper)
    /// before flagging. Reduces false positives on noisy CI runners.
    pub require_ci_proof: bool,
}

impl Default for Threshold {
    fn default() -> Self {
        Threshold {
            mean_pct: 0.10,
            p99_pct: 0.15,
            require_ci_proof: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Verdict {
    /// Within threshold in both dimensions.
    Ok,
    /// Significant improvement (change below -threshold).
    Improved,
    /// Mean regressed beyond threshold.
    RegressedMean,
    /// P99 regressed beyond threshold.
    RegressedP99,
    /// Both mean and p99 regressed.
    RegressedBoth,
    /// Change is large but CI proof is required and not met.
    Noisy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchComparison {
    pub id: String,
    pub baseline_mean_ns: f64,
    pub current_mean_ns: f64,
    pub mean_change_pct: f64,
    pub baseline_p99_ns: f64,
    pub current_p99_ns: f64,
    pub p99_change_pct: f64,
    pub baseline_ci_upper_ns: f64,
    pub current_ci_lower_ns: f64,
    pub verdict: Verdict,
}

pub fn compare(
    baseline: &RunManifest,
    current: &RunManifest,
    threshold: Threshold,
) -> ComparisonReport {
    let mut baseline_map: BTreeMap<&str, &crate::criterion_ingest::BenchRecord> = BTreeMap::new();
    for b in &baseline.benches {
        baseline_map.insert(b.id.as_str(), b);
    }
    let mut current_map: BTreeMap<&str, &crate::criterion_ingest::BenchRecord> = BTreeMap::new();
    for b in &current.benches {
        current_map.insert(b.id.as_str(), b);
    }

    let added: Vec<String> = current_map
        .keys()
        .filter(|k| !baseline_map.contains_key(*k))
        .map(|k| (*k).to_string())
        .collect();
    let removed: Vec<String> = baseline_map
        .keys()
        .filter(|k| !current_map.contains_key(*k))
        .map(|k| (*k).to_string())
        .collect();

    let mut results = Vec::new();
    let mut regressions = 0usize;
    let mut improvements = 0usize;

    for (id, current) in &current_map {
        let Some(baseline) = baseline_map.get(id) else {
            continue;
        };
        let mean_change_pct = rel_change(baseline.summary.mean_ns, current.summary.mean_ns);
        let p99_change_pct = rel_change(baseline.summary.p99_ns, current.summary.p99_ns);

        let mean_regressed = mean_change_pct > threshold.mean_pct * 100.0;
        let p99_regressed = p99_change_pct > threshold.p99_pct * 100.0;
        let mean_improved = mean_change_pct < -(threshold.mean_pct * 100.0);

        // CI proof: current's lower bound must exceed baseline's upper bound
        // for a regression to be considered statistically real.
        let ci_proven_regression =
            current.summary.ci_lower_ns > baseline.summary.ci_upper_ns;

        let verdict = match (mean_regressed, p99_regressed, mean_improved) {
            (true, true, _) if !threshold.require_ci_proof || ci_proven_regression => {
                Verdict::RegressedBoth
            }
            (true, false, _) if !threshold.require_ci_proof || ci_proven_regression => {
                Verdict::RegressedMean
            }
            (false, true, _) => Verdict::RegressedP99,
            (_, _, true) => Verdict::Improved,
            (true, _, _) | (_, true, _) if threshold.require_ci_proof => Verdict::Noisy,
            _ => Verdict::Ok,
        };

        match &verdict {
            Verdict::RegressedMean | Verdict::RegressedP99 | Verdict::RegressedBoth => {
                regressions += 1
            }
            Verdict::Improved => improvements += 1,
            _ => {}
        }

        results.push(BenchComparison {
            id: (*id).to_string(),
            baseline_mean_ns: baseline.summary.mean_ns,
            current_mean_ns: current.summary.mean_ns,
            mean_change_pct,
            baseline_p99_ns: baseline.summary.p99_ns,
            current_p99_ns: current.summary.p99_ns,
            p99_change_pct,
            baseline_ci_upper_ns: baseline.summary.ci_upper_ns,
            current_ci_lower_ns: current.summary.ci_lower_ns,
            verdict,
        });
    }

    // Stable order
    results.sort_by(|a, b| a.id.cmp(&b.id));

    ComparisonReport {
        baseline_run_id: baseline.run_id.clone(),
        current_run_id: current.run_id.clone(),
        threshold,
        results,
        regressions,
        improvements,
        added,
        removed,
    }
}

fn rel_change(baseline: f64, current: f64) -> f64 {
    if baseline <= 0.0 || !baseline.is_finite() || !current.is_finite() {
        return 0.0;
    }
    (current - baseline) / baseline * 100.0
}

/// Render a comparison report as markdown suitable for a CI summary / PR comment.
pub fn render_markdown(report: &ComparisonReport) -> String {
    let mut out = String::new();
    out.push_str("# Benchmark Comparison\n\n");
    out.push_str(&format!(
        "- Baseline: `{}`\n- Current:  `{}`\n- Threshold: mean ±{:.0}%, p99 ±{:.0}%, CI-proof: {}\n",
        report.baseline_run_id,
        report.current_run_id,
        report.threshold.mean_pct * 100.0,
        report.threshold.p99_pct * 100.0,
        report.threshold.require_ci_proof,
    ));
    out.push_str(&format!(
        "- **Regressions: {}**  |  Improvements: {}  |  Added: {}  |  Removed: {}\n\n",
        report.regressions,
        report.improvements,
        report.added.len(),
        report.removed.len(),
    ));

    out.push_str("| Benchmark | Baseline mean (ns) | Current mean (ns) | Δ mean | Δ p99 | Verdict |\n");
    out.push_str("|-----------|-------------------:|------------------:|-------:|------:|:--------|\n");
    for r in &report.results {
        let verdict_str = match r.verdict {
            Verdict::Ok => "OK",
            Verdict::Improved => "IMPROVED",
            Verdict::RegressedMean => "REGRESSED (mean)",
            Verdict::RegressedP99 => "REGRESSED (p99)",
            Verdict::RegressedBoth => "REGRESSED (both)",
            Verdict::Noisy => "noisy (under-CI)",
        };
        out.push_str(&format!(
            "| {} | {:.1} | {:.1} | {:+.2}% | {:+.2}% | {} |\n",
            r.id,
            r.baseline_mean_ns,
            r.current_mean_ns,
            r.mean_change_pct,
            r.p99_change_pct,
            verdict_str,
        ));
    }

    if !report.added.is_empty() {
        out.push_str("\n## Added benchmarks\n");
        for id in &report.added {
            out.push_str(&format!("- {id}\n"));
        }
    }
    if !report.removed.is_empty() {
        out.push_str("\n## Removed benchmarks\n");
        for id in &report.removed {
            out.push_str(&format!("- {id}\n"));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::criterion_ingest::BenchRecord;
    use crate::stats::Summary;

    fn bench(id: &str, mean: f64, p99: f64) -> BenchRecord {
        let samples: Vec<f64> = (0..30).map(|_| mean).collect();
        let mut summary = Summary::from_samples(&samples);
        summary.p99_ns = p99;
        // Tighten the CI so comparisons are deterministic in tests.
        summary.ci_lower_ns = mean * 0.99;
        summary.ci_upper_ns = mean * 1.01;
        BenchRecord {
            id: id.to_string(),
            source: String::new(),
            summary,
            criterion_mean_ns: mean,
            criterion_median_ns: mean,
            criterion_stddev_ns: 0.1,
            criterion_ci_lower_ns: mean * 0.99,
            criterion_ci_upper_ns: mean * 1.01,
            criterion_confidence_level: 0.95,
            criterion_change_pct: None,
        }
    }

    fn manifest(id: &str, benches: Vec<BenchRecord>) -> RunManifest {
        let mut m = RunManifest::collect(benches).unwrap();
        m.run_id = id.to_string();
        m
    }

    #[test]
    fn ok_when_no_change() {
        let baseline = manifest("base", vec![bench("a", 100.0, 200.0)]);
        let current = manifest("curr", vec![bench("a", 100.5, 201.0)]);
        let report = compare(&baseline, &current, Threshold::default());
        assert_eq!(report.regressions, 0);
        assert_eq!(report.results[0].verdict, Verdict::Ok);
    }

    #[test]
    fn flags_mean_regression_with_ci_proof() {
        // 30% slower mean — well above 10% threshold
        let baseline = manifest("base", vec![bench("a", 100.0, 200.0)]);
        let current = manifest("curr", vec![bench("a", 130.0, 240.0)]);
        let report = compare(&baseline, &current, Threshold::default());
        assert_eq!(report.regressions, 1);
        assert_eq!(report.results[0].verdict, Verdict::RegressedBoth);
    }

    #[test]
    fn flags_improvement_when_mean_drops() {
        let baseline = manifest("base", vec![bench("a", 100.0, 200.0)]);
        let current = manifest("curr", vec![bench("a", 70.0, 150.0)]);
        let report = compare(&baseline, &current, Threshold::default());
        assert_eq!(report.improvements, 1);
        assert_eq!(report.results[0].verdict, Verdict::Improved);
    }

    #[test]
    fn tracks_added_and_removed() {
        let baseline = manifest(
            "base",
            vec![bench("a", 100.0, 200.0), bench("removed", 50.0, 100.0)],
        );
        let current = manifest(
            "curr",
            vec![bench("a", 100.0, 200.0), bench("added", 25.0, 50.0)],
        );
        let report = compare(&baseline, &current, Threshold::default());
        assert_eq!(report.added, vec!["added".to_string()]);
        assert_eq!(report.removed, vec!["removed".to_string()]);
    }

    #[test]
    fn noisy_verdict_when_ci_overlap_despite_large_change() {
        // Large point-estimate change but overlapping CIs — should be Noisy.
        // We have to construct this by hand because the helper makes tight CIs.
        let mut b_record = bench("a", 100.0, 200.0);
        b_record.summary.ci_upper_ns = 200.0;
        let mut c_record = bench("a", 130.0, 220.0);
        c_record.summary.ci_lower_ns = 90.0;
        let baseline = manifest("base", vec![b_record]);
        let current = manifest("curr", vec![c_record]);
        let report = compare(&baseline, &current, Threshold::default());
        assert_eq!(
            report.regressions, 0,
            "CI overlap should prevent regression flag"
        );
        assert_eq!(report.results[0].verdict, Verdict::Noisy);
    }

    #[test]
    fn markdown_render_contains_key_fields() {
        let baseline = manifest("base-1", vec![bench("a", 100.0, 200.0)]);
        let current = manifest("curr-1", vec![bench("a", 130.0, 240.0)]);
        let report = compare(&baseline, &current, Threshold::default());
        let md = render_markdown(&report);
        assert!(md.contains("base-1"));
        assert!(md.contains("curr-1"));
        assert!(md.contains("REGRESSED"));
    }
}
