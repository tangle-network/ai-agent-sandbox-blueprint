use serde_json::json;

use crate::JsonResponse;
use crate::SandboxCreateRequest;
use crate::SandboxIdRequest;
use crate::SandboxSnapshotRequest;
use crate::auth::require_sidecar_token;
use crate::http::sidecar_post_json;
use crate::runtime::{
    create_sidecar, delete_sidecar, get_sandbox_by_id, require_sidecar_auth, resume_sidecar,
    sandboxes, stop_sidecar,
};
use crate::tangle_evm::extract::{Caller, TangleEvmArg, TangleEvmResult};
use crate::util::build_snapshot_command;

pub async fn sandbox_create(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxCreateRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let record = create_sidecar(&request).await?;

    if request.ssh_enabled && !request.ssh_public_key.trim().is_empty() {
        let _ = crate::jobs::ssh::provision_key(
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

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn sandbox_delete(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxIdRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let record = get_sandbox_by_id(&request.sandbox_id)?;
    delete_sidecar(&record).await?;

    sandboxes()?
        .lock()
        .map_err(|_| "Sandbox store poisoned".to_string())?
        .remove(&request.sandbox_id);

    let response = json!({
        "sandboxId": request.sandbox_id,
        "deleted": true,
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn sandbox_stop(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxIdRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let record = get_sandbox_by_id(&request.sandbox_id)?;
    stop_sidecar(&record).await?;

    let response = json!({
        "sandboxId": request.sandbox_id,
        "stopped": true,
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn sandbox_resume(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxIdRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let record = get_sandbox_by_id(&request.sandbox_id)?;
    resume_sidecar(&record).await?;

    let response = json!({
        "sandboxId": request.sandbox_id,
        "resumed": true,
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn sandbox_snapshot(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxSnapshotRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
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

    let response = sidecar_post_json(
        &request.sidecar_url,
        "/exec",
        &token,
        payload,
        crate::runtime::SidecarRuntimeConfig::load().timeout,
    )
    .await?;

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}
