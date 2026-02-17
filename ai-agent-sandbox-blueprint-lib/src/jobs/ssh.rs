use serde_json::{Value, json};

use crate::JsonResponse;
use crate::SshProvisionRequest;
use crate::SshRevokeRequest;
use crate::http::sidecar_post_json;
use crate::runtime::require_sandbox_owner_by_url;
use crate::tangle::extract::{Caller, TangleArg, TangleResult};
use crate::util::{normalize_username, shell_escape};

fn build_ssh_command(username: &str, public_key: &str) -> String {
    let user_arg = shell_escape(username);
    let key_arg = shell_escape(public_key);
    // Fail if the user does not exist rather than silently falling back to root
    format!(
        "set -euo pipefail; user={user_arg}; \
home=$(getent passwd \"${{user}}\" | cut -d: -f6); \
if [ -z \"$home\" ]; then echo \"User ${{user}} does not exist\" >&2; exit 1; fi; \
mkdir -p \"$home/.ssh\"; chmod 700 \"$home/.ssh\"; \
if ! grep -qxF {key_arg} \"$home/.ssh/authorized_keys\" 2>/dev/null; then \
    echo {key_arg} >> \"$home/.ssh/authorized_keys\"; \
fi; chmod 600 \"$home/.ssh/authorized_keys\""
    )
}

fn build_ssh_revoke_command(username: &str, public_key: &str) -> String {
    let user_arg = shell_escape(username);
    let key_arg = shell_escape(public_key);
    // Fail if the user does not exist rather than silently falling back to root
    format!(
        "set -euo pipefail; user={user_arg}; \
home=$(getent passwd \"${{user}}\" | cut -d: -f6); \
if [ -z \"$home\" ]; then echo \"User ${{user}} does not exist\" >&2; exit 1; fi; \
if [ -f \"$home/.ssh/authorized_keys\" ]; then \
    tmp=$(mktemp /tmp/authorized_keys.XXXXXX); \
    grep -vxF {key_arg} \"$home/.ssh/authorized_keys\" > \"$tmp\" || true; \
    mv \"$tmp\" \"$home/.ssh/authorized_keys\"; chmod 600 \"$home/.ssh/authorized_keys\"; \
fi"
    )
}

pub async fn ssh_provision(
    Caller(caller): Caller,
    TangleArg(request): TangleArg<SshProvisionRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let caller_hex = super::caller_hex(&caller);
    let record = require_sandbox_owner_by_url(&request.sidecar_url, &caller_hex)?;

    let response = provision_key(
        &request.sidecar_url,
        &request.username,
        &request.public_key,
        &record.token,
    )
    .await?;

    crate::runtime::touch_sandbox(&record.id);

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn revoke_key(
    sidecar_url: &str,
    username: &str,
    public_key: &str,
    token: &str,
) -> Result<Value, String> {
    let username = normalize_username(username)?;
    let command = build_ssh_revoke_command(&username, public_key);

    let payload = json!({ "command": format!("sh -c {}", shell_escape(&command)) });
    sidecar_post_json(sidecar_url, "/terminals/commands", token, payload)
        .await
        .map_err(|e| e.to_string())
}

pub async fn ssh_revoke(
    Caller(caller): Caller,
    TangleArg(request): TangleArg<SshRevokeRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let caller_hex = super::caller_hex(&caller);
    let record = require_sandbox_owner_by_url(&request.sidecar_url, &caller_hex)?;

    let response = revoke_key(
        &request.sidecar_url,
        &request.username,
        &request.public_key,
        &record.token,
    )
    .await?;

    crate::runtime::touch_sandbox(&record.id);

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn provision_key(
    sidecar_url: &str,
    username: &str,
    public_key: &str,
    token: &str,
) -> Result<Value, String> {
    let username = normalize_username(username)?;
    let command = build_ssh_command(&username, public_key);

    let payload = json!({ "command": format!("sh -c {}", shell_escape(&command)) });
    sidecar_post_json(sidecar_url, "/terminals/commands", token, payload)
        .await
        .map_err(|e| e.to_string())
}
