use super::*;

pub(crate) fn bind_service_id(
    mut record: crate::SandboxRecord,
    service_id: u64,
) -> crate::SandboxRecord {
    record.service_id = Some(service_id);
    record
}

pub(crate) fn should_reuse_existing_record(
    record: &crate::SandboxRecord,
    service_id: u64,
    current_owner: Option<&str>,
) -> bool {
    if record.service_id == Some(service_id) {
        return true;
    }

    record.service_id.is_none()
        && current_owner
            .map(|owner| !owner.is_empty() && record.owner.eq_ignore_ascii_case(owner))
            .unwrap_or(false)
}

pub(crate) fn sync_runtime_service_binding(record: &crate::SandboxRecord) -> Result<(), String> {
    let Some(service_id) = record.service_id else {
        return Ok(());
    };

    if let Ok(store) = crate::runtime::sandboxes() {
        let updated = store.update(&record.id, |existing| {
            existing.service_id = Some(service_id);
        });

        if matches!(updated, Ok(true)) {
            return Ok(());
        }

        let mut sealed = record.clone();
        crate::runtime::seal_record(&mut sealed).map_err(|e| e.to_string())?;
        store
            .insert(record.id.clone(), sealed)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

pub(crate) async fn reset_stale_instance_record(
    record: &crate::SandboxRecord,
    tee: Option<&dyn TeeBackend>,
) -> Result<(), String> {
    warn!(
        sandbox_id = %record.id,
        previous_service_id = ?record.service_id,
        previous_owner = %record.owner,
        "Auto-provision: clearing stale singleton instance state before reprovisioning"
    );

    if let Err(err) = crate::runtime::delete_sidecar(record, tee).await {
        warn!(
            sandbox_id = %record.id,
            error = %err,
            "Auto-provision: stale sandbox teardown failed; clearing local state anyway"
        );
    }

    if let Ok(store) = crate::runtime::sandboxes() {
        let _ = store.remove(&record.id);
    }

    clear_instance_sandbox().map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) async fn reuse_existing_instance_record(
    record: crate::SandboxRecord,
    service_id: u64,
    report_client: Option<&blueprint_sdk::contexts::tangle::TangleClient>,
) -> Result<(), String> {
    let record = bind_service_id(record, service_id);
    set_instance_sandbox(record.clone()).map_err(|e| e.to_string())?;
    sync_runtime_service_binding(&record)?;

    info!(
        "Auto-provision: local instance already provisioned (sandbox_id='{}')",
        record.id
    );

    if let Some(client) = report_client
        && let Err(err) = ensure_local_provision_reported(client, service_id, &record).await
    {
        warn!(
            service_id = service_id,
            error = %err,
            sandbox_id = %record.id,
            "Auto-provision: reconcile report failed; pending report will be retried"
        );
    }

    Ok(())
}
