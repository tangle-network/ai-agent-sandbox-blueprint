//! Run manifest — a JSONL record per benchmark run with reproducibility metadata.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::criterion_ingest::BenchRecord;

/// A single benchmark run.
///
/// Written as one JSON line to `bench-results/runs.jsonl` for time-series
/// analysis. The full report (with all benchmarks) also goes to
/// `bench-results/latest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunManifest {
    pub run_id: String,
    pub timestamp_utc: DateTime<Utc>,
    pub git: GitInfo,
    pub host: HostInfo,
    pub rust: RustInfo,
    pub env: BTreeMap<String, String>,
    pub bench_count: usize,
    pub benches: Vec<BenchRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitInfo {
    pub sha: String,
    pub branch: String,
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInfo {
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub cpu_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustInfo {
    pub rustc_version: String,
    pub target_triple: String,
    pub profile: String,
}

impl RunManifest {
    pub fn collect(records: Vec<BenchRecord>) -> Result<Self> {
        let run_id = format!("{}-{}", Utc::now().format("%Y%m%dT%H%M%SZ"), uuid_short());

        // Gather env vars relevant to reproducibility. We whitelist specific
        // keys rather than dump everything to avoid leaking secrets.
        let env_keys = [
            "GITHUB_ACTIONS",
            "GITHUB_SHA",
            "GITHUB_REF",
            "GITHUB_RUN_ID",
            "GITHUB_RUNNER_NAME",
            "CARGO_PROFILE",
            "RUSTFLAGS",
            "RUSTC_WRAPPER",
            "RUST_BACKTRACE",
        ];
        let env: BTreeMap<String, String> = env_keys
            .iter()
            .filter_map(|k| std::env::var(k).ok().map(|v| ((*k).to_string(), v)))
            .collect();

        Ok(Self {
            run_id,
            timestamp_utc: Utc::now(),
            git: gather_git_info(),
            host: gather_host_info(),
            rust: gather_rust_info(),
            env,
            bench_count: records.len(),
            benches: records,
        })
    }

    /// Append this manifest as a single JSON line to the given JSONL file.
    pub fn append_to_jsonl(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let line = serde_json::to_string(self).context("serializing manifest")?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        writeln!(file, "{line}").context("writing manifest line")?;
        Ok(())
    }

    /// Write the full (pretty-printed) manifest to a JSON file.
    pub fn write_json(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let json = serde_json::to_string_pretty(self).context("serializing manifest")?;
        std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

fn gather_git_info() -> GitInfo {
    let sha = run_stdout(&["rev-parse", "HEAD"])
        .or_else(|| std::env::var("GITHUB_SHA").ok())
        .unwrap_or_else(|| "unknown".to_string());
    let branch = run_stdout(&["rev-parse", "--abbrev-ref", "HEAD"])
        .or_else(|| std::env::var("GITHUB_REF_NAME").ok())
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    GitInfo { sha, branch, dirty }
}

fn run_stdout(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn gather_host_info() -> HostInfo {
    HostInfo {
        hostname: hostname(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu_count: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0),
    }
}

fn hostname() -> String {
    // Prefer `hostname` env var (set in many CI environments); fall back to
    // running `hostname` or `uname -n`.
    if let Ok(h) = std::env::var("HOSTNAME") {
        if !h.is_empty() {
            return h;
        }
    }
    Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn gather_rust_info() -> RustInfo {
    let rustc_version = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    // rustc --print cfg is more portable than target_triple env; but both
    // are fine here. Prefer the env var Cargo sets for the current build.
    let target_triple = std::env::var("CARGO_BUILD_TARGET").unwrap_or_else(|_| {
        Command::new("rustc")
            .args(["-vV"])
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .find(|l| l.starts_with("host:"))
                    .map(|l| l.trim_start_matches("host:").trim().to_string())
            })
            .unwrap_or_else(|| "unknown".to_string())
    });

    let profile = if cfg!(debug_assertions) {
        "dev".to_string()
    } else {
        "release".to_string()
    };
    RunInfoBuilder {
        rustc_version,
        target_triple,
        profile,
    }
    .build()
}

// Small helper so we can reuse fields by name without re-typing them
struct RunInfoBuilder {
    rustc_version: String,
    target_triple: String,
    profile: String,
}
impl RunInfoBuilder {
    fn build(self) -> RustInfo {
        RustInfo {
            rustc_version: self.rustc_version,
            target_triple: self.target_triple,
            profile: self.profile,
        }
    }
}

/// Short unique-ish suffix for the run id.
fn uuid_short() -> String {
    // Deterministic-per-process-start pseudo-UUID using timestamp + pid.
    // We don't need cryptographic uniqueness — just something that won't
    // collide for the same second on the same host.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let mix = ts.wrapping_mul(2862933555777941757).wrapping_add(pid);
    format!("{:08x}", (mix as u64) & 0xFFFF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_collect_has_timestamps_and_host() {
        let m = RunManifest::collect(Vec::new()).unwrap();
        assert_eq!(m.bench_count, 0);
        assert!(!m.run_id.is_empty());
        assert!(!m.host.os.is_empty());
        assert!(!m.host.arch.is_empty());
    }

    #[test]
    fn manifest_roundtrips_through_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.jsonl");

        let m = RunManifest::collect(Vec::new()).unwrap();
        m.append_to_jsonl(&path).unwrap();
        m.append_to_jsonl(&path).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        let _: RunManifest = serde_json::from_str(lines[0]).unwrap();
        let _: RunManifest = serde_json::from_str(lines[1]).unwrap();
    }

    #[test]
    fn manifest_writes_pretty_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("latest.json");
        let m = RunManifest::collect(Vec::new()).unwrap();
        m.write_json(&path).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\"run_id\""));
        assert!(contents.starts_with('{'));
    }

    #[test]
    fn run_ids_are_unique_per_call() {
        let a = RunManifest::collect(Vec::new()).unwrap();
        let b = RunManifest::collect(Vec::new()).unwrap();
        assert_ne!(a.run_id, b.run_id);
    }
}
