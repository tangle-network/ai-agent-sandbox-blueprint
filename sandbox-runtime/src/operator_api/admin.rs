//! Extracted from operator_api.rs — admin route group.

use super::*;

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Sidecar image upgrade (fleet drift remediation)
// ---------------------------------------------------------------------------
//
// Sandboxes are pinned to their birth image. When the operator ships a new
// `SIDECAR_IMAGE` (security patch, new agent harness, opencode bump), existing
// sandboxes keep running the old image — silently rotting the fleet (e.g. agent
// runs failing on an image that predates opencode). These operator-only
// endpoints detect that drift and roll sandboxes onto the current image
// in place, preserving secrets/token/ports/identity (see
// `runtime::upgrade_sidecar_image`). Gated to the managing operator: this is an
// infra/fleet action, not a per-bot owner action.

pub(crate) fn require_managing_operator(
    address: &str,
) -> std::result::Result<(), (StatusCode, Json<ApiError>)> {
    match current_managing_operator() {
        Some(op) if op.eq_ignore_ascii_case(address) => Ok(()),
        Some(_) => Err(api_error(
            StatusCode::FORBIDDEN,
            "Only the managing operator may upgrade sidecar images".to_string(),
        )),
        None => Err(api_error(
            StatusCode::FORBIDDEN,
            "Managing operator address is not configured on this node".to_string(),
        )),
    }
}

/// GET /api/operator/sidecar-image — the current target image + the sandboxes
/// still running a stale image.
pub(crate) async fn sidecar_image_drift_handler(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    if let Err(e) = require_managing_operator(&address) {
        return e.into_response();
    }
    match runtime::sandboxes_needing_image_upgrade() {
        Ok(stale) => (
            StatusCode::OK,
            Json(json!({
                "target_image": runtime::current_sidecar_image(),
                "stale": stale
                    .iter()
                    .map(|(id, img)| json!({ "sandbox_id": id, "current_image": img }))
                    .collect::<Vec<_>>(),
            })),
        )
            .into_response(),
        Err(e) => classify_sandbox_error(e).into_response(),
    }
}

/// POST /api/sandboxes/{id}/upgrade-image — recreate one sandbox onto the
/// current target image, preserving its secrets/identity.
pub(crate) async fn upgrade_sandbox_image_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_managing_operator(&address) {
        return e.into_response();
    }
    let target = runtime::current_sidecar_image();
    let _lock = runtime::acquire_lifecycle_lock(&sandbox_id).await;
    match runtime::upgrade_sidecar_image(&sandbox_id, &target, None).await {
        Ok(record) => (
            StatusCode::OK,
            Json(json!({ "sandbox_id": record.id, "image": record.original_image })),
        )
            .into_response(),
        Err(e) => classify_sandbox_error(e).into_response(),
    }
}

/// POST /api/operator/sidecar-image/upgrade-stale — roll every drifted sandbox
/// onto the current target image. The fleet-wide "clean upgrade" the operator
/// runs after shipping a new sidecar.
pub(crate) async fn upgrade_stale_sidecar_images_handler(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    if let Err(e) = require_managing_operator(&address) {
        return e.into_response();
    }
    let target = runtime::current_sidecar_image();
    let stale = match runtime::sandboxes_needing_image_upgrade() {
        Ok(s) => s,
        Err(e) => return classify_sandbox_error(e).into_response(),
    };
    let mut upgraded: Vec<String> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    for (id, _img) in stale {
        let _lock = runtime::acquire_lifecycle_lock(&id).await;
        match runtime::upgrade_sidecar_image(&id, &target, None).await {
            Ok(_) => upgraded.push(id),
            Err(e) => failed.push(json!({ "sandbox_id": id, "error": e.to_string() })),
        }
    }
    (
        StatusCode::OK,
        Json(json!({ "target_image": target, "upgraded": upgraded, "failed": failed })),
    )
        .into_response()
}
