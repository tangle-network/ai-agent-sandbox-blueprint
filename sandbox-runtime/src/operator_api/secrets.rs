//! Extracted from operator_api.rs — secrets route group.

use super::*;

// ---------------------------------------------------------------------------
// Secret provisioning endpoints (2-phase)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct InjectSecretsRequest {
    pub(crate) env_json: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize)]
pub(crate) struct SecretsResponse {
    pub(crate) status: String,
    pub(crate) sandbox_id: String,
    /// Whether AI credentials are available after this operation.
    pub(crate) credentials_available: bool,
}

#[derive(Serialize)]
pub(crate) struct GetSecretsResponse {
    pub(crate) sandbox_id: String,
    pub(crate) env_json: serde_json::Map<String, serde_json::Value>,
    pub(crate) credentials_available: bool,
}

pub(crate) async fn instance_get_secrets(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    let record = match resolve_instance(&address) {
        Ok(record) => record,
        Err(err) => return err.into_response(),
    };
    if let Err(err) = reject_instance_tee_secrets(&record) {
        return err.into_response();
    }

    let env_map: serde_json::Map<String, serde_json::Value> =
        if record.user_env_json.trim().is_empty() {
            serde_json::Map::new()
        } else {
            serde_json::from_str(&record.user_env_json).unwrap_or_default()
        };

    let creds =
        workflow_runtime_credentials_available(&record.effective_env_json()).unwrap_or(false);

    (
        StatusCode::OK,
        Json(GetSecretsResponse {
            sandbox_id: record.id,
            env_json: env_map,
            credentials_available: creds,
        }),
    )
        .into_response()
}

pub(crate) async fn instance_inject_secrets(
    SessionAuth(address): SessionAuth,
    Json(body): Json<InjectSecretsRequest>,
) -> impl IntoResponse {
    if let Err(e) = crate::api_types::validate_secrets_map(&body.env_json) {
        return api_error(StatusCode::BAD_REQUEST, e).into_response();
    }

    let record = match resolve_instance(&address) {
        Ok(record) => record,
        Err(err) => return err.into_response(),
    };
    if let Err(err) = reject_instance_tee_secrets(&record) {
        return err.into_response();
    }

    match secret_provisioning::inject_secrets(&record.id, body.env_json, None).await {
        Ok(updated) => {
            sync_instance_record(&updated.id);
            let creds = workflow_runtime_credentials_available(&updated.effective_env_json())
                .unwrap_or(false);
            (
                StatusCode::OK,
                Json(SecretsResponse {
                    status: "secrets_configured".to_string(),
                    sandbox_id: updated.id,
                    credentials_available: creds,
                }),
            )
                .into_response()
        }
        Err(e) => classify_sandbox_error(e).into_response(),
    }
}

pub(crate) async fn instance_wipe_secrets(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    let record = match resolve_instance(&address) {
        Ok(record) => record,
        Err(err) => return err.into_response(),
    };
    if let Err(err) = reject_instance_tee_secrets(&record) {
        return err.into_response();
    }

    match secret_provisioning::wipe_secrets(&record.id, None).await {
        Ok(updated) => {
            sync_instance_record(&updated.id);
            let creds = workflow_runtime_credentials_available(&updated.effective_env_json())
                .unwrap_or(false);
            (
                StatusCode::OK,
                Json(SecretsResponse {
                    status: "secrets_wiped".to_string(),
                    sandbox_id: updated.id,
                    credentials_available: creds,
                }),
            )
                .into_response()
        }
        Err(e) => classify_sandbox_error(e).into_response(),
    }
}

pub(crate) async fn get_secrets(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = secret_provisioning::validate_secret_access(&sandbox_id, &address) {
        return api_error(StatusCode::FORBIDDEN, e.to_string()).into_response();
    }

    let record = match runtime::get_sandbox_by_id(&sandbox_id) {
        Ok(r) => r,
        Err(e) => return api_error(StatusCode::NOT_FOUND, e.to_string()).into_response(),
    };

    let env_map: serde_json::Map<String, serde_json::Value> =
        if record.user_env_json.trim().is_empty() {
            serde_json::Map::new()
        } else {
            serde_json::from_str(&record.user_env_json).unwrap_or_default()
        };

    let creds =
        workflow_runtime_credentials_available(&record.effective_env_json()).unwrap_or(false);

    (
        StatusCode::OK,
        Json(GetSecretsResponse {
            sandbox_id: record.id,
            env_json: env_map,
            credentials_available: creds,
        }),
    )
        .into_response()
}

pub(crate) async fn inject_secrets(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(body): Json<InjectSecretsRequest>,
) -> impl IntoResponse {
    if let Err(e) = crate::api_types::validate_secrets_map(&body.env_json) {
        return api_error(StatusCode::BAD_REQUEST, e).into_response();
    }
    if let Err(e) = secret_provisioning::validate_secret_access(&sandbox_id, &address) {
        return api_error(StatusCode::FORBIDDEN, e.to_string()).into_response();
    }

    // Lifecycle lock prevents concurrent inject/wipe from creating orphaned
    // containers via the stop → delete → create sequence in recreate_sidecar_with_env.
    let _lock = runtime::acquire_lifecycle_lock(&sandbox_id).await;
    match secret_provisioning::inject_secrets(&sandbox_id, body.env_json, None).await {
        Ok(record) => {
            let creds = workflow_runtime_credentials_available(&record.effective_env_json())
                .unwrap_or(false);
            (
                StatusCode::OK,
                Json(SecretsResponse {
                    status: "secrets_configured".to_string(),
                    sandbox_id: record.id,
                    credentials_available: creds,
                }),
            )
                .into_response()
        }
        Err(e) => classify_sandbox_error(e).into_response(),
    }
}

pub(crate) async fn wipe_secrets(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = secret_provisioning::validate_secret_access(&sandbox_id, &address) {
        return api_error(StatusCode::FORBIDDEN, e.to_string()).into_response();
    }

    let _lock = runtime::acquire_lifecycle_lock(&sandbox_id).await;
    match secret_provisioning::wipe_secrets(&sandbox_id, None).await {
        Ok(record) => {
            let creds = workflow_runtime_credentials_available(&record.effective_env_json())
                .unwrap_or(false);
            (
                StatusCode::OK,
                Json(SecretsResponse {
                    status: "secrets_wiped".to_string(),
                    sandbox_id: record.id,
                    credentials_available: creds,
                }),
            )
                .into_response()
        }
        Err(e) => classify_sandbox_error(e).into_response(),
    }
}

pub(crate) fn reject_instance_tee_secrets(
    record: &SandboxRecord,
) -> Result<(), (StatusCode, Json<ApiError>)> {
    if record.tee_config.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "TEE instances do not support plain secrets injection. Use sealed secrets instead.",
        ));
    }

    Ok(())
}

pub(crate) fn sync_instance_record(id: &str) {
    if let Ok(Some(updated)) = sandboxes().and_then(|s| s.get(id)) {
        let _ = runtime::instance_store().and_then(|s| s.insert("instance".to_string(), updated));
    }
}
