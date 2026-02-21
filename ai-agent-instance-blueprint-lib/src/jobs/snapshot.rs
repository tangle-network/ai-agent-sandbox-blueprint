use serde_json::json;

use crate::InstanceSnapshotRequest;
use crate::JsonResponse;
use crate::http::sidecar_post_json;
use crate::require_instance_sandbox;
use crate::tangle::extract::{Caller, TangleArg, TangleResult};
use crate::util::build_snapshot_command;

/// Core snapshot logic — testable without TangleArg extractors.
pub async fn run_instance_snapshot(
    sidecar_url: &str,
    sidecar_token: &str,
    sandbox_id: &str,
    destination: &str,
    include_workspace: bool,
    include_state: bool,
) -> Result<String, String> {
    if destination.trim().is_empty() {
        return Err("Snapshot destination is required".to_string());
    }

    let command = build_snapshot_command(destination, include_workspace, include_state)
        .map_err(|e| e.to_string())?;

    let payload = json!({
        "command": format!("sh -c {}", crate::util::shell_escape(&command)),
    });

    let response = sidecar_post_json(sidecar_url, "/terminals/commands", sidecar_token, payload)
        .await
        .map_err(|e| e.to_string())?;

    crate::runtime::touch_sandbox(sandbox_id);

    Ok(response.to_string())
}

pub async fn instance_snapshot(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<InstanceSnapshotRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let sandbox = require_instance_sandbox()?;
    let json = run_instance_snapshot(
        &sandbox.sidecar_url,
        &sandbox.token,
        &sandbox.id,
        &request.destination,
        request.include_workspace,
        request.include_state,
    )
    .await?;
    Ok(TangleResult(JsonResponse { json }))
}
