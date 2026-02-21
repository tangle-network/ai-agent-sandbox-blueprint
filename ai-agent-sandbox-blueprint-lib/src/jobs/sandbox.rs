use serde_json::json;

use crate::CreateSandboxParams;
use crate::JsonResponse;
use crate::SandboxCreateOutput;
use crate::SandboxCreateRequest;
use crate::SandboxIdRequest;
use crate::SandboxSnapshotRequest;
use crate::http::sidecar_post_json;
use crate::runtime::{
    create_sidecar, delete_sidecar, require_sandbox_owner, require_sandbox_owner_by_url,
    resume_sidecar, sandboxes, stop_sidecar,
};
use crate::tangle::extract::{CallId, Caller, TangleArg, TangleResult};
use crate::util::build_snapshot_command;
use sandbox_runtime::provision_progress::{self, ProvisionPhase};

pub async fn sandbox_create(
    Caller(caller): Caller,
    CallId(call_id): CallId,
    TangleArg(request): TangleArg<SandboxCreateRequest>,
) -> Result<TangleResult<SandboxCreateOutput>, String> {
    // Track provision progress for this call
    let _ = provision_progress::start_provision(call_id);

    let _ = provision_progress::update_provision(
        call_id,
        ProvisionPhase::ImagePull,
        Some("Preparing sandbox image".into()),
        None,
        None,
    );

    let mut params = CreateSandboxParams::from(&request);
    params.owner = super::caller_hex(&caller);

    let _ = provision_progress::update_provision(
        call_id,
        ProvisionPhase::ContainerCreate,
        Some("Creating container".into()),
        None,
        None,
    );

    let tee = crate::tee_backend().map(|b| b.as_ref());
    let (record, attestation) = create_sidecar(&params, tee).await.map_err(|e| {
        let _ = provision_progress::update_provision(
            call_id,
            ProvisionPhase::Failed,
            Some(format!("Container creation failed: {e}")),
            None,
            None,
        );
        e
    })?;

    let _ = provision_progress::update_provision(
        call_id,
        ProvisionPhase::ContainerStart,
        Some("Container started, configuring".into()),
        Some(record.id.clone()),
        None,
    );

    if request.ssh_enabled && !request.ssh_public_key.trim().is_empty() {
        crate::jobs::ssh::provision_key(
            &record.sidecar_url,
            "root",
            &request.ssh_public_key,
            &record.token,
        )
        .await
        .map_err(|e| {
            let _ = provision_progress::update_provision(
                call_id,
                ProvisionPhase::Failed,
                Some(format!("SSH key provisioning failed: {e}")),
                Some(record.id.clone()),
                None,
            );
            e
        })?;
    }

    let _ = provision_progress::update_provision(
        call_id,
        ProvisionPhase::Ready,
        Some("Sandbox ready".into()),
        Some(record.id.clone()),
        Some(record.sidecar_url.clone()),
    );

    // If TEE was used, serialize attestation and derive the public key.
    let tee_attestation_json = attestation
        .as_ref()
        .map(|att| serde_json::to_string(att).unwrap_or_default())
        .unwrap_or_default();

    let tee_public_key_json =
        if let (Some(dep_id), Some(backend)) = (&record.tee_deployment_id, crate::tee_backend()) {
            match backend.derive_public_key(dep_id).await {
                Ok(pk) => serde_json::to_string(&pk).unwrap_or_default(),
                Err(_) => String::new(),
            }
        } else {
            String::new()
        };

    let response = json!({
        "sandboxId": record.id,
        "sidecarUrl": record.sidecar_url,
        "token": record.token,
        "sshPort": record.ssh_port,
        "teeAttestationJson": tee_attestation_json,
        "teePublicKeyJson": tee_public_key_json,
    });

    Ok(TangleResult(SandboxCreateOutput {
        sandboxId: record.id.clone(),
        json: response.to_string(),
    }))
}

pub async fn sandbox_delete(
    Caller(caller): Caller,
    TangleArg(request): TangleArg<SandboxIdRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let caller_hex = super::caller_hex(&caller);
    let record = require_sandbox_owner(&request.sandbox_id, &caller_hex)?;
    let tee = crate::tee_backend().map(|b| b.as_ref());
    delete_sidecar(&record, tee).await?;

    let sandbox_id = request.sandbox_id.to_string();
    sandboxes()
        .map_err(|e| e.to_string())?
        .remove(&sandbox_id)
        .map_err(|e| e.to_string())?;

    let response = json!({
        "sandboxId": request.sandbox_id,
        "deleted": true,
    });

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn sandbox_stop(
    Caller(caller): Caller,
    TangleArg(request): TangleArg<SandboxIdRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let caller_hex = super::caller_hex(&caller);
    let record = require_sandbox_owner(&request.sandbox_id, &caller_hex)?;
    stop_sidecar(&record).await?;

    let response = json!({
        "sandboxId": request.sandbox_id,
        "stopped": true,
    });

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn sandbox_resume(
    Caller(caller): Caller,
    TangleArg(request): TangleArg<SandboxIdRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let caller_hex = super::caller_hex(&caller);
    let record = require_sandbox_owner(&request.sandbox_id, &caller_hex)?;
    resume_sidecar(&record).await?;

    let response = json!({
        "sandboxId": request.sandbox_id,
        "resumed": true,
    });

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn sandbox_snapshot(
    Caller(caller): Caller,
    TangleArg(request): TangleArg<SandboxSnapshotRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    if request.destination.trim().is_empty() {
        return Err("Snapshot destination is required".to_string());
    }

    let caller_hex = super::caller_hex(&caller);
    let record = require_sandbox_owner_by_url(&request.sidecar_url, &caller_hex)?;

    let command = build_snapshot_command(
        &request.destination,
        request.include_workspace,
        request.include_state,
    )?;

    let payload = json!({
        "command": format!("sh -c {}", crate::util::shell_escape(&command)),
    });

    let response =
        sidecar_post_json(&request.sidecar_url, "/terminals/commands", &record.token, payload).await?;

    crate::runtime::touch_sandbox(&record.id);

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}
