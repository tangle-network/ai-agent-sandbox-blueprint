use serde_json::Value;

use crate::InstanceSshProvisionRequest;
use crate::InstanceSshRevokeRequest;
use crate::JsonResponse;
use crate::require_instance_sandbox;
use crate::runtime::get_sandbox_by_url;
use crate::tangle::extract::{Caller, TangleArg, TangleResult};

pub async fn provision_key(
    sidecar_url: &str,
    username: &str,
    public_key: &str,
    _token: &str,
) -> Result<Value, String> {
    let record = get_sandbox_by_url(sidecar_url).map_err(|e| e.to_string())?;
    let (_, result) =
        sandbox_runtime::runtime::provision_ssh_key(&record, Some(username), public_key)
            .await
            .map_err(|e| e.to_string())?;
    Ok(result)
}

pub async fn revoke_key(
    sidecar_url: &str,
    username: &str,
    public_key: &str,
    _token: &str,
) -> Result<Value, String> {
    let record = get_sandbox_by_url(sidecar_url).map_err(|e| e.to_string())?;
    let (_, result) = sandbox_runtime::runtime::revoke_ssh_key(&record, Some(username), public_key)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result)
}

pub async fn instance_ssh_provision(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<InstanceSshProvisionRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let sandbox = require_instance_sandbox()?;

    let (username, result) = sandbox_runtime::runtime::provision_ssh_key(
        &sandbox,
        Some(request.username.as_str()),
        &request.public_key,
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::runtime::touch_sandbox(&sandbox.id);

    Ok(TangleResult(JsonResponse {
        json: serde_json::json!({
            "success": true,
            "username": username,
            "result": result.get("result").cloned().unwrap_or(result),
        })
        .to_string(),
    }))
}

pub async fn instance_ssh_revoke(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<InstanceSshRevokeRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let sandbox = require_instance_sandbox()?;

    let (username, result) = sandbox_runtime::runtime::revoke_ssh_key(
        &sandbox,
        Some(request.username.as_str()),
        &request.public_key,
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::runtime::touch_sandbox(&sandbox.id);

    Ok(TangleResult(JsonResponse {
        json: serde_json::json!({
            "success": true,
            "username": username,
            "result": result.get("result").cloned().unwrap_or(result),
        })
        .to_string(),
    }))
}
