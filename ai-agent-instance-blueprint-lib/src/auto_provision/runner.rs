use super::*;

/// Run auto-provision: poll for config and provision when available.
///
/// This is designed to be spawned as a background task. It will:
/// 1. Check if already provisioned (skip if so)
/// 2. Poll `getServiceConfig` until config is available
/// 3. Decode as `ProvisionRequest` and call `provision_core`
/// 4. Store the sandbox record
pub async fn run_auto_provision(
    config: AutoProvisionConfig,
    tee: Option<&dyn TeeBackend>,
    report_client: Option<blueprint_sdk::contexts::tangle::TangleClient>,
) -> Result<(), String> {
    // Already provisioned locally?
    if let Some(record) = get_instance_sandbox().map_err(|e| e.to_string())? {
        if should_reuse_existing_record(&record, config.service_id, None) {
            return reuse_existing_instance_record(
                record,
                config.service_id,
                report_client.as_ref(),
            )
            .await;
        }

        if record.service_id.is_none() {
            let owner = read_service_owner(&config).await?;
            if should_reuse_existing_record(&record, config.service_id, Some(&owner)) {
                return reuse_existing_instance_record(
                    record,
                    config.service_id,
                    report_client.as_ref(),
                )
                .await;
            }
        }

        reset_stale_instance_record(&record, tee).await?;
    }

    info!(
        "Auto-provision: polling BSM {} for service {} config (interval={}s, max_attempts={})",
        config.bsm_address, config.service_id, config.poll_interval_secs, config.max_attempts
    );

    let mut attempts = 0;
    let config_bytes = loop {
        attempts += 1;
        match read_service_config(&config).await {
            Ok(Some(bytes)) => {
                info!(
                    "Auto-provision: service config found ({} bytes)",
                    bytes.len()
                );
                break bytes;
            }
            Ok(None) => {
                if attempts >= config.max_attempts {
                    return Err(format!(
                        "Auto-provision: no service config after {} attempts",
                        config.max_attempts
                    ));
                }
                if attempts % 12 == 1 {
                    info!(
                        "Auto-provision: waiting for service config (attempt {}/{})",
                        attempts, config.max_attempts
                    );
                }
            }
            Err(e) => {
                warn!(
                    "Auto-provision: RPC error (attempt {}/{}): {e}",
                    attempts, config.max_attempts
                );
                if attempts >= config.max_attempts {
                    return Err(format!(
                        "Auto-provision: RPC failed after {} attempts: {e}",
                        config.max_attempts
                    ));
                }
            }
        }

        // Check if provisioned by another path.
        if get_instance_sandbox().map_err(|e| e.to_string())?.is_some() {
            info!("Auto-provision: instance was provisioned externally, skipping");
            return Ok(());
        }

        tokio::time::sleep(Duration::from_secs(config.poll_interval_secs)).await;
    };

    // Decode config
    let request = decode_provision_config(&config_bytes)?;
    info!(
        "Auto-provision: decoded config — name='{}', image='{}', tee={}",
        request.name, request.image, request.tee_required
    );

    // Read service owner from chain so the sandbox record has correct ownership.
    // We never auto-provision ownerless instances because instance API auth relies on owner.
    let mut owner_attempts = 0;
    let owner = loop {
        owner_attempts += 1;
        match read_service_owner(&config).await {
            Ok(addr) if !addr.is_empty() => {
                info!("Auto-provision: service owner = {addr}");
                break addr;
            }
            Ok(_) => {
                warn!(
                    "Auto-provision: service owner not set yet (attempt {}/{})",
                    owner_attempts, config.max_attempts
                );
            }
            Err(e) => {
                warn!(
                    "Auto-provision: failed to read service owner (attempt {}/{}): {e}",
                    owner_attempts, config.max_attempts
                );
            }
        }

        if owner_attempts >= config.max_attempts {
            return Err(format!(
                "Auto-provision: service owner unavailable after {} attempts",
                config.max_attempts
            ));
        }

        // Check if provisioned by another path while waiting for owner.
        if get_instance_sandbox().map_err(|e| e.to_string())?.is_some() {
            info!("Auto-provision: instance was provisioned externally, skipping");
            return Ok(());
        }

        tokio::time::sleep(Duration::from_secs(config.poll_interval_secs)).await;
    };

    // Final check before provisioning.
    if get_instance_sandbox().map_err(|e| e.to_string())?.is_some() {
        info!("Auto-provision: instance was provisioned externally, skipping");
        return Ok(());
    }

    // Provision
    let (output, record) = provision_core(&request, tee, &owner).await?;
    let record = bind_service_id(record, config.service_id);

    // Store record
    set_instance_sandbox(record.clone()).map_err(|e| e.to_string())?;
    sync_runtime_service_binding(&record)?;

    if let Some(client) = report_client.as_ref()
        && let Err(err) = report_local_provision(client, config.service_id, &output).await
    {
        warn!(
            service_id = config.service_id,
            error = %err,
            sandbox_id = %output.sandbox_id,
            "Auto-provision: direct report failed; queued pending report for retry"
        );
        mark_pending_provision_report(config.service_id, &output, &err)?;
    }

    info!(
        "Auto-provision: sandbox '{}' created at {} (ssh_port={})",
        output.sandbox_id, output.sidecar_url, output.ssh_port
    );

    Ok(())
}
