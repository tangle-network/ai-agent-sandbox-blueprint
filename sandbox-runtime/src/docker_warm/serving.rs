//! The warm-serving engine: seed / refill / claim / evict, plus the
//! process-wide handle and the production Docker host.
//!
//! Structure mirrors [`crate::firecracker_warm::serving`]:
//!
//! - the engine ([`DockerWarmServing`]) owns the ready pool
//!   (`Mutex<Vec<WarmContainer>>`), the seed throttle (`seeds_in_flight`), and
//!   the claim gate; it is generic over a [`DockerWarmHost`] so tests drive it
//!   with an in-memory fake and production wires the real bollard host;
//! - the CLAIM pops the entry out of `ready` **before any await** so two
//!   concurrent claims can never receive the same container;
//! - seeding runs in DETACHED tasks OUTSIDE the creation permit (refilling the
//!   pool must never serialize behind — the up-front memory reservation, not a
//!   per-seed budget check, keeps it fail-closed).

use super::*;

/// Docker container labels stamped at seed. `tangle.warm-pool` is the reconcile
/// filter; the other two are provenance/observability. Labels are create-time
/// immutable, so a claimed container keeps them — the reconcile guard keys on
/// "warm label AND no matching store record", which is why that is safe.
pub(crate) const WARM_POOL_LABEL: &str = "tangle.warm-pool";
pub(crate) const WARM_IMAGE_LABEL: &str = "tangle.warm-image";
pub(crate) const WARM_SEQ_LABEL: &str = "tangle.warm-seq";

/// Container-name prefix for an UNCLAIMED warm entry (`sidecar-warm-<seq>`). A
/// claim renames the container to `sidecar-<sandbox_id>`, so this prefix is the
/// structural signal that a container is still pooled and never handed to a
/// customer — the reconcile guard uses it so a claimed live sandbox can never
/// be a reap candidate even if the store is unreadable (see [`reconcile`]).
pub(crate) const WARM_NAME_PREFIX: &str = "sidecar-warm-";

/// Never seed more than this many containers concurrently, so a cold-start
/// burst does not create the entire pool at once (each seed is a ~700ms Docker
/// create + a `memory_mb` RAM spike). The pool still fills across subsequent
/// creates; the total in-flight can never exceed `pool_size` (the
/// `seeds_in_flight` compare-exchange guarantees that separately).
const MAX_CONCURRENT_SEEDS: usize = 2;

/// Health-wait budget while seeding a warm container (background, off the
/// request path).
const WARM_SEED_HEALTH_TIMEOUT_SECS: u64 = 60;

/// Health probe budget at claim. Short: the container was proven healthy at
/// seed, so a failure here means it died while idle — fail fast to cold.
const WARM_CLAIM_HEALTH_TIMEOUT_SECS: u64 = 5;

/// Everything a host needs to seed one warm container.
#[derive(Debug, Clone)]
pub(crate) struct WarmSeedSpec {
    pub seq: u64,
    /// Container name (`sidecar-warm-<seq>`).
    pub name: String,
    /// Random operator↔sidecar token baked into the env.
    pub token: String,
    pub image: String,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub base_env_json: String,
    pub capabilities_json: String,
}

/// Host-side Docker operations the engine delegates. Production wires
/// [`BollardDockerWarmHost`]; tests substitute an in-memory fake so the pool's
/// lifecycle (seed → refill → claim → evict) is exercised without a daemon.
#[async_trait]
pub(crate) trait DockerWarmHost: Send + Sync + 'static {
    /// Cold-create + start + bootstrap + health-prove a warm-labelled container
    /// on the pooled shape. Returns the container id once it is claimable.
    async fn seed_container(&self, spec: &WarmSeedSpec) -> Result<String>;

    /// Rename the pooled container onto `sidecar-<sandbox_id>`, read back its
    /// (already-assigned) host ports, and health-probe the sidecar.
    async fn claim_container(
        &self,
        container_id: &str,
        sandbox_id: &str,
    ) -> std::result::Result<ClaimResolved, ClaimFailure>;

    /// Force-remove a container (a failed/evicted/orphaned warm entry).
    async fn reap_container(&self, container_id: &str);
}

#[derive(Debug, Default)]
struct WarmCounters {
    claims: AtomicU64,
    misses: AtomicU64,
    seed_failures: AtomicU64,
}

/// The warm-serving engine.
pub(crate) struct DockerWarmServing {
    host: Arc<dyn DockerWarmHost>,
    settings: DockerWarmSettings,
    ready: Mutex<Vec<WarmContainer>>,
    seeds_in_flight: Arc<AtomicUsize>,
    seq: AtomicU64,
    counters: WarmCounters,
}

impl DockerWarmServing {
    pub(crate) fn new(host: Arc<dyn DockerWarmHost>, settings: DockerWarmSettings) -> Self {
        Self {
            host,
            settings,
            ready: Mutex::new(Vec::new()),
            seeds_in_flight: Arc::new(AtomicUsize::new(0)),
            seq: AtomicU64::new(0),
            counters: WarmCounters::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn ready_count(&self) -> usize {
        lock_ready(&self.ready).len()
    }

    #[cfg(test)]
    pub(crate) fn seeds_in_flight(&self) -> usize {
        self.seeds_in_flight.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn claims(&self) -> u64 {
        self.counters.claims.load(Ordering::Relaxed)
    }

    /// Top the pool back up to `pool_size`. Called on every create: cheap when
    /// saturated, and it doubles as the retry loop for failed seeds (no
    /// supervisor thread). Verbatim shape of
    /// [`crate::firecracker_warm::WarmServing::ensure_seeding`], plus a
    /// concurrent-seed cap and lazy age eviction.
    pub(crate) fn ensure_seeding(self: &Arc<Self>) {
        self.evict_over_age();
        loop {
            // Sample `ready` and `in_flight` TOGETHER under the ready lock. The
            // seed-completion path below publishes the container and clears its
            // in-flight slot inside this same lock, so a completing seed is
            // always visible in exactly one of the two counts — never neither
            // (which would let a concurrent create over-seed past pool_size and
            // exceed the up-front memory reservation) and never both.
            let (ready, in_flight) = {
                let guard = lock_ready(&self.ready);
                (guard.len(), self.seeds_in_flight.load(Ordering::SeqCst))
            };
            if ready + in_flight >= self.settings.pool_size {
                return;
            }
            // Bound the concurrent seed burst.
            if in_flight >= MAX_CONCURRENT_SEEDS.min(self.settings.pool_size) {
                return;
            }
            // Reserve the slot before spawning so concurrent creates cannot
            // over-seed past pool_size.
            if self
                .seeds_in_flight
                .compare_exchange(in_flight, in_flight + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                continue;
            }
            let this = Arc::clone(self);
            tokio::spawn(async move {
                match this.seed_one().await {
                    Ok(container) => {
                        // Publish the container and release the in-flight slot as
                        // ONE critical section (the fetch_sub is a non-blocking
                        // atomic, so no await is held under the lock). This makes
                        // the (push, decrement) pair atomic against
                        // `ensure_seeding`'s paired read above.
                        let mut guard = lock_ready(&this.ready);
                        guard.push(container);
                        this.seeds_in_flight.fetch_sub(1, Ordering::SeqCst);
                    }
                    Err(err) => {
                        this.seeds_in_flight.fetch_sub(1, Ordering::SeqCst);
                        this.counters.seed_failures.fetch_add(1, Ordering::Relaxed);
                        tracing::error!(%err, "docker warm-pool container seed failed");
                    }
                }
            });
        }
    }

    async fn seed_one(&self) -> Result<WarmContainer> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let token = crate::auth::generate_token();
        let spec = WarmSeedSpec {
            seq,
            name: format!("{WARM_NAME_PREFIX}{seq}"),
            token: token.clone(),
            image: self.settings.image.clone(),
            cpu_cores: self.settings.cpu_cores,
            memory_mb: self.settings.memory_mb,
            base_env_json: self.settings.base_env_json.clone(),
            capabilities_json: self.settings.capabilities_json.clone(),
        };
        let container_id = self.host.seed_container(&spec).await?;
        tracing::info!(
            warm_seq = seq,
            "docker warm-pool container seeded and ready"
        );
        Ok(WarmContainer {
            container_id,
            token,
            seq,
            created_at: crate::util::now_ts(),
        })
    }

    /// Reap pooled containers older than the configured max age, then let
    /// [`ensure_seeding`](Self::ensure_seeding) refill. Reaps run detached; the
    /// ready lock is dropped before any await (a std Mutex guard across await is
    /// both a clippy denial and a deadlock risk).
    fn evict_over_age(&self) {
        let max_age = self.settings.entry_max_age.as_secs();
        if max_age == 0 {
            return;
        }
        let now = crate::util::now_ts();
        let evicted: Vec<WarmContainer> = {
            let mut ready = lock_ready(&self.ready);
            let (keep, evict): (Vec<WarmContainer>, Vec<WarmContainer>) =
                std::mem::take(&mut *ready)
                    .into_iter()
                    .partition(|w| now.saturating_sub(w.created_at) < max_age);
            *ready = keep;
            evict
        };
        for w in evicted {
            tracing::info!(
                container_id = %w.container_id,
                warm_seq = w.seq,
                "docker warm-pool evicting over-age container"
            );
            let host = Arc::clone(&self.host);
            tokio::spawn(async move { host.reap_container(&w.container_id).await });
        }
    }

    /// Try to serve `request` from the pool. Never cold-falls-back internally —
    /// the caller owns the (logged) fallback.
    pub(crate) async fn claim(&self, request: &DockerWarmClaimRequest) -> DockerWarmOutcome {
        if let Some(miss) = self.shape_gate(request) {
            return self.miss(miss);
        }

        // Pop a ready container synchronously BEFORE any await so a concurrent
        // claim cannot double-serve it. On a downstream failure the container
        // was possibly already renamed and cannot return to the pool — it is
        // reaped, not pushed back.
        let warm = {
            let mut ready = lock_ready(&self.ready);
            match ready.pop() {
                Some(w) => w,
                None => {
                    drop(ready);
                    let reason = if self.seeds_in_flight.load(Ordering::SeqCst) > 0 {
                        DockerWarmMiss::NotReady
                    } else {
                        DockerWarmMiss::Empty
                    };
                    return self.miss(reason);
                }
            }
        };

        match self
            .host
            .claim_container(&warm.container_id, &request.sandbox_id)
            .await
        {
            Ok(resolved) => {
                self.counters.claims.fetch_add(1, Ordering::Relaxed);
                tracing::info!(
                    sandbox_id = %request.sandbox_id,
                    container_id = %warm.container_id,
                    warm_seq = warm.seq,
                    "docker warm-pool claim served (warm container renamed onto sandbox id)"
                );
                DockerWarmOutcome::Claimed(DockerWarmClaim {
                    container_id: warm.container_id,
                    token: warm.token,
                    sidecar_url: resolved.sidecar_url,
                    sidecar_port: resolved.sidecar_port,
                    ssh_port: resolved.ssh_port,
                    extra_ports: resolved.extra_ports,
                })
            }
            Err(failure) => {
                self.host.reap_container(&warm.container_id).await;
                self.miss(DockerWarmMiss::from_claim_failure(failure))
            }
        }
    }

    /// `None` = request matches the pooled shape. Every branch is a distinct
    /// typed miss so the create path can log exactly why it fell to cold.
    pub(crate) fn shape_gate(&self, request: &DockerWarmClaimRequest) -> Option<DockerWarmMiss> {
        if request.image.trim() != self.settings.image.trim() {
            return Some(DockerWarmMiss::ImageMismatch {
                requested: request.image.clone(),
                pooled: self.settings.image.clone(),
            });
        }
        if request.ssh_enabled {
            return Some(DockerWarmMiss::SshRequested);
        }
        if has_user_env(&request.user_env_json) {
            return Some(DockerWarmMiss::UserEnvPresent);
        }
        if request.extra_ports_len > 0 {
            return Some(DockerWarmMiss::ExtraPortsRequested);
        }
        if request.cpu_cores != 0 && request.cpu_cores != self.settings.cpu_cores {
            return Some(DockerWarmMiss::CpuMismatch {
                requested: request.cpu_cores,
                pooled: self.settings.cpu_cores,
            });
        }
        if request.memory_mb != 0 && request.memory_mb != self.settings.memory_mb {
            return Some(DockerWarmMiss::MemoryMismatch {
                requested: request.memory_mb,
                pooled: self.settings.memory_mb,
            });
        }
        if !env_json_matches(&request.env_json, &self.settings.base_env_json) {
            return Some(DockerWarmMiss::BaseEnvMismatch);
        }
        if crate::runtime::parse_sidecar_capabilities(&request.capabilities_json)
            != crate::runtime::parse_sidecar_capabilities(&self.settings.capabilities_json)
        {
            return Some(DockerWarmMiss::CapabilitiesMismatch);
        }
        None
    }

    fn miss(&self, miss: DockerWarmMiss) -> DockerWarmOutcome {
        self.counters.misses.fetch_add(1, Ordering::Relaxed);
        DockerWarmOutcome::Miss(miss)
    }
}

/// A request carries user env if its `user_env_json` is a non-empty, non-`{}`
/// object — mirrors [`crate::SandboxRecord::has_user_secrets`].
fn has_user_env(user_env_json: &str) -> bool {
    let s = user_env_json.trim();
    !s.is_empty() && s != "{}"
}

/// Structural equality of two base-env JSON strings (order-independent; empty
/// parses to `{}`). Unparseable input never silently matches — it is treated as
/// a mismatch so a malformed base env falls to the cold path.
fn env_json_matches(a: &str, b: &str) -> bool {
    fn normalized(s: &str) -> Option<serde_json::Value> {
        let t = s.trim();
        if t.is_empty() {
            return Some(serde_json::json!({}));
        }
        serde_json::from_str::<serde_json::Value>(t).ok()
    }
    match (normalized(a), normalized(b)) {
        (Some(va), Some(vb)) => va == vb,
        _ => false,
    }
}

fn lock_ready(m: &Mutex<Vec<WarmContainer>>) -> std::sync::MutexGuard<'_, Vec<WarmContainer>> {
    match m.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Process-wide handle + entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Process-wide warm-serving engine. Unset until the first Docker create with
/// `SANDBOX_DOCKER_WARM_POOL_SIZE > 0`. A `tokio::sync::OnceCell` (async init)
/// rather than the Firecracker pool's `OnceLock` because the Docker restart
/// reconcile is async (bollard `list_containers`) — the Firecracker analogue is
/// a synchronous `/proc` scan.
static DOCKER_WARM: tokio::sync::OnceCell<Arc<DockerWarmServing>> =
    tokio::sync::OnceCell::const_new();

/// Try to serve a Docker create from the warm pool.
///
/// MUST only be called from the Docker create arm, which the runtime dispatches
/// AFTER `admit_sandbox_resources` (count cap + host memory budget, under the
/// creation permit). That is what makes a warm claim count against
/// `SANDBOX_MAX_COUNT` / the host budget exactly like a cold boot — pool
/// inventory itself is never admission-accounted (its standing RAM is reserved
/// via [`reserved_host_memory_mb`]).
///
/// A hard configuration error (unparseable pool size, or the pool enabled with
/// no memory value) propagates — it must fail the create loudly, never silently
/// fall through to cold.
pub(crate) async fn claim_docker_warm(
    request: &CreateSandboxParams,
    sandbox_id: &str,
) -> Result<DockerWarmOutcome> {
    let settings = match load_settings()? {
        Some(s) => s,
        None => return Ok(DockerWarmOutcome::Miss(DockerWarmMiss::Disabled)),
    };

    let serving = DOCKER_WARM
        .get_or_init(|| async move {
            // Reap warm containers orphaned by a previous operator process
            // BEFORE the first seed (mirrors firecracker/warm.rs). Best-effort:
            // a Docker/reconcile failure is logged, never blocks pool init.
            match crate::runtime::docker_builder().await {
                Ok(builder) => reconcile_docker_warm_orphans(&builder).await,
                Err(err) => tracing::warn!(
                    %err,
                    "docker warm-pool: startup reconcile skipped (Docker connect failed)"
                ),
            }
            Arc::new(DockerWarmServing::new(
                Arc::new(BollardDockerWarmHost),
                settings,
            ))
        })
        .await;

    serving.ensure_seeding();

    let config = SidecarRuntimeConfig::load();
    let effective_image = if request.image.trim().is_empty() {
        config.image.clone()
    } else {
        request.image.clone()
    };
    let claim_req = DockerWarmClaimRequest {
        sandbox_id: sandbox_id.to_string(),
        image: effective_image,
        cpu_cores: request.cpu_cores,
        memory_mb: request.memory_mb,
        ssh_enabled: request.ssh_enabled,
        env_json: request.env_json.clone(),
        user_env_json: request.user_env_json.clone(),
        capabilities_json: request.capabilities_json.clone(),
        extra_ports_len: crate::runtime::parse_extra_ports(
            &request.metadata_json,
            &request.port_mappings,
        )
        .len(),
    };
    Ok(serving.claim(&claim_req).await)
}

// ─────────────────────────────────────────────────────────────────────────────
// Production host — real bollard I/O
// ─────────────────────────────────────────────────────────────────────────────

/// The production [`DockerWarmHost`]: fresh Docker client per operation (never
/// cached process-wide, matching `docker_builder`'s stale-socket rationale).
pub(crate) struct BollardDockerWarmHost;

#[async_trait]
impl DockerWarmHost for BollardDockerWarmHost {
    async fn seed_container(&self, spec: &WarmSeedSpec) -> Result<String> {
        let config = SidecarRuntimeConfig::load();
        let builder = crate::runtime::docker_builder().await?;

        // Env baked identically to the cold path (build_env_vars), carrying the
        // warm token — a random operator↔sidecar secret copied verbatim into
        // the store record at claim, so no post-create mutation is needed.
        let env_vars = crate::runtime::build_env_vars(
            &spec.base_env_json,
            &spec.token,
            config.container_port,
            &spec.capabilities_json,
        )?;

        let mut labels = std::collections::HashMap::new();
        labels.insert(WARM_POOL_LABEL.to_string(), "1".to_string());
        labels.insert(WARM_IMAGE_LABEL.to_string(), spec.image.clone());
        labels.insert(WARM_SEQ_LABEL.to_string(), spec.seq.to_string());

        // SSH disabled + no extra ports = the warm default shape.
        let override_config = crate::runtime::build_docker_config(
            config,
            false,
            spec.cpu_cores,
            spec.memory_mb,
            Some(labels),
            &[],
        );

        let mut container = Container::new(builder.client(), spec.image.clone())
            .with_name(spec.name.clone())
            .env(env_vars)
            .config_override(override_config);

        // Create + start — the ~700ms + ~200ms pre-paid off the request path.
        if let Err(err) =
            crate::runtime::docker_timeout("warm_create_container", container.create()).await
        {
            tracing::debug!(error = %err, "warm container create failed; start path will retry it");
        }
        crate::runtime::start_container_with_retry(&mut container).await?;

        let container_id = container
            .id()
            .ok_or_else(|| SandboxError::Docker("warm container missing id after start".into()))?
            .to_string();

        // On any post-start failure, reap the half-seeded container so it does
        // not leak (it is not yet in the ready pool).
        let seeded = async {
            // Pre-pay the workspace bootstrap exec (chown + mkdir .opencode-home).
            crate::runtime::run_workspace_bootstrap(&builder.client(), &container_id, &spec.name)
                .await;

            // Prove the pooled sidecar is live before it can ever be claimed.
            let (sidecar_url, _port, _ssh, _extra) =
                crate::runtime::refresh_port_mapping_with_retry(
                    "warm seed endpoint",
                    builder.client(),
                    &container_id,
                    config.container_port,
                    false,
                    &config.public_host,
                    &std::collections::HashMap::new(),
                )
                .await?;
            if !crate::runtime::wait_for_sidecar_health(&sidecar_url, WARM_SEED_HEALTH_TIMEOUT_SECS)
                .await
            {
                return Err(SandboxError::Unavailable(format!(
                    "warm container {container_id} sidecar at {sidecar_url} never became healthy"
                )));
            }
            Ok(())
        }
        .await;

        if let Err(err) = seeded {
            self.reap_container(&container_id).await;
            return Err(err);
        }
        Ok(container_id)
    }

    async fn claim_container(
        &self,
        container_id: &str,
        sandbox_id: &str,
    ) -> std::result::Result<ClaimResolved, ClaimFailure> {
        let config = SidecarRuntimeConfig::load();
        let builder = crate::runtime::docker_builder()
            .await
            .map_err(|e| ClaimFailure::Rename(e.to_string()))?;

        // Rename by id (stable, unambiguous) onto the real sandbox id. No
        // recreate — the container keeps its baked env, token, and host ports.
        let new_name = format!("sidecar-{sandbox_id}");
        crate::runtime::docker_timeout(
            "warm_rename_container",
            builder.client().rename_container(
                container_id,
                RenameContainerOptions {
                    name: new_name.clone(),
                },
            ),
        )
        .await
        .map_err(|e| ClaimFailure::Rename(e.to_string()))?;

        // Read back the already-assigned host ports (container is already
        // started, so this resolves immediately — no start latency).
        let (sidecar_url, sidecar_port, ssh_port, extra_ports) =
            crate::runtime::refresh_port_mapping_with_retry(
                "warm claim endpoint",
                builder.client(),
                container_id,
                config.container_port,
                false,
                &config.public_host,
                &std::collections::HashMap::new(),
            )
            .await
            .map_err(|e| ClaimFailure::PortResolve(e.to_string()))?;

        // A container idle for minutes could have a dead sidecar behind a live
        // process; prove it before handing it out.
        if !crate::runtime::wait_for_sidecar_health(&sidecar_url, WARM_CLAIM_HEALTH_TIMEOUT_SECS)
            .await
        {
            return Err(ClaimFailure::Unhealthy(format!(
                "sidecar at {sidecar_url} not healthy within {WARM_CLAIM_HEALTH_TIMEOUT_SECS}s"
            )));
        }

        Ok(ClaimResolved {
            sidecar_url,
            sidecar_port,
            ssh_port,
            extra_ports,
        })
    }

    async fn reap_container(&self, container_id: &str) {
        match crate::runtime::docker_builder().await {
            Ok(builder) => {
                if let Ok(c) = Container::from_id(builder.client(), container_id).await {
                    let _ = c
                        .remove(Some(RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }))
                        .await;
                }
            }
            Err(err) => tracing::warn!(
                container_id,
                %err,
                "docker warm-pool reap: Docker connect failed"
            ),
        }
    }
}
