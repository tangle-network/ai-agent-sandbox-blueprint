//! Create→ready→delete lifecycle benchmark for the sandbox runtime's REAL
//! Docker create path.
//!
//! Measures `create_sidecar_timed` (admission → Docker connect → image check
//! → container create/start → port mapping → bootstrap execs → store insert)
//! with per-stage [`CreateTimings`], then the create→ready gap, then delete.
//!
//! ## Relationship to the existing lifecycle bench
//!
//! `ai-agent-sandbox-blueprint-lib/tests/lifecycle_bench.rs` measures the
//! SIDECAR's HTTP surface (health/auth/exec/terminal ops) against a single
//! container it provisions by hand with raw bollard — it never enters
//! `create_sidecar`. This bench covers the complementary half: the runtime's
//! own provisioning loop, per rep and per stage.
//!
//! ## "Ready" definition
//!
//! `create_sidecar` returns BEFORE the sidecar's `/health` endpoint passes
//! unless `ssh_enabled` forces the SSH bootstrap. This bench therefore polls
//! `wait_for_sidecar_health` itself after each create and reports
//! `health_ready` (create-return → first /health 200) as its own stage, and
//! `create_to_ready` as the caller-visible end-to-end number.
//!
//! ## Running
//!
//! Needs a live Docker daemon and the sidecar image already pulled. It is
//! `#[ignore]`d AND env-gated, so no CI lane without Docker ever runs it —
//! it runs on a real host, on demand:
//!
//! ```bash
//! docker pull ghcr.io/tangle-network/blueprint-sidecar:all-harness
//! RUN_LIFECYCLE_BENCH=1 cargo test -p sandbox-runtime --test lifecycle_bench \
//!     -- --ignored --nocapture
//! ```
//!
//! Knobs: `LIFECYCLE_BENCH_REPS` (default 5, + 1 discarded warmup rep),
//! `LIFECYCLE_BENCH_IMAGE` (default the all-harness sidecar image).
//!
//! State isolation: `BLUEPRINT_STATE_DIR` points at a fresh temp dir for the
//! whole run, so the bench never reads or corrupts an operator's real store.

use std::process::Command;
use std::time::{Duration, Instant};

use bench_harness::stats::Summary;
use tempfile::TempDir;

use sandbox_runtime::runtime::{
    CreateSandboxParams, create_sidecar_timed, delete_sidecar, wait_for_sidecar_health,
};

const DEFAULT_IMAGE: &str = "ghcr.io/tangle-network/blueprint-sidecar:all-harness";

fn bench_enabled() -> bool {
    std::env::var("RUN_LIFECYCLE_BENCH")
        .map(|v| v == "1")
        .unwrap_or(false)
}

fn bench_reps() -> usize {
    std::env::var("LIFECYCLE_BENCH_REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5)
}

fn bench_image() -> String {
    std::env::var("LIFECYCLE_BENCH_IMAGE").unwrap_or_else(|_| DEFAULT_IMAGE.to_string())
}

fn docker_available() -> bool {
    Command::new("docker")
        .args(["info"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn image_present(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn setup_env(state_dir: &TempDir, image: &str) {
    // SAFETY: this is the only test in this binary (own process), and the env
    // is set before the first `SidecarRuntimeConfig::load()` / store access.
    unsafe {
        std::env::set_var("BLUEPRINT_STATE_DIR", state_dir.path());
        std::env::set_var("SIDECAR_IMAGE", image);
        // Keep registry pulls out of the measured loop: the image must be
        // local (checked above), so `image_pull` measures only the local
        // "already present" fast path.
        std::env::set_var("SIDECAR_PULL_IMAGE", "false");
        std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
        std::env::set_var("REQUEST_TIMEOUT_SECS", "60");
        std::env::set_var("SESSION_AUTH_SECRET", "lifecycle-bench-secret");
        std::env::remove_var("DOCKER_HOST");
    }
}

fn bench_params(image: &str, rep: usize) -> CreateSandboxParams {
    CreateSandboxParams {
        name: format!("lifecycle-bench-{rep}"),
        image: image.to_string(),
        stack: "default".to_string(),
        agent_identifier: "default".to_string(),
        metadata_json: r#"{"runtime_backend":"docker"}"#.to_string(),
        // ssh_enabled=false is the common product path AND the one where
        // create returns before /health — exactly the gap this bench exists
        // to make visible.
        ssh_enabled: false,
        max_lifetime_seconds: 3600,
        idle_timeout_seconds: 3600,
        cpu_cores: 2,
        memory_mb: 2048,
        disk_gb: 10,
        owner: "0x9965507d1a55bcc2695c58ba16fb37d819b0a4dc".to_string(),
        ..Default::default()
    }
}

/// One named series of per-rep samples (milliseconds).
struct StageSeries {
    name: &'static str,
    samples_ms: Vec<f64>,
}

impl StageSeries {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            samples_ms: Vec::new(),
        }
    }

    fn push(&mut self, duration: Option<Duration>) {
        if let Some(d) = duration {
            self.samples_ms.push(d.as_secs_f64() * 1e3);
        }
    }
}

fn print_summaries(series: &[StageSeries]) {
    eprintln!();
    eprintln!(
        "{:<18} {:>3} {:>10} {:>10} {:>10} {:>10}",
        "stage", "n", "min(ms)", "p50(ms)", "p90(ms)", "max(ms)"
    );
    for s in series {
        if s.samples_ms.is_empty() {
            eprintln!("{:<18} {:>3} (no samples)", s.name, 0);
            continue;
        }
        // bench-harness works in nanoseconds; feed it ns, print ms.
        let ns: Vec<f64> = s.samples_ms.iter().map(|ms| ms * 1e6).collect();
        let summary = Summary::from_samples(&ns);
        eprintln!(
            "{:<18} {:>3} {:>10.1} {:>10.1} {:>10.1} {:>10.1}",
            s.name,
            summary.n,
            summary.min_ns / 1e6,
            summary.p50_ns / 1e6,
            summary.p90_ns / 1e6,
            summary.max_ns / 1e6,
        );
    }
    eprintln!();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "real-docker lifecycle bench; set RUN_LIFECYCLE_BENCH=1 and run with --ignored on a host with Docker"]
async fn lifecycle_create_ready_delete_bench() {
    if !bench_enabled() {
        eprintln!("SKIP: set RUN_LIFECYCLE_BENCH=1 to run the lifecycle bench");
        return;
    }
    // The bench was explicitly requested — missing prerequisites are loud
    // failures, not silent skips.
    assert!(
        docker_available(),
        "RUN_LIFECYCLE_BENCH=1 but no reachable Docker daemon (`docker info` failed)"
    );
    let image = bench_image();
    assert!(
        image_present(&image),
        "RUN_LIFECYCLE_BENCH=1 but image {image} is not local; run `docker pull {image}` \
         (or set LIFECYCLE_BENCH_IMAGE)"
    );

    let state_dir = TempDir::new().expect("temp state dir");
    setup_env(&state_dir, &image);

    let reps = bench_reps();
    let mut create_total = StageSeries::new("create_total");
    let mut permit_wait = StageSeries::new("permit_wait");
    let mut admission = StageSeries::new("admission");
    let mut docker_connect = StageSeries::new("docker_connect");
    let mut image_pull = StageSeries::new("image_pull");
    let mut container_create = StageSeries::new("container_create");
    let mut container_start = StageSeries::new("container_start");
    let mut port_mapping = StageSeries::new("port_mapping");
    let mut bootstrap_exec = StageSeries::new("bootstrap_exec");
    let mut store_insert = StageSeries::new("store_insert");
    let mut health_ready = StageSeries::new("health_ready");
    let mut create_to_ready = StageSeries::new("create_to_ready");
    let mut delete = StageSeries::new("delete");

    // Warmup rep (discarded): primes the process-wide image-pull once-cell,
    // the HTTP client, and the Docker daemon's caches so measured reps
    // reflect steady state. Cold-start cost is visible in its own log line.
    eprintln!("=== lifecycle bench: image={image} reps={reps} (+1 warmup) ===");
    for rep in 0..=reps {
        let warmup = rep == 0;
        let params = bench_params(&image, rep);

        let rep_start = Instant::now();
        let (record, _attestation, timings) = create_sidecar_timed(&params, None)
            .await
            .unwrap_or_else(|e| panic!("rep {rep}: create failed: {e}"));

        let health_start = Instant::now();
        let ready = wait_for_sidecar_health(&record.sidecar_url, 120).await;
        let health_elapsed = health_start.elapsed();
        let ready_elapsed = rep_start.elapsed();

        let delete_start = Instant::now();
        let delete_result = delete_sidecar(&record, None).await;
        let delete_elapsed = delete_start.elapsed();

        // Assert AFTER cleanup so a failed rep never leaks a container.
        assert!(
            ready,
            "rep {rep}: sidecar at {} never became healthy within 120s",
            record.sidecar_url
        );
        delete_result.unwrap_or_else(|e| panic!("rep {rep}: delete failed: {e}"));

        eprintln!(
            "rep {rep}{}: create={:.1}ms [{}] health_ready={:.1}ms create_to_ready={:.1}ms delete={:.1}ms",
            if warmup { " (warmup, discarded)" } else { "" },
            timings.total.as_secs_f64() * 1e3,
            timings.summary(),
            health_elapsed.as_secs_f64() * 1e3,
            ready_elapsed.as_secs_f64() * 1e3,
            delete_elapsed.as_secs_f64() * 1e3,
        );
        if warmup {
            continue;
        }

        create_total.push(Some(timings.total));
        permit_wait.push(timings.permit_wait);
        admission.push(timings.admission);
        docker_connect.push(timings.docker_connect);
        image_pull.push(timings.image_pull);
        container_create.push(timings.container_create);
        container_start.push(timings.container_start);
        port_mapping.push(timings.port_mapping);
        bootstrap_exec.push(timings.bootstrap_exec);
        store_insert.push(timings.store_insert);
        health_ready.push(Some(health_elapsed));
        create_to_ready.push(Some(ready_elapsed));
        delete.push(Some(delete_elapsed));
    }

    print_summaries(&[
        create_total,
        permit_wait,
        admission,
        docker_connect,
        image_pull,
        container_create,
        container_start,
        port_mapping,
        bootstrap_exec,
        store_insert,
        health_ready,
        create_to_ready,
        delete,
    ]);
}
