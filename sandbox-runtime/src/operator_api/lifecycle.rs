//! Extracted from operator_api.rs — lifecycle route group.

use super::*;

// ── Stop / Resume ────────────────────────────────────────────────────────

/// Timeout for stop/resume operations (Docker stop + potential health polling).
pub(crate) const STOP_RESUME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

pub(crate) fn handle_lifecycle_outcome(
    result: Result<(), crate::SandboxError>,
    already_message: &str,
) -> Result<(), (StatusCode, Json<ApiError>)> {
    match result {
        Ok(()) => Ok(()),
        Err(crate::SandboxError::Validation(msg))
            if msg.to_ascii_lowercase().contains(already_message) =>
        {
            // Idempotent lifecycle call: already in target state.
            Ok(())
        }
        Err(crate::SandboxError::Unavailable(msg)) => {
            Err(api_error(StatusCode::SERVICE_UNAVAILABLE, msg))
        }
        Err(e) => Err(classify_sandbox_error(e)),
    }
}

pub(crate) async fn sandbox_stop_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    // Lifecycle lock prevents concurrent stop+resume or stop+stop from
    // creating divergent container/store state (TOCTOU fix).
    let _lock = runtime::acquire_lifecycle_lock(&record.id).await;
    let stop_result = tokio::time::timeout(STOP_RESUME_TIMEOUT, runtime::stop_sidecar(&record))
        .await
        .map_err(|_| api_error(StatusCode::GATEWAY_TIMEOUT, "Stop operation timed out"))?;
    handle_lifecycle_outcome(stop_result, "already stopped")?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(LifecycleApiResponse {
            success: true,
            sandbox_id: record.id,
            state: "stopped".into(),
        }),
    ))
}

pub(crate) async fn sandbox_resume_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let _lock = runtime::acquire_lifecycle_lock(&record.id).await;
    let resume_result = tokio::time::timeout(STOP_RESUME_TIMEOUT, runtime::resume_sidecar(&record))
        .await
        .map_err(|_| api_error(StatusCode::GATEWAY_TIMEOUT, "Resume operation timed out"))?;
    handle_lifecycle_outcome(resume_result, "already running")?;
    circuit_breaker::mark_healthy(&record.id);
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(LifecycleApiResponse {
            success: true,
            sandbox_id: record.id,
            state: "running".into(),
        }),
    ))
}

pub(crate) async fn instance_stop_handler(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let id = record.id.clone();
    let _lock = runtime::acquire_lifecycle_lock(&id).await;
    let stop_result = tokio::time::timeout(STOP_RESUME_TIMEOUT, runtime::stop_sidecar(&record))
        .await
        .map_err(|_| api_error(StatusCode::GATEWAY_TIMEOUT, "Stop operation timed out"))?;
    handle_lifecycle_outcome(stop_result, "already stopped")?;

    // Sync updated state back to instance store.
    if let Ok(Some(updated)) = sandboxes().and_then(|s| s.get(&id)) {
        let _ = runtime::instance_store().and_then(|s| s.insert("instance".to_string(), updated));
    }

    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(LifecycleApiResponse {
            success: true,
            sandbox_id: id,
            state: "stopped".into(),
        }),
    ))
}

pub(crate) async fn instance_resume_handler(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let id = record.id.clone();
    let _lock = runtime::acquire_lifecycle_lock(&id).await;
    let resume_result = tokio::time::timeout(STOP_RESUME_TIMEOUT, runtime::resume_sidecar(&record))
        .await
        .map_err(|_| api_error(StatusCode::GATEWAY_TIMEOUT, "Resume operation timed out"))?;
    handle_lifecycle_outcome(resume_result, "already running")?;
    circuit_breaker::mark_healthy(&id);

    // Sync updated record (port mappings may have changed) back to instance store.
    if let Ok(Some(updated)) = sandboxes().and_then(|s| s.get(&id)) {
        let _ = runtime::instance_store().and_then(|s| s.insert("instance".to_string(), updated));
    }

    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(LifecycleApiResponse {
            success: true,
            sandbox_id: id,
            state: "running".into(),
        }),
    ))
}

// ── Snapshot ─────────────────────────────────────────────────────────────

pub(crate) async fn run_snapshot(
    record: &SandboxRecord,
    req: &SnapshotApiRequest,
) -> Result<SnapshotApiResponse, (StatusCode, Json<ApiError>)> {
    if req.destination.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Snapshot destination is required",
        ));
    }
    let command = crate::util::build_snapshot_command(
        &req.destination,
        req.include_workspace,
        req.include_state,
    )
    .map_err(|e| api_error(StatusCode::BAD_REQUEST, e.to_string()))?;
    let payload = json!({ "command": format!("sh -c {}", crate::util::shell_escape(&command)) });
    let parsed = sidecar_call(
        record,
        "/terminals/commands",
        payload,
        SIDECAR_DEFAULT_TIMEOUT,
        "snapshot",
        true,
    )
    .await?;
    Ok(SnapshotApiResponse {
        success: true,
        result: parsed,
    })
}

pub(crate) async fn sandbox_snapshot_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<SnapshotApiRequest>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resp = run_snapshot(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

pub(crate) async fn instance_snapshot_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<SnapshotApiRequest>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let resp = run_snapshot(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

// ── SSH ──────────────────────────────────────────────────────────────────
