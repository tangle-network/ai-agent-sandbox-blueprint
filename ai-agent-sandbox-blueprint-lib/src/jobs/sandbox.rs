use serde_json::json;

use crate::JsonResponse;
use crate::SandboxCreateOutput;
use crate::SandboxCreateRequest;
use crate::SandboxIdRequest;
use crate::SandboxSnapshotRequest;
use crate::auth::require_sidecar_token;
use crate::http::sidecar_post_json;
use crate::runtime::{
    create_sidecar, delete_sidecar, get_sandbox_by_id, require_sidecar_auth, resume_sidecar,
    sandboxes, stop_sidecar,
};
use crate::tangle::extract::{Caller, TangleArg, TangleResult};
use crate::util::build_snapshot_command;

pub async fn sandbox_create(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<SandboxCreateRequest>,
) -> Result<TangleResult<SandboxCreateOutput>, String> {
    let record = create_sidecar(&request).await?;

    if request.ssh_enabled && !request.ssh_public_key.trim().is_empty() {
        crate::jobs::ssh::provision_key(
            &record.sidecar_url,
            "root",
            &request.ssh_public_key,
            &record.token,
        )
        .await?;
    }

    let response = json!({
        "sandboxId": record.id,
        "sidecarUrl": record.sidecar_url,
        "token": record.token,
        "sshPort": record.ssh_port,
    });

    Ok(TangleResult(SandboxCreateOutput {
        sandboxId: record.id.clone(),
        json: response.to_string(),
    }))
}

pub async fn sandbox_delete(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<SandboxIdRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let record = get_sandbox_by_id(&request.sandbox_id)?;
    delete_sidecar(&record).await?;

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
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<SandboxIdRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let record = get_sandbox_by_id(&request.sandbox_id)?;
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
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<SandboxIdRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let record = get_sandbox_by_id(&request.sandbox_id)?;
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
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<SandboxSnapshotRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    if request.destination.trim().is_empty() {
        return Err("Snapshot destination is required".to_string());
    }

    let token = require_sidecar_token(&request.sidecar_token)?;
    require_sidecar_auth(&request.sidecar_url, &token)?;

    let command = build_snapshot_command(
        &request.destination,
        request.include_workspace,
        request.include_state,
    )?;

    let payload = json!({
        "command": format!("sh -c {}", crate::util::shell_escape(&command)),
    });

    let response =
        sidecar_post_json(&request.sidecar_url, "/terminals/commands", &token, payload).await?;

    if let Some(record) = crate::runtime::get_sandbox_by_url_opt(&request.sidecar_url) {
        crate::runtime::touch_sandbox(&record.id);
    }

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}
