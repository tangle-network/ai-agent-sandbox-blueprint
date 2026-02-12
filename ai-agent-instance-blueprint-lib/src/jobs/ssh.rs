use serde_json::{Value, json};

use crate::InstanceSshProvisionRequest;
use crate::InstanceSshRevokeRequest;
use crate::JsonResponse;
use crate::http::sidecar_post_json;
use crate::require_instance_sandbox;
use crate::tangle::extract::{Caller, TangleArg, TangleResult};
use crate::util::{normalize_username, shell_escape};

fn build_ssh_command(username: &str, public_key: &str) -> String {
    let user_arg = shell_escape(username);
    let key_arg = shell_escape(public_key);
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

pub async fn instance_ssh_provision(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<InstanceSshProvisionRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let sandbox = require_instance_sandbox()?;

    let response = provision_key(
        &sandbox.sidecar_url,
        &request.username,
        &request.public_key,
        &sandbox.token,
    )
    .await?;

    crate::runtime::touch_sandbox(&sandbox.id);

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn instance_ssh_revoke(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<InstanceSshRevokeRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let sandbox = require_instance_sandbox()?;

    let response = revoke_key(
        &sandbox.sidecar_url,
        &request.username,
        &request.public_key,
        &sandbox.token,
    )
    .await?;

    crate::runtime::touch_sandbox(&sandbox.id);

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}
