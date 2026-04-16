//! Real-infrastructure lifecycle benchmark.
//!
//! No mocks. Real Docker container. Real sidecar. Real HTTP. Real timing.
//!
//! Measures actual wall-clock time for every product-surface operation that a
//! customer performs, from the moment the HTTP request leaves the client to
//! the moment the response is fully received.
//!
//! ## Running
//!
//! ```bash
//! REAL_SIDECAR=1 LIFECYCLE_BENCH=1 cargo test -p ai-agent-sandbox-blueprint-lib \
//!     --test lifecycle_bench -- --test-threads=1 --nocapture
//! ```
//!
//! Increase runs: `LIFECYCLE_N=10` (default: 5)
//!
//! ## Output
//!
//! Writes `bench-results/lifecycle-real.json` with per-operation stats:
//! mean, p50, p95, p99, stddev, CI, outliers, throughput.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use docktopus::DockerBuilder;
use docktopus::bollard::container::{
    Config as BollardConfig, InspectContainerOptions, RemoveContainerOptions,
};
use docktopus::bollard::models::{HostConfig, PortBinding, PortMap};
use docktopus::container::Container;
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::OnceCell;

// ---------------------------------------------------------------------------
// Gate
// ---------------------------------------------------------------------------

fn should_run() -> bool {
    std::env::var("REAL_SIDECAR")
        .map(|v| v == "1")
        .unwrap_or(false)
        && std::env::var("LIFECYCLE_BENCH")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
}

fn run_count() -> usize {
    std::env::var("LIFECYCLE_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5)
}

macro_rules! skip_unless {
    () => {
        if !should_run() {
            eprintln!("Skipped (set REAL_SIDECAR=1 LIFECYCLE_BENCH=1 to enable)");
            return;
        }
    };
}

// ---------------------------------------------------------------------------
// Real sidecar setup (same pattern as real_sidecar.rs — no mocks)
// ---------------------------------------------------------------------------

const AUTH_TOKEN: &str = "lifecycle-bench-token-9a3c7e";
const CONTAINER_NAME: &str = "lifecycle-bench-sidecar";
const CONTAINER_PORT: u16 = 8080;

struct RealSidecar {
    url: String,
    #[allow(dead_code)]
    container_id: String,
    /// Time it took to create + start + become healthy.
    provision_time: Duration,
}

static SIDECAR: OnceCell<RealSidecar> = OnceCell::const_new();

async fn docker_builder() -> DockerBuilder {
    match DockerBuilder::new().await {
        Ok(b) => b,
        Err(_) => {
            let home = std::env::var("HOME").unwrap_or_default();
            let mac_sock = format!("unix://{home}/.docker/run/docker.sock");
            DockerBuilder::with_address(&mac_sock)
                .await
                .expect("Docker daemon not reachable")
        }
    }
}

async fn extract_host_port(builder: &DockerBuilder, container_id: &str) -> u16 {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let inspect = builder
            .client()
            .inspect_container(container_id, None::<InspectContainerOptions>)
            .await
            .expect("inspect container");
        if let Some(port) = inspect
            .network_settings
            .as_ref()
            .and_then(|ns| ns.ports.as_ref())
            .and_then(|p| p.get(&format!("{CONTAINER_PORT}/tcp")))
            .and_then(|v| v.as_ref())
            .and_then(|v| v.first())
            .and_then(|b| b.host_port.as_ref())
            .and_then(|p| p.parse::<u16>().ok())
            .filter(|p| *p > 0)
        {
            return port;
        }
        assert!(tokio::time::Instant::now() <= deadline, "no host port");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Provision a real Docker sidecar. Times the full cycle:
/// image exists check → container create → start → health-check polling → ready.
async fn ensure_sidecar() -> &'static RealSidecar {
    SIDECAR
        .get_or_init(|| async {
            let image = std::env::var("SIDECAR_IMAGE")
                .unwrap_or_else(|_| "tangle-sidecar:local".to_string());

            let provision_start = Instant::now();

            let builder = docker_builder().await;

            // Clean up leftover container from crashed run
            let _ = builder
                .client()
                .remove_container(
                    CONTAINER_NAME,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await;

            let mut port_bindings = PortMap::new();
            port_bindings.insert(
                format!("{CONTAINER_PORT}/tcp"),
                Some(vec![PortBinding {
                    host_ip: Some("0.0.0.0".to_string()),
                    host_port: None,
                }]),
            );

            let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();
            exposed_ports.insert(format!("{CONTAINER_PORT}/tcp"), HashMap::new());

            let env_vars = vec![
                format!("SIDECAR_PORT={CONTAINER_PORT}"),
                format!("SIDECAR_AUTH_TOKEN={AUTH_TOKEN}"),
                "NODE_ENV=development".to_string(),
                "PORT_WATCHER_ENABLED=false".to_string(),
            ];

            let override_config = BollardConfig {
                exposed_ports: Some(exposed_ports),
                host_config: Some(HostConfig {
                    port_bindings: Some(port_bindings),
                    ..Default::default()
                }),
                ..Default::default()
            };

            let mut container = Container::new(builder.client(), image.clone())
                .with_name(CONTAINER_NAME.to_string())
                .env(env_vars)
                .config_override(override_config);

            container
                .start(false)
                .await
                .unwrap_or_else(|e| panic!("Failed to start sidecar ({image}): {e}"));

            let container_id = container.id().expect("no container ID").to_string();

            let host_port = extract_host_port(&builder, &container_id).await;
            let url = format!("http://127.0.0.1:{host_port}");

            // Wait for healthy
            let client = Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
            loop {
                if tokio::time::Instant::now() > deadline {
                    panic!("Sidecar not healthy within 60s at {url}");
                }
                match client.get(format!("{url}/health")).send().await {
                    Ok(resp) if resp.status().is_success() => break,
                    _ => {}
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            let provision_time = provision_start.elapsed();
            eprintln!(
                "sidecar ready in {:.2}s (container={}, port={})",
                provision_time.as_secs_f64(),
                &container_id[..12],
                host_port
            );

            RealSidecar {
                url,
                container_id,
                provision_time,
            }
        })
        .await
}

fn http() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

fn auth() -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {AUTH_TOKEN}")).unwrap()
}

// ---------------------------------------------------------------------------
// Stats (reuse bench-harness stats via computation, not crate dep)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpStats {
    n: usize,
    mean_ms: f64,
    median_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    min_ms: f64,
    max_ms: f64,
    stddev_ms: f64,
    ci_lower_ms: f64,
    ci_upper_ms: f64,
    samples_ms: Vec<f64>,
}

fn compute_stats(samples_ms: &[f64]) -> OpStats {
    let n = samples_ms.len();
    assert!(n > 0);
    let mut sorted: Vec<f64> = samples_ms.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let sum: f64 = sorted.iter().sum();
    let mean = sum / n as f64;
    let variance = if n > 1 {
        sorted.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0)
    } else {
        0.0
    };
    let stddev = variance.sqrt();
    let pct = |p: f64| -> f64 {
        let rank = p * (n as f64 - 1.0);
        let lo = rank.floor() as usize;
        let hi = rank.ceil() as usize;
        if lo == hi {
            sorted[lo]
        } else {
            sorted[lo] * (1.0 - rank.fract()) + sorted[hi] * rank.fract()
        }
    };
    // Basic bootstrap CI
    let (ci_lower, ci_upper) = if n >= 3 {
        let se = stddev / (n as f64).sqrt();
        (mean - 1.96 * se, mean + 1.96 * se)
    } else {
        (sorted[0], sorted[n - 1])
    };

    OpStats {
        n,
        mean_ms: mean,
        median_ms: pct(0.50),
        p95_ms: pct(0.95),
        p99_ms: pct(0.99),
        min_ms: sorted[0],
        max_ms: sorted[n - 1],
        stddev_ms: stddev,
        ci_lower_ms: ci_lower,
        ci_upper_ms: ci_upper,
        samples_ms: sorted,
    }
}

// ---------------------------------------------------------------------------
// Timing helper
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct OpResult {
    operation: String,
    stats: OpStats,
}

async fn time_op<F, Fut>(name: &str, n: usize, f: F) -> OpResult
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let start = Instant::now();
        f().await;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        samples.push(elapsed_ms);
        if n > 1 {
            eprintln!("  {name} [{}/{}]: {:.2}ms", i + 1, n, elapsed_ms);
        }
    }
    let stats = compute_stats(&samples);
    eprintln!(
        "  {name}: mean={:.2}ms p50={:.2}ms p99={:.2}ms CI=[{:.2},{:.2}]",
        stats.mean_ms, stats.median_ms, stats.p99_ms, stats.ci_lower_ms, stats.ci_upper_ms
    );
    OpResult {
        operation: name.to_string(),
        stats,
    }
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct LifecycleReport {
    timestamp: String,
    runs_per_op: usize,
    provision_time_ms: f64,
    operations: Vec<OpResult>,
}

fn write_report(report: &LifecycleReport) {
    let dir = std::path::Path::new("bench-results");
    std::fs::create_dir_all(dir).ok();
    let path = dir.join("lifecycle-real.json");
    let json = serde_json::to_string_pretty(report).unwrap();
    std::fs::write(&path, &json).unwrap();

    eprintln!();
    eprintln!(
        "┌─────────────────────────────────────────┬──────────┬──────────┬──────────┬──────────┐"
    );
    eprintln!(
        "│ Operation                               │ Mean     │ p50      │ p99      │ 95% CI   │"
    );
    eprintln!(
        "├─────────────────────────────────────────┼──────────┼──────────┼──────────┼──────────┤"
    );
    for op in &report.operations {
        let s = &op.stats;
        eprintln!(
            "│ {:<39} │ {:>6.1}ms │ {:>6.1}ms │ {:>6.1}ms │ ±{:.1}ms │",
            op.operation,
            s.mean_ms,
            s.median_ms,
            s.p99_ms,
            (s.ci_upper_ms - s.ci_lower_ms) / 2.0
        );
    }
    eprintln!(
        "└─────────────────────────────────────────┴──────────┴──────────┴──────────┴──────────┘"
    );
    eprintln!();
    eprintln!("provision time: {:.2}ms", report.provision_time_ms);
    eprintln!("report: {}", path.display());
}

// ---------------------------------------------------------------------------
// The benchmark
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lifecycle_bench() {
    skip_unless!();

    let n = run_count();
    eprintln!("\n=== Real Lifecycle Bench: N={n} ===\n");

    let sidecar = ensure_sidecar().await;
    let url = sidecar.url.clone();

    let mut results = Vec::new();

    // ── Health (real sidecar health check) ──

    results.push(
        time_op("health", n, || {
            let url = url.clone();
            async move {
                let resp = http().get(format!("{url}/health")).send().await.unwrap();
                assert!(resp.status().is_success());
                let _body: Value = resp.json().await.unwrap();
            }
        })
        .await,
    );

    // ── Auth rejection (real sidecar auth) ──

    results.push(
        time_op("auth_reject_missing_token", n, || {
            let url = url.clone();
            async move {
                let resp = http()
                    .post(format!("{url}/terminals/commands"))
                    .header(CONTENT_TYPE, "application/json")
                    .json(&json!({"command": "echo hi"}))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(resp.status(), 401);
            }
        })
        .await,
    );

    results.push(
        time_op("auth_reject_wrong_token", n, || {
            let url = url.clone();
            async move {
                let resp = http()
                    .post(format!("{url}/terminals/commands"))
                    .header(AUTHORIZATION, "Bearer wrong-token")
                    .header(CONTENT_TYPE, "application/json")
                    .json(&json!({"command": "echo hi"}))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(resp.status(), 403);
            }
        })
        .await,
    );

    // ── Exec: real command in real container ──

    results.push(
        time_op("exec_echo", n, || {
            let url = url.clone();
            async move {
                let resp = http()
                    .post(format!("{url}/terminals/commands"))
                    .header(AUTHORIZATION, auth())
                    .header(CONTENT_TYPE, "application/json")
                    .json(&json!({"command": "echo lifecycle-bench-ok", "timeout": 10000}))
                    .send()
                    .await
                    .unwrap();
                assert!(resp.status().is_success(), "exec_echo: {}", resp.status());
                let body: Value = resp.json().await.unwrap();
                let stdout = body
                    .pointer("/result/stdout")
                    .or_else(|| body.get("stdout"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                assert!(stdout.contains("lifecycle-bench-ok"), "stdout: {stdout}");
            }
        })
        .await,
    );

    results.push(
        time_op("exec_ls", n, || {
            let url = url.clone();
            async move {
                let resp = http()
                    .post(format!("{url}/terminals/commands"))
                    .header(AUTHORIZATION, auth())
                    .header(CONTENT_TYPE, "application/json")
                    .json(&json!({"command": "ls -la /", "timeout": 10000}))
                    .send()
                    .await
                    .unwrap();
                assert!(resp.status().is_success());
            }
        })
        .await,
    );

    results.push(time_op("exec_write_read_file", n, || {
        let url = url.clone();
        async move {
            // Write
            let resp = http()
                .post(format!("{url}/terminals/commands"))
                .header(AUTHORIZATION, auth())
                .header(CONTENT_TYPE, "application/json")
                .json(&json!({"command": "echo bench-data-$(date +%s%N) > /tmp/bench-test.txt && cat /tmp/bench-test.txt", "timeout": 10000}))
                .send().await.unwrap();
            assert!(resp.status().is_success());
            let body: Value = resp.json().await.unwrap();
            let stdout = body.pointer("/result/stdout")
                .or_else(|| body.get("stdout"))
                .and_then(Value::as_str)
                .unwrap_or("");
            assert!(stdout.contains("bench-data-"), "file content: {stdout}");
        }
    }).await);

    results.push(
        time_op("exec_env_vars", n, || {
            let url = url.clone();
            async move {
                let resp = http()
                    .post(format!("{url}/terminals/commands"))
                    .header(AUTHORIZATION, auth())
                    .header(CONTENT_TYPE, "application/json")
                    .json(&json!({"command": "env | wc -l", "timeout": 10000}))
                    .send()
                    .await
                    .unwrap();
                assert!(resp.status().is_success());
            }
        })
        .await,
    );

    // ── Terminal session lifecycle ──

    results.push(
        time_op("terminal_create_destroy", n, || {
            let url = url.clone();
            async move {
                // Create
                let resp = http()
                    .post(format!("{url}/terminals"))
                    .header(AUTHORIZATION, auth())
                    .header(CONTENT_TYPE, "application/json")
                    .json(&json!({}))
                    .send()
                    .await
                    .unwrap();
                assert!(
                    resp.status().is_success(),
                    "terminal create: {}",
                    resp.status()
                );
                let body: Value = resp.json().await.unwrap();
                let session_id = body
                    .pointer("/data/sessionId")
                    .or_else(|| body.get("sessionId"))
                    .and_then(Value::as_str)
                    .expect("terminal session ID");

                // Delete
                let resp = http()
                    .delete(format!("{url}/terminals/{session_id}"))
                    .header(AUTHORIZATION, auth())
                    .send()
                    .await
                    .unwrap();
                assert!(
                    resp.status().is_success() || resp.status() == 204,
                    "terminal delete: {}",
                    resp.status()
                );
            }
        })
        .await,
    );

    // ── Terminal list ──

    results.push(
        time_op("terminal_list", n, || {
            let url = url.clone();
            async move {
                let resp = http()
                    .get(format!("{url}/terminals"))
                    .header(AUTHORIZATION, auth())
                    .send()
                    .await
                    .unwrap();
                assert!(resp.status().is_success());
                let _body: Value = resp.json().await.unwrap();
            }
        })
        .await,
    );

    // ── Agents listing (real sidecar agents) ──

    results.push(
        time_op("agents_list", n, || {
            let url = url.clone();
            async move {
                let resp = http()
                    .get(format!("{url}/agents"))
                    .header(AUTHORIZATION, auth())
                    .send()
                    .await
                    .unwrap();
                assert!(resp.status().is_success());
                let _body: Value = resp.json().await.unwrap();
            }
        })
        .await,
    );

    // ── Health detailed ──

    results.push(
        time_op("health_detailed", n, || {
            let url = url.clone();
            async move {
                let resp = http()
                    .get(format!("{url}/health/detailed"))
                    .header(AUTHORIZATION, auth())
                    .send()
                    .await
                    .unwrap();
                assert!(resp.status().is_success());
                let _body: Value = resp.json().await.unwrap();
            }
        })
        .await,
    );

    // ── Sequential exec burst (measures sidecar under load) ──

    {
        let burst_n = 10;
        let start = Instant::now();
        for i in 0..burst_n {
            let resp = http()
                .post(format!("{url}/terminals/commands"))
                .header(AUTHORIZATION, auth())
                .header(CONTENT_TYPE, "application/json")
                .json(&json!({"command": format!("echo burst-{i}"), "timeout": 5000}))
                .send()
                .await
                .unwrap();
            assert!(resp.status().is_success());
        }
        let total_ms = start.elapsed().as_secs_f64() * 1000.0;
        let per_op_ms = total_ms / burst_n as f64;
        eprintln!(
            "  exec_burst_10: total={:.1}ms, per-op={:.1}ms, throughput={:.0}/s",
            total_ms,
            per_op_ms,
            1000.0 / per_op_ms
        );
        results.push(OpResult {
            operation: "exec_burst_10_sequential".to_string(),
            stats: OpStats {
                n: 1,
                mean_ms: per_op_ms,
                median_ms: per_op_ms,
                p95_ms: per_op_ms,
                p99_ms: per_op_ms,
                min_ms: per_op_ms,
                max_ms: per_op_ms,
                stddev_ms: 0.0,
                ci_lower_ms: per_op_ms,
                ci_upper_ms: per_op_ms,
                samples_ms: vec![per_op_ms],
            },
        });
    }

    let report = LifecycleReport {
        timestamp: format!(
            "{}Z",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ),
        runs_per_op: n,
        provision_time_ms: sidecar.provision_time.as_secs_f64() * 1000.0,
        operations: results,
    };

    write_report(&report);

    // Cleanup
    let builder = docker_builder().await;
    let _ = builder
        .client()
        .remove_container(
            CONTAINER_NAME,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
}
