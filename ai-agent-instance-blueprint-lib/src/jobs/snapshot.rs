use serde_json::json;

use crate::InstanceSnapshotRequest;
use crate::JsonResponse;
use crate::http::sidecar_post_json;
use crate::require_instance_sandbox;
use crate::tangle::extract::{Caller, TangleArg, TangleResult};
use crate::util::build_snapshot_command;

pub async fn instance_snapshot(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<InstanceSnapshotRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    if request.destination.trim().is_empty() {
        return Err("Snapshot destination is required".to_string());
    }

    let sandbox = require_instance_sandbox()?;

    let command = build_snapshot_command(
        &request.destination,
        request.include_workspace,
        request.include_state,
    )
    .map_err(|e| e.to_string())?;

    let payload = json!({
        "command": format!("sh -c {}", crate::util::shell_escape(&command)),
    });

    let response = sidecar_post_json(
        &sandbox.sidecar_url,
        "/terminals/commands",
        &sandbox.token,
        payload,
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::runtime::touch_sandbox(&sandbox.id);

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}
