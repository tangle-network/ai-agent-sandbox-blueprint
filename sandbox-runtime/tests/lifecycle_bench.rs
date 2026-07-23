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

/// Serializes the process-env mutation in `setup_env`. See the note at its
/// acquisition site for why this is a local static rather than the crate's
/// feature-gated `TEST_ENV_GUARD`.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    {
        // Serialize env mutation, matching this dir's convention for
        // env-mutating tests. We use a local static (like ssh_e2e's
        // TEST_LOCK) rather than sandbox_runtime::TEST_ENV_GUARD because that
        // symbol is gated behind the `test-utils` feature, which this bench
        // does not enable — it runs in the default (no-feature) nextest lane.
        //
        // Guard scoped to the env mutation only (NOT held across awaits — a
        // std Mutex guard across an await is a clippy denial and a deadlock
        // risk): this binary has a single #[ignore] test, so nothing else
        // reads the env before the first SidecarRuntimeConfig::load() inside
        // the loop below. A future second test in this binary must acquire the
        // same lock around its own setup_env to stay race-free.
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        setup_env(&state_dir, &image);
    }

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

    // NOTE: delete_sidecar force-removes the Docker container but does NOT
    // remove the store row (consistent with the operator's own delete path),
    // so the isolated store accumulates reps+1 stale records over the run.
    // Harmless while reps+1 stays under SANDBOX_MAX_COUNT (default 100); a
    // user setting LIFECYCLE_BENCH_REPS near/over that cap would start hitting
    // admission rejection on later reps for a reason unrelated to the bench.
    //
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
            .unwrap_or_else(|e| {
                panic!(
                    "rep {rep}{}: create failed: {e}",
                    if warmup { " (warmup)" } else { "" }
                )
            });

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

/// The docker warm-pool proof-of-win: measure `create_to_ready` for the cold
/// path (pool disabled) vs a warm hit (pool pre-seeded), in one process.
///
/// A warm hit is identified structurally — `timings.warm_claim.is_some()` and
/// the `container_create` / `container_start` / `bootstrap_exec` stages are
/// `None` because they were pre-paid by the background refill. The pool shape is
/// set to exactly match [`bench_params`] (cpu 2, mem 2048, default image, no
/// user env, no extra ports, ssh disabled) so the create qualifies.
///
/// ```bash
/// docker pull ghcr.io/tangle-network/blueprint-sidecar:all-harness
/// RUN_LIFECYCLE_BENCH=1 cargo test -p sandbox-runtime --test lifecycle_bench \
///     warm_vs_cold -- --ignored --nocapture
/// ```
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "real-docker warm-pool bench; set RUN_LIFECYCLE_BENCH=1 and run with --ignored on a host with Docker"]
async fn warm_vs_cold_create_ready_bench() {
    if !bench_enabled() {
        eprintln!("SKIP: set RUN_LIFECYCLE_BENCH=1 to run the warm-vs-cold bench");
        return;
    }
    assert!(
        docker_available(),
        "RUN_LIFECYCLE_BENCH=1 but no reachable Docker daemon (`docker info` failed)"
    );
    let image = bench_image();
    assert!(
        image_present(&image),
        "RUN_LIFECYCLE_BENCH=1 but image {image} is not local; run `docker pull {image}`"
    );

    let state_dir = TempDir::new().expect("temp state dir");
    {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        setup_env(&state_dir, &image);
        // Start with the pool disabled for the cold baseline.
        // SAFETY: single test in its own binary section, guarded by ENV_LOCK.
        unsafe {
            std::env::remove_var("SANDBOX_DOCKER_WARM_POOL_SIZE");
        }
    }

    let reps = bench_reps();
    reap_warm_leftovers();

    // ── Phase A: cold baseline (pool disabled) ──
    eprintln!("=== warm-vs-cold: image={image} reps={reps} — phase A (cold) ===");
    let mut cold = StageSeries::new("cold_create_to_ready");
    for rep in 0..=reps {
        let warmup = rep == 0;
        let params = bench_params(&image, 1000 + rep);
        let rep_start = Instant::now();
        let (record, _a, timings) = create_sidecar_timed(&params, None)
            .await
            .unwrap_or_else(|e| panic!("cold rep {rep}: create failed: {e}"));
        let ready = wait_for_sidecar_health(&record.sidecar_url, 120).await;
        let ready_elapsed = rep_start.elapsed();
        let _ = delete_sidecar(&record, None).await;
        assert!(ready, "cold rep {rep}: sidecar never healthy");
        assert!(
            timings.warm_claim.is_none(),
            "cold rep {rep}: pool disabled but a warm claim fired"
        );
        eprintln!(
            "cold rep {rep}{}: create_to_ready={:.1}ms [{}]",
            if warmup { " (warmup, discarded)" } else { "" },
            ready_elapsed.as_secs_f64() * 1e3,
            timings.summary(),
        );
        if !warmup {
            cold.push(Some(ready_elapsed));
        }
    }

    // ── Phase B: warm hits (pool enabled, shape matches bench_params) ──
    {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: guarded by ENV_LOCK; the warm-pool config helpers read env
        // live on the next create.
        unsafe {
            std::env::set_var("SANDBOX_DOCKER_WARM_POOL_SIZE", "2");
            std::env::set_var("SANDBOX_DOCKER_WARM_MEMORY_MB", "2048");
            std::env::set_var("SANDBOX_DOCKER_WARM_CPU_CORES", "2");
            std::env::set_var("SANDBOX_DOCKER_WARM_IMAGE", &image);
        }
    }
    eprintln!("=== warm-vs-cold: phase B (warm) — priming pool ===");

    let mut warm = StageSeries::new("warm_create_to_ready");
    let mut warm_claim = StageSeries::new("warm_claim");
    let mut warm_total = StageSeries::new("warm_create_total");
    let mut warm_health = StageSeries::new("warm_health_ready");
    let mut collected = 0usize;
    let max_attempts = reps * 6 + 24;
    for attempt in 0..max_attempts {
        if collected >= reps {
            break;
        }
        // Let the background refill land (~1s create + bootstrap + health).
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let params = bench_params(&image, 2000 + attempt);
        let rep_start = Instant::now();
        let (record, _a, timings) = create_sidecar_timed(&params, None)
            .await
            .unwrap_or_else(|e| panic!("warm attempt {attempt}: create failed: {e}"));
        let health_start = Instant::now();
        let ready = wait_for_sidecar_health(&record.sidecar_url, 120).await;
        let health_elapsed = health_start.elapsed();
        let ready_elapsed = rep_start.elapsed();
        let _ = delete_sidecar(&record, None).await;
        assert!(ready, "warm attempt {attempt}: sidecar never healthy");

        let hit = timings.warm_claim.is_some();
        eprintln!(
            "warm attempt {attempt}: {} create_to_ready={:.1}ms [{}]",
            if hit { "HIT " } else { "miss" },
            ready_elapsed.as_secs_f64() * 1e3,
            timings.summary(),
        );
        if hit {
            warm.push(Some(ready_elapsed));
            warm_claim.push(timings.warm_claim);
            warm_total.push(Some(timings.total));
            warm_health.push(Some(health_elapsed));
            collected += 1;
        }
    }

    reap_warm_leftovers();

    assert!(
        collected > 0,
        "no warm hit observed after {max_attempts} attempts — the pool never served a claim"
    );
    print_summaries(&[cold, warm, warm_total, warm_claim, warm_health]);
    eprintln!(
        "warm hits collected: {collected}/{reps}. A warm hit erases container_create + \
         container_start + bootstrap_exec from the request path (they are pre-paid by refill)."
    );
}

/// Force-remove any warm-pool containers this bench left running (the pool
/// seeds up to `pool_size` containers that are never claimed). Keyed on the
/// `tangle.warm-pool` label so it never touches a real sandbox.
fn reap_warm_leftovers() {
    let ids = Command::new("docker")
        .args(["ps", "-aq", "--filter", "label=tangle.warm-pool=1"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for id in ids {
        let _ = Command::new("docker").args(["rm", "-f", &id]).output();
    }
}
