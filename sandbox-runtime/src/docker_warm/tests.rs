//! Unit + engine tests for the Docker warm pool. All run without Docker: the
//! engine is driven by an in-memory [`FakeDockerWarmHost`], config parsing is
//! exercised under the process-wide env guard, and the reap decision + shape
//! gate are pure. The real-Docker warm-hit is proven by the env-gated
//! `tests/lifecycle_bench.rs`.

use super::*;
use std::collections::{HashMap, HashSet};

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────────

fn test_settings(pool_size: usize) -> DockerWarmSettings {
    DockerWarmSettings {
        pool_size,
        image: "test:latest".to_string(),
        cpu_cores: 2,
        memory_mb: 2048,
        base_env_json: String::new(),
        capabilities_json: String::new(),
        entry_max_age: Duration::from_secs(3600),
    }
}

/// A request that matches [`test_settings`]'s pooled shape.
fn matching_req() -> DockerWarmClaimRequest {
    DockerWarmClaimRequest {
        sandbox_id: "sb-test".to_string(),
        image: "test:latest".to_string(),
        cpu_cores: 0,
        memory_mb: 0,
        ssh_enabled: false,
        env_json: String::new(),
        user_env_json: String::new(),
        capabilities_json: String::new(),
        extra_ports_len: 0,
    }
}

/// In-memory host: seeds return synthetic ids, claims resolve a dummy endpoint,
/// reaps are recorded. `fail_claim` drives the reap-on-claim-failure path.
#[derive(Default)]
struct FakeDockerWarmHost {
    seeded_ids: Mutex<Vec<String>>,
    claimed_ids: Mutex<Vec<String>>,
    reaped_ids: Mutex<Vec<String>>,
    fail_claim: bool,
}

#[async_trait]
impl DockerWarmHost for FakeDockerWarmHost {
    async fn seed_container(&self, spec: &WarmSeedSpec) -> Result<String> {
        let id = format!("fake-container-{}", spec.seq);
        self.seeded_ids.lock().unwrap().push(id.clone());
        Ok(id)
    }

    async fn claim_container(
        &self,
        container_id: &str,
        sandbox_id: &str,
    ) -> std::result::Result<ClaimResolved, ClaimFailure> {
        if self.fail_claim {
            return Err(ClaimFailure::Unhealthy(
                "injected claim failure".to_string(),
            ));
        }
        self.claimed_ids
            .lock()
            .unwrap()
            .push(container_id.to_string());
        Ok(ClaimResolved {
            sidecar_url: format!("http://127.0.0.1:0/{sandbox_id}"),
            sidecar_port: 8080,
            ssh_port: None,
            extra_ports: HashMap::new(),
        })
    }

    async fn reap_container(&self, container_id: &str) {
        self.reaped_ids
            .lock()
            .unwrap()
            .push(container_id.to_string());
    }
}

/// Drive `ensure_seeding` (called per-create in production) until the pool
/// reaches `target`, matching how the pool fills across successive creates.
async fn fill_pool(serving: &Arc<DockerWarmServing>, target: usize) -> bool {
    for _ in 0..400 {
        serving.ensure_seeding();
        if serving.ready_count() >= target {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    serving.ready_count() >= target
}

fn engine(host: Arc<FakeDockerWarmHost>, pool_size: usize) -> Arc<DockerWarmServing> {
    Arc::new(DockerWarmServing::new(host, test_settings(pool_size)))
}

// ─────────────────────────────────────────────────────────────────────────────
// Shape gate
// ─────────────────────────────────────────────────────────────────────────────

fn gate(req: DockerWarmClaimRequest) -> Option<DockerWarmMiss> {
    let serving = engine(Arc::new(FakeDockerWarmHost::default()), 1);
    serving.shape_gate(&req)
}

#[test]
fn shape_gate_accepts_default_shape() {
    assert!(gate(matching_req()).is_none());
}

#[test]
fn shape_gate_accepts_empty_object_base_env() {
    // "" and "{}" both normalize to an empty object → match.
    let mut req = matching_req();
    req.env_json = "{}".to_string();
    assert!(gate(req).is_none());
}

#[test]
fn shape_gate_accepts_exact_cpu_and_memory() {
    let mut req = matching_req();
    req.cpu_cores = 2; // == pooled
    req.memory_mb = 2048; // == pooled
    assert!(gate(req).is_none());
}

#[test]
fn shape_gate_rejects_image_mismatch() {
    let mut req = matching_req();
    req.image = "other:latest".to_string();
    assert!(matches!(
        gate(req),
        Some(DockerWarmMiss::ImageMismatch { .. })
    ));
}

#[test]
fn shape_gate_rejects_ssh() {
    let mut req = matching_req();
    req.ssh_enabled = true;
    assert!(matches!(gate(req), Some(DockerWarmMiss::SshRequested)));
}

#[test]
fn shape_gate_rejects_user_env() {
    let mut req = matching_req();
    req.user_env_json = r#"{"SECRET":"x"}"#.to_string();
    assert!(matches!(gate(req), Some(DockerWarmMiss::UserEnvPresent)));
}

#[test]
fn shape_gate_rejects_extra_ports() {
    let mut req = matching_req();
    req.extra_ports_len = 1;
    assert!(matches!(
        gate(req),
        Some(DockerWarmMiss::ExtraPortsRequested)
    ));
}

#[test]
fn shape_gate_rejects_cpu_mismatch() {
    let mut req = matching_req();
    req.cpu_cores = 8; // pooled is 2
    assert!(matches!(
        gate(req),
        Some(DockerWarmMiss::CpuMismatch { .. })
    ));
}

#[test]
fn shape_gate_rejects_memory_mismatch() {
    let mut req = matching_req();
    req.memory_mb = 4096; // pooled is 2048
    assert!(matches!(
        gate(req),
        Some(DockerWarmMiss::MemoryMismatch { .. })
    ));
}

#[test]
fn shape_gate_rejects_base_env_mismatch() {
    let mut req = matching_req();
    req.env_json = r#"{"A":"1"}"#.to_string(); // pooled base is empty
    assert!(matches!(gate(req), Some(DockerWarmMiss::BaseEnvMismatch)));
}

#[test]
fn shape_gate_rejects_capabilities_mismatch() {
    let mut req = matching_req();
    req.capabilities_json = r#"["computer_use"]"#.to_string(); // pooled has none
    assert!(matches!(
        gate(req),
        Some(DockerWarmMiss::CapabilitiesMismatch)
    ));
}

#[test]
fn every_miss_variant_has_a_distinct_display() {
    // Guards against a Display arm silently sharing another's text.
    let variants = [
        DockerWarmMiss::Disabled,
        DockerWarmMiss::NotReady,
        DockerWarmMiss::Empty,
        DockerWarmMiss::ImageMismatch {
            requested: "a".into(),
            pooled: "b".into(),
        },
        DockerWarmMiss::CpuMismatch {
            requested: 1,
            pooled: 2,
        },
        DockerWarmMiss::MemoryMismatch {
            requested: 1,
            pooled: 2,
        },
        DockerWarmMiss::SshRequested,
        DockerWarmMiss::UserEnvPresent,
        DockerWarmMiss::BaseEnvMismatch,
        DockerWarmMiss::CapabilitiesMismatch,
        DockerWarmMiss::ExtraPortsRequested,
        DockerWarmMiss::RenameFailed("e".into()),
        DockerWarmMiss::PortResolveFailed("e".into()),
        DockerWarmMiss::Unhealthy("e".into()),
    ];
    let rendered: HashSet<String> = variants.iter().map(|m| m.to_string()).collect();
    assert_eq!(
        rendered.len(),
        variants.len(),
        "Display strings must be distinct"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Concurrency / claim race — the pop-before-await atomicity guarantee
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_claims_never_double_serve() {
    let pool_size = 3;
    let fake = Arc::new(FakeDockerWarmHost::default());
    let serving = engine(fake.clone(), pool_size);

    assert!(
        fill_pool(&serving, pool_size).await,
        "pool never filled to {pool_size}"
    );
    // Never over-seeds past pool_size (the seeds_in_flight compare-exchange).
    assert_eq!(serving.ready_count(), pool_size);
    assert_eq!(fake.seeded_ids.lock().unwrap().len(), pool_size);
    assert_eq!(serving.seeds_in_flight(), 0);

    // Fire M > N concurrent claims.
    let m = 8usize;
    let mut handles = Vec::new();
    for i in 0..m {
        let s = Arc::clone(&serving);
        handles.push(tokio::spawn(async move {
            let mut req = matching_req();
            req.sandbox_id = format!("sb-{i}");
            s.claim(&req).await
        }));
    }

    let mut claimed_ids = Vec::new();
    let mut misses = 0usize;
    for h in handles {
        match h.await.unwrap() {
            DockerWarmOutcome::Claimed(c) => claimed_ids.push(c.container_id),
            DockerWarmOutcome::Miss(_) => misses += 1,
        }
    }

    assert_eq!(claimed_ids.len(), pool_size, "exactly pool_size claims win");
    assert_eq!(misses, m - pool_size, "the rest miss");
    let unique: HashSet<_> = claimed_ids.iter().cloned().collect();
    assert_eq!(
        unique.len(),
        claimed_ids.len(),
        "no container id was handed to two claims"
    );
    assert_eq!(
        fake.claimed_ids.lock().unwrap().len(),
        pool_size,
        "host saw exactly pool_size claim_container calls"
    );
    assert_eq!(serving.ready_count(), 0, "pool drained");
    assert_eq!(serving.claims(), pool_size as u64);
}

#[tokio::test]
async fn claim_reuses_the_seeded_container_id() {
    let fake = Arc::new(FakeDockerWarmHost::default());
    let serving = engine(fake.clone(), 1);
    assert!(fill_pool(&serving, 1).await);
    let seeded = fake.seeded_ids.lock().unwrap().clone();
    assert_eq!(seeded.len(), 1);

    let out = serving.claim(&matching_req()).await;
    match out {
        DockerWarmOutcome::Claimed(claim) => {
            // The claim reuses the seeded container (rename, not recreate).
            assert_eq!(claim.container_id, seeded[0]);
            // The baked token flows through unchanged (it is inside the
            // container's immutable env; a fresh token would not authenticate).
            assert!(!claim.token.is_empty());
        }
        DockerWarmOutcome::Miss(m) => panic!("expected a claim, got miss: {m}"),
    }
    assert_eq!(
        fake.claimed_ids.lock().unwrap().as_slice(),
        &[seeded[0].clone()]
    );
}

#[tokio::test]
async fn empty_pool_misses_without_touching_host() {
    let fake = Arc::new(FakeDockerWarmHost::default());
    // pool_size 0 would be Disabled at a higher layer; here we exercise the
    // engine directly with a non-zero size but an unfilled pool.
    let serving = engine(fake.clone(), 1);
    let out = serving.claim(&matching_req()).await;
    // Nothing ready and (in this direct test) nothing seeding yet.
    assert!(matches!(
        out,
        DockerWarmOutcome::Miss(DockerWarmMiss::Empty | DockerWarmMiss::NotReady)
    ));
    assert!(fake.claimed_ids.lock().unwrap().is_empty());
}

#[tokio::test]
async fn claim_failure_reaps_and_misses() {
    let fake = Arc::new(FakeDockerWarmHost {
        fail_claim: true,
        ..Default::default()
    });
    let serving = engine(fake.clone(), 1);
    assert!(fill_pool(&serving, 1).await);

    let out = serving.claim(&matching_req()).await;
    assert!(
        matches!(out, DockerWarmOutcome::Miss(DockerWarmMiss::Unhealthy(_))),
        "a downstream claim failure maps to a typed miss"
    );
    assert_eq!(
        fake.reaped_ids.lock().unwrap().len(),
        1,
        "the popped-but-failed container is reaped, not returned to the pool"
    );
    assert_eq!(
        serving.ready_count(),
        0,
        "the container was popped, not left"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Restart reconcile — the reap decision (data-loss guard)
// ─────────────────────────────────────────────────────────────────────────────

/// Helper: a warm-labelled listing with an explicit name.
fn listing(id: &str, name: &str) -> WarmContainerListing {
    WarmContainerListing {
        id: id.into(),
        name: name.into(),
    }
}

#[test]
fn reconcile_reaps_name_prefixed_orphans_and_leaves_claimed() {
    let listings = vec![
        listing("orphan-1", "sidecar-warm-1"), // never claimed
        listing("claimed", "sidecar-abc-123"), // renamed by a claim → not reaped
        listing("orphan-2", "sidecar-warm-2"),
    ];
    let mut reap = containers_to_reap(&listings);
    reap.sort();
    assert_eq!(
        reap,
        vec!["orphan-1".to_string(), "orphan-2".to_string()],
        "name-prefixed orphans reaped, the renamed (claimed) container left untouched"
    );
}

#[test]
fn reconcile_leaves_everything_when_none_are_name_prefixed() {
    // All renamed (claimed) — none carry the `sidecar-warm-` prefix.
    let listings = vec![listing("a", "sidecar-alpha"), listing("b", "sidecar-beta")];
    assert!(containers_to_reap(&listings).is_empty());
}

/// The data-loss guard is purely structural: a claimed container (renamed to
/// `sidecar-<id>`) is NEVER reaped, no matter the store state. This covers the
/// catastrophic path — startup after a crash with a corrupt/unreadable
/// `sandboxes.json` (which loads as an empty map, not an error) — that an
/// id-in-store guard would have gotten wrong by reaping the live sandbox.
#[test]
fn reconcile_never_reaps_renamed_container_regardless_of_store() {
    let listings = vec![
        listing("orphan", "sidecar-warm-7"), // pooled, safe to reap by name
        listing("live-customer", "sidecar-xyz-999"), // a running claimed sandbox
    ];
    let reap = containers_to_reap(&listings);
    assert_eq!(
        reap,
        vec!["orphan".to_string()],
        "reap only the name-prefixed orphan; the renamed live sandbox is never a candidate"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Config parsing + host-memory reservation (env-guarded)
// ─────────────────────────────────────────────────────────────────────────────

/// Run `body` with the given warm-pool env applied under the process-wide env
/// guard, then restore. The config helpers read env live (no caching), so this
/// is race-free as long as every env-mutating test holds `TEST_ENV_GUARD`.
fn with_env<F: FnOnce()>(vars: &[(&str, Option<&str>)], body: F) {
    let _guard = crate::TEST_ENV_GUARD
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let keys = [
        "SANDBOX_DOCKER_WARM_POOL_SIZE",
        "SANDBOX_DOCKER_WARM_MEMORY_MB",
        "SANDBOX_DOCKER_WARM_CPU_CORES",
        "SANDBOX_DOCKER_WARM_MAX_AGE_SECS",
    ];
    // SAFETY: env mutation is serialized by TEST_ENV_GUARD; the config helpers
    // read env synchronously within `body` while the guard is held.
    unsafe {
        for k in keys {
            std::env::remove_var(k);
        }
        for (k, v) in vars {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }
    body();
    unsafe {
        for k in keys {
            std::env::remove_var(k);
        }
    }
}

#[test]
fn pool_size_absent_or_zero_is_disabled() {
    with_env(&[("SANDBOX_DOCKER_WARM_POOL_SIZE", None)], || {
        assert_eq!(configured_pool_size().unwrap(), 0);
    });
    with_env(&[("SANDBOX_DOCKER_WARM_POOL_SIZE", Some(""))], || {
        assert_eq!(configured_pool_size().unwrap(), 0);
    });
    with_env(&[("SANDBOX_DOCKER_WARM_POOL_SIZE", Some("0"))], || {
        assert_eq!(configured_pool_size().unwrap(), 0);
    });
}

#[test]
fn pool_size_parses_and_rejects_garbage() {
    with_env(&[("SANDBOX_DOCKER_WARM_POOL_SIZE", Some("4"))], || {
        assert_eq!(configured_pool_size().unwrap(), 4);
    });
    with_env(&[("SANDBOX_DOCKER_WARM_POOL_SIZE", Some("x"))], || {
        assert!(matches!(
            configured_pool_size(),
            Err(SandboxError::Validation(_))
        ));
    });
}

#[test]
fn memory_required_when_enabled() {
    // Enabled but no memory → hard error (never a silent 0).
    with_env(
        &[
            ("SANDBOX_DOCKER_WARM_POOL_SIZE", Some("2")),
            ("SANDBOX_DOCKER_WARM_MEMORY_MB", None),
        ],
        || {
            assert!(matches!(
                configured_warm_memory_mb(2),
                Err(SandboxError::Validation(_))
            ));
        },
    );
    // Disabled → 0, no error.
    with_env(&[], || {
        assert_eq!(configured_warm_memory_mb(0).unwrap(), 0);
    });
    // Enabled + set → parsed.
    with_env(&[("SANDBOX_DOCKER_WARM_MEMORY_MB", Some("2048"))], || {
        assert_eq!(configured_warm_memory_mb(2).unwrap(), 2048);
    });
    // Enabled + explicit 0 → hard error.
    with_env(&[("SANDBOX_DOCKER_WARM_MEMORY_MB", Some("0"))], || {
        assert!(matches!(
            configured_warm_memory_mb(2),
            Err(SandboxError::Validation(_))
        ));
    });
}

#[test]
fn reserved_host_memory_is_pool_times_memory_factor_one() {
    // Disabled → 0.
    with_env(&[("SANDBOX_DOCKER_WARM_POOL_SIZE", None)], || {
        assert_eq!(reserved_host_memory_mb().unwrap(), 0);
    });
    // 4 × 2048 = 8192 (factor 1, distinct from Firecracker's factor 2).
    with_env(
        &[
            ("SANDBOX_DOCKER_WARM_POOL_SIZE", Some("4")),
            ("SANDBOX_DOCKER_WARM_MEMORY_MB", Some("2048")),
        ],
        || {
            assert_eq!(reserved_host_memory_mb().unwrap(), 8192);
        },
    );
    // Enabled without memory → error (fail-closed, never a silent under-reserve).
    with_env(
        &[
            ("SANDBOX_DOCKER_WARM_POOL_SIZE", Some("2")),
            ("SANDBOX_DOCKER_WARM_MEMORY_MB", None),
        ],
        || {
            assert!(reserved_host_memory_mb().is_err());
        },
    );
}

#[test]
fn entry_max_age_defaults_and_parses() {
    with_env(&[("SANDBOX_DOCKER_WARM_MAX_AGE_SECS", None)], || {
        assert_eq!(
            configured_entry_max_age().unwrap(),
            Duration::from_secs(3600)
        );
    });
    with_env(&[("SANDBOX_DOCKER_WARM_MAX_AGE_SECS", Some("60"))], || {
        assert_eq!(configured_entry_max_age().unwrap(), Duration::from_secs(60));
    });
    with_env(
        &[("SANDBOX_DOCKER_WARM_MAX_AGE_SECS", Some("nope"))],
        || {
            assert!(configured_entry_max_age().is_err());
        },
    );
}

#[test]
fn cpu_defaults_zero_and_parses() {
    with_env(&[("SANDBOX_DOCKER_WARM_CPU_CORES", None)], || {
        assert_eq!(configured_warm_cpu_cores().unwrap(), 0);
    });
    with_env(&[("SANDBOX_DOCKER_WARM_CPU_CORES", Some("4"))], || {
        assert_eq!(configured_warm_cpu_cores().unwrap(), 4);
    });
}
