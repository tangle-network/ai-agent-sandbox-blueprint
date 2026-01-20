use serde_json::{Value, json};

use crate::JsonResponse;
use crate::SshProvisionRequest;
use crate::SshRevokeRequest;
use crate::auth::require_sidecar_token;
use crate::http::sidecar_post_json;
use crate::runtime::require_sidecar_auth;
use crate::tangle_evm::extract::{Caller, TangleEvmArg, TangleEvmResult};
use crate::util::{normalize_username, shell_escape};

fn build_ssh_command(username: &str, public_key: &str) -> String {
    let user_arg = shell_escape(username);
    let key_arg = shell_escape(public_key);
    format!(
        "set -euo pipefail; user={user_arg}; \
home=$(getent passwd \"${{user}}\" | cut -d: -f6 || echo \"/root\"); \
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
home=$(getent passwd \"${{user}}\" | cut -d: -f6 || echo \"/root\"); \
if [ -f \"$home/.ssh/authorized_keys\" ]; then \
    tmp=$(mktemp /tmp/authorized_keys.XXXXXX); \
    grep -vxF {key_arg} \"$home/.ssh/authorized_keys\" > \"$tmp\" || true; \
    mv \"$tmp\" \"$home/.ssh/authorized_keys\"; chmod 600 \"$home/.ssh/authorized_keys\"; \
fi"
    )
}

pub async fn ssh_provision(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SshProvisionRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let token = require_sidecar_token(&request.sidecar_token)?;
    require_sidecar_auth(&request.sidecar_url, &token)?;

    let response = provision_key(
        &request.sidecar_url,
        &request.username,
        &request.public_key,
        &token,
    )
    .await?;

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn ssh_revoke(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SshRevokeRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let token = require_sidecar_token(&request.sidecar_token)?;
    require_sidecar_auth(&request.sidecar_url, &token)?;

    let username = normalize_username(&request.username)?;
    let command = build_ssh_revoke_command(&username, &request.public_key);

    let payload = json!({ "command": format!("sh -c {}", shell_escape(&command)) });
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

pub(crate) async fn provision_key(
    sidecar_url: &str,
    username: &str,
    public_key: &str,
    token: &str,
) -> Result<Value, String> {
    let username = normalize_username(username)?;
    let command = build_ssh_command(&username, public_key);

    let payload = json!({ "command": format!("sh -c {}", shell_escape(&command)) });
    sidecar_post_json(
        sidecar_url,
        "/exec",
        token,
        payload,
        crate::runtime::SidecarRuntimeConfig::load().timeout,
    )
    .await
}
