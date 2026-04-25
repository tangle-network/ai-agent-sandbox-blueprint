//! `bench-harness` CLI.
//!
//! Three subcommands:
//!
//! - `collect` — walk `target/criterion/` and write a run manifest to
//!   `bench-results/latest.json` (plus append to `bench-results/runs.jsonl`).
//! - `compare` — diff a current manifest against a baseline, print a markdown
//!   report, exit non-zero on regressions.
//! - `report` — render an existing manifest as markdown (no comparison).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use bench_harness::{
    compare::{Threshold, compare, render_markdown},
    criterion_ingest::{collect_all, default_criterion_dir},
    manifest::RunManifest,
};

#[derive(Parser, Debug)]
#[command(about = "Benchmark harness — aggregate, compare, report")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Ingest Criterion output and write the run manifest.
    Collect {
        /// Workspace root (where `target/criterion` lives).
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        /// Where to write the full per-run JSON.
        #[arg(long, default_value = "bench-results/latest.json")]
        output: PathBuf,
        /// Append run to this JSONL file.
        #[arg(long, default_value = "bench-results/runs.jsonl")]
        jsonl: PathBuf,
    },
    /// Compare a current manifest against a baseline.
    Compare {
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long)]
        current: PathBuf,
        /// Mean regression threshold as a fraction (0.10 = 10%).
        #[arg(long, default_value_t = 0.10)]
        mean_threshold: f64,
        /// p99 regression threshold as a fraction.
        #[arg(long, default_value_t = 0.15)]
        p99_threshold: f64,
        /// Mean regression must also exceed this absolute nanosecond delta.
        #[arg(long, default_value_t = 10.0)]
        mean_abs_ns: f64,
        /// P99 regression must also exceed this absolute nanosecond delta.
        #[arg(long, default_value_t = 10.0)]
        p99_abs_ns: f64,
        /// Require CI-bound proof (current lower > baseline upper) before flagging.
        #[arg(long, default_value_t = true)]
        require_ci_proof: bool,
        /// Write markdown output to this path (also printed to stdout).
        #[arg(long)]
        markdown_output: Option<PathBuf>,
        /// If set, exit 0 even when regressions exist (report-only mode).
        #[arg(long, default_value_t = false)]
        no_fail: bool,
    },
    /// Render a manifest as markdown.
    Report {
        #[arg(long)]
        manifest: PathBuf,
    },
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Collect {
            workspace,
            output,
            jsonl,
        } => collect(workspace, output, jsonl),
        Cmd::Compare {
            baseline,
            current,
            mean_threshold,
            p99_threshold,
            mean_abs_ns,
            p99_abs_ns,
            require_ci_proof,
            markdown_output,
            no_fail,
        } => {
            let threshold = Threshold {
                mean_pct: mean_threshold,
                p99_pct: p99_threshold,
                mean_abs_ns,
                p99_abs_ns,
                require_ci_proof,
            };
            compare_cmd(baseline, current, threshold, markdown_output, no_fail)
        }
        Cmd::Report { manifest } => report_cmd(manifest),
    }
}

fn collect(workspace: PathBuf, output: PathBuf, jsonl: PathBuf) -> Result<ExitCode> {
    let criterion_dir = default_criterion_dir(&workspace);
    let records = collect_all(&criterion_dir).context("collecting Criterion output")?;
    tracing::info!(bench_count = records.len(), "collected benchmarks");

    if records.is_empty() {
        tracing::warn!(
            path = %criterion_dir.display(),
            "no Criterion output found — did you run `cargo bench` first?"
        );
    }

    let manifest = RunManifest::collect(records)?;
    manifest.write_json(&output)?;
    manifest.append_to_jsonl(&jsonl)?;

    println!(
        "wrote {} benchmarks to {} (run_id={})",
        manifest.bench_count,
        output.display(),
        manifest.run_id
    );
    Ok(ExitCode::from(0))
}

fn compare_cmd(
    baseline_path: PathBuf,
    current_path: PathBuf,
    threshold: Threshold,
    markdown_output: Option<PathBuf>,
    no_fail: bool,
) -> Result<ExitCode> {
    let baseline: RunManifest = read_manifest(&baseline_path)?;
    let current: RunManifest = read_manifest(&current_path)?;

    let report = compare(&baseline, &current, threshold);
    let md = render_markdown(&report);

    println!("{md}");
    if let Some(path) = markdown_output {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&path, &md).with_context(|| format!("writing {}", path.display()))?;
    }

    if report.regressions > 0 && !no_fail {
        eprintln!(
            "FAIL: {} benchmark(s) regressed beyond threshold",
            report.regressions
        );
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::from(0))
}

fn report_cmd(manifest_path: PathBuf) -> Result<ExitCode> {
    let manifest: RunManifest = read_manifest(&manifest_path)?;
    println!("# Benchmark Run: {}", manifest.run_id);
    println!();
    println!("- Timestamp: {}", manifest.timestamp_utc);
    println!(
        "- Git: {} on {} ({})",
        manifest.git.sha,
        manifest.git.branch,
        if manifest.git.dirty { "dirty" } else { "clean" }
    );
    println!(
        "- Host: {} {}/{} ({} CPUs)",
        manifest.host.hostname, manifest.host.os, manifest.host.arch, manifest.host.cpu_count
    );
    println!(
        "- Rust: {} on {} ({})",
        manifest.rust.rustc_version, manifest.rust.target_triple, manifest.rust.profile
    );
    println!("- Benchmarks: {}", manifest.bench_count);
    println!();
    println!("| Benchmark | mean (ns) | p50 | p95 | p99 | stddev | throughput/s |");
    println!("|-----------|----------:|----:|----:|----:|-------:|-------------:|");
    for b in &manifest.benches {
        println!(
            "| {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.0} |",
            b.id,
            b.summary.mean_ns,
            b.summary.p50_ns,
            b.summary.p95_ns,
            b.summary.p99_ns,
            b.summary.stddev_ns,
            b.summary.throughput_ops_per_sec,
        );
    }
    Ok(ExitCode::from(0))
}

fn read_manifest(path: &std::path::Path) -> Result<RunManifest> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let manifest: RunManifest =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(manifest)
}
