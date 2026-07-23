//! The production [`DockerWarmHost`]: real bollard I/O for seeding, claiming,
//! and reaping warm containers. Split out of [`super::serving`] so the engine
//! (pool state + claim/refill/reconcile policy) stays separate from the concrete
//! Docker calls — mirroring [`crate::firecracker_warm`]'s `host.rs`.

use super::*;

/// Health-wait budget while seeding a warm container (background, off the
/// request path).
const WARM_SEED_HEALTH_TIMEOUT_SECS: u64 = 60;

/// Health probe budget at claim. Short: the container was proven healthy at
/// seed, so a failure here means it died while idle — fail fast to cold.
const WARM_CLAIM_HEALTH_TIMEOUT_SECS: u64 = 5;

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
