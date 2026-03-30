use serde_json::Value;

use crate::JsonResponse;
use crate::SshProvisionRequest;
use crate::SshRevokeRequest;
use crate::runtime::{get_sandbox_by_url, require_sandbox_owner_by_url};
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

pub async fn ssh_provision(
    Caller(caller): Caller,
    TangleArg(request): TangleArg<SshProvisionRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let caller_hex = super::caller_hex(&caller);
    let record = require_sandbox_owner_by_url(&request.sidecar_url, &caller_hex)?;

    let (username, result) = sandbox_runtime::runtime::provision_ssh_key(
        &record,
        Some(request.username.as_str()),
        &request.public_key,
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::runtime::touch_sandbox(&record.id);

    Ok(TangleResult(JsonResponse {
        json: serde_json::json!({
            "success": true,
            "username": username,
            "result": result.get("result").cloned().unwrap_or(result),
        })
        .to_string(),
    }))
}

pub async fn ssh_revoke(
    Caller(caller): Caller,
    TangleArg(request): TangleArg<SshRevokeRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let caller_hex = super::caller_hex(&caller);
    let record = require_sandbox_owner_by_url(&request.sidecar_url, &caller_hex)?;

    let (username, result) = sandbox_runtime::runtime::revoke_ssh_key(
        &record,
        Some(request.username.as_str()),
        &request.public_key,
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::runtime::touch_sandbox(&record.id);

    Ok(TangleResult(JsonResponse {
        json: serde_json::json!({
            "success": true,
            "username": username,
            "result": result.get("result").cloned().unwrap_or(result),
        })
        .to_string(),
    }))
}
