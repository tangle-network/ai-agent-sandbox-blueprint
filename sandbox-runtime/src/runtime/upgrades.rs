use super::*;

/// Recreate a sidecar container with updated user environment variables.
///
/// Stops and removes the old container, creates a new one with the
/// base env preserved and the provided `user_env_json` merged on top.
/// All other settings (image, CPU, memory, lifetime, token, agent identifier,
/// metadata, etc.) are faithfully preserved from the existing record.
///
/// Pass an empty string to clear user secrets (base env only).
///
/// Returns the new [`SandboxRecord`] for the recreated container.
/// The sidecar image the operator is currently configured to run
/// (`SIDECAR_IMAGE` env, falling back to the build-time default). This is the
/// target for fleet image upgrades — see [`upgrade_sidecar_image`].
#[must_use]
pub fn current_sidecar_image() -> String {
    env::var("SIDECAR_IMAGE").unwrap_or_else(|_| DEFAULT_SIDECAR_IMAGE.to_string())
}

/// List sandboxes whose container was created from an image other than the
/// operator's current `SIDECAR_IMAGE` — i.e. they're running a stale sidecar and
/// would benefit from an in-place image upgrade. Returns `(sandbox_id, original_image)`.
/// TEE sandboxes are excluded (their image can't be swapped without breaking
/// attestation). This is how an operator detects post-deploy image drift without
/// shelling into Docker.
pub fn sandboxes_needing_image_upgrade() -> Result<Vec<(String, String)>> {
    let target = current_sidecar_image();
    Ok(sandboxes()?
        .values()?
        .into_iter()
        .filter(|r| r.tee_deployment_id.is_none() && r.original_image != target)
        .map(|r| (r.id, r.original_image))
        .collect())
}

/// Sidecar image upgrade policy, read from `SIDECAR_UPGRADE_POLICY`.
/// Mirrors the on-chain binary `UpgradePolicy` one layer down: the blueprint
/// manager swaps the operator *binary* per its on-chain policy; the freshly
/// booted binary then reconciles its *sidecars* per this policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarUpgradePolicy {
    /// Roll drifted sandboxes onto the current image automatically (default).
    Auto,
    /// Only report drift — an operator triggers the upgrade explicitly
    /// (`POST /api/operator/sidecar-image/upgrade-stale`).
    Manual,
}

impl SidecarUpgradePolicy {
    #[must_use]
    pub fn from_env() -> Self {
        match env::var("SIDECAR_UPGRADE_POLICY").ok().as_deref() {
            Some(v) if v.eq_ignore_ascii_case("manual") => Self::Manual,
            _ => Self::Auto,
        }
    }
}

/// Outcome of [`reconcile_sidecar_images`].
#[derive(Debug, Default)]
pub struct SidecarReconcileReport {
    pub target_image: String,
    pub upgraded: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub pending: Vec<String>,
}

/// Reconcile every running sandbox onto the operator's current `SIDECAR_IMAGE`.
///
/// This is the cascade that makes sidecar upgrades "just happen" off the
/// manager's binary upgrade: the blueprint binary calls this at startup, so when
/// the manager swaps the operator binary (the on-chain BinaryVersion CD loop) the
/// new binary boots and rolls any stale sidecars forward — no manual step. Under
/// `Auto` it upgrades drifted sandboxes; under `Manual` it only records what's
/// pending (the operator triggers the upgrade endpoint). Drift detection means a
/// no-op when nothing changed, so calling it on every boot is safe.
pub async fn reconcile_sidecar_images(
    policy: SidecarUpgradePolicy,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<SidecarReconcileReport> {
    let mut report = SidecarReconcileReport {
        target_image: current_sidecar_image(),
        ..Default::default()
    };
    let stale = sandboxes_needing_image_upgrade()?;
    if stale.is_empty() {
        return Ok(report);
    }
    if policy == SidecarUpgradePolicy::Manual {
        tracing::warn!(
            count = stale.len(),
            target = %report.target_image,
            "Sidecar image drift detected; SIDECAR_UPGRADE_POLICY=manual — not auto-upgrading. \
             Trigger POST /api/operator/sidecar-image/upgrade-stale to roll them."
        );
        report.pending = stale.into_iter().map(|(id, _)| id).collect();
        return Ok(report);
    }
    tracing::info!(
        count = stale.len(),
        target = %report.target_image,
        "Sidecar image drift detected; auto-upgrading stale sandboxes to current image"
    );
    for (id, from_image) in stale {
        let _lock = acquire_lifecycle_lock(&id).await;
        match upgrade_sidecar_image(&id, &report.target_image, tee).await {
            Ok(_) => {
                tracing::info!(sandbox = %id, from = %from_image, to = %report.target_image, "sidecar upgraded");
                report.upgraded.push(id);
            }
            Err(e) => {
                tracing::error!(sandbox = %id, error = %e, "sidecar image upgrade failed");
                report.failed.push((id, e.to_string()));
            }
        }
    }
    Ok(report)
}

/// Recreate a sandbox onto the operator's current `SIDECAR_IMAGE`, preserving
/// the bot's secrets/env, token, ports, capabilities, and identity.
///
/// This is the clean fleet-upgrade primitive: when the operator ships a new
/// sidecar image (security patch, new agent harness, opencode bump), existing
/// sandboxes stay pinned to their *birth* image forever unless explicitly
/// migrated — which silently rots a fleet (e.g. agent runs failing on an old
/// image that never had opencode). Call this per sandbox (or over
/// [`sandboxes_needing_image_upgrade`]) to roll them forward in place.
///
/// Secrets are preserved: `get_sandbox_by_id` unseals the record, so the
/// existing `user_env_json` is replayed verbatim — no re-entry of API keys.
pub async fn upgrade_sidecar_image(
    sandbox_id: &str,
    target_image: &str,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<SandboxRecord> {
    let old = get_sandbox_by_id(sandbox_id)?;
    let preserved_user_env = old.user_env_json.clone();
    recreate_sidecar_impl(sandbox_id, &preserved_user_env, Some(target_image), tee).await
}

pub async fn recreate_sidecar_with_env(
    sandbox_id: &str,
    user_env_json: &str,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<SandboxRecord> {
    recreate_sidecar_impl(sandbox_id, user_env_json, None, tee).await
}

/// Shared recreate engine. `image_override = Some(img)` swaps the sidecar onto
/// `img` (image upgrade); `None` preserves the sandbox's existing image (the
/// secret re-injection / wipe path). Everything else — env, token, ports,
/// capabilities, identity — is replayed faithfully from the stored record.
pub(crate) async fn recreate_sidecar_impl(
    sandbox_id: &str,
    user_env_json: &str,
    image_override: Option<&str>,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<SandboxRecord> {
    let old = get_sandbox_by_id(sandbox_id)?;

    // TEE sandboxes cannot be recreated — it would invalidate attestation,
    // break sealed secrets, and orphan the on-chain deployment ID.
    if old.tee_deployment_id.is_some() {
        return Err(SandboxError::Validation(
            "Secret re-injection via container recreation is not supported for TEE sandboxes. \
             Use the sealed-secrets API instead."
                .into(),
        ));
    }

    // Stop if running, then delete
    if old.state == SandboxState::Running {
        let _ = stop_sidecar(&old).await;
    }
    delete_sidecar(&old, tee).await?;

    // Rebuild creation params faithfully from the stored record. An explicit
    // `image_override` (fleet upgrade) wins; otherwise keep the sandbox's own
    // image, falling back to the configured one only if it was never recorded.
    let image = match image_override {
        Some(img) => img.to_string(),
        None if old.original_image.is_empty() => current_sidecar_image(),
        None => old.original_image.clone(),
    };

    let old_token = old.token.clone();
    let params = CreateSandboxParams {
        name: old.name.clone(),
        image,
        stack: old.stack.clone(),
        agent_identifier: old.agent_identifier.clone(),
        env_json: old.base_env_json.clone(),
        user_env_json: user_env_json.to_string(),
        metadata_json: old.metadata_json.clone(),
        ssh_enabled: old.ssh_port.is_some(),
        ssh_public_key: String::new(),
        web_terminal_enabled: false,
        max_lifetime_seconds: old.max_lifetime_seconds,
        idle_timeout_seconds: old.idle_timeout_seconds,
        cpu_cores: old.cpu_cores,
        memory_mb: old.memory_mb,
        disk_gb: if old.disk_gb > 0 { old.disk_gb } else { 10 },
        owner: old.owner.clone(),
        service_id: old.service_id,
        tee_config: old.tee_config.clone(),
        port_mappings: old.extra_ports.keys().copied().collect(),
        // Replay the capability set the sandbox was originally booted
        // with — recreation after secret-injection / wipe must hand the
        // sidecar the same SIDECAR_CAPABILITIES it had before, otherwise
        // computer_use sandboxes lose Xvfb on every refresh.
        capabilities_json: old.capabilities_json.clone(),
    };

    // Preserve the original token so existing workflows/references keep working.
    let (_new_record, _attestation, _timings) =
        create_sidecar_with_token(&params, tee, Some(&old_token), Some(&old.id)).await?;
    let updated = sandboxes()?.update(&old.id, |record| {
        record.ssh_login_user = old.ssh_login_user.clone();
        record.ssh_authorized_keys = old.ssh_authorized_keys.clone();
    })?;
    if !updated {
        return Err(SandboxError::NotFound(format!(
            "Sandbox '{}' not found while restoring SSH state",
            old.id
        )));
    }
    if old.ssh_port.is_some() {
        restore_ssh_access(&get_sandbox_by_id(&old.id)?).await
    } else {
        Ok(get_sandbox_by_id(&old.id)?)
    }
}
