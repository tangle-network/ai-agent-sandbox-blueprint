//! Extracted from operator_api.rs — ssh route group.

use super::*;

pub(crate) fn require_ssh(record: &SandboxRecord) -> Result<(), (StatusCode, Json<ApiError>)> {
    if record.ssh_port.is_none() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "SSH is not enabled for this sandbox",
        ));
    }
    Ok(())
}

pub(crate) async fn detect_ssh_username(
    record: &SandboxRecord,
) -> Result<String, (StatusCode, Json<ApiError>)> {
    runtime::detect_ssh_username(record)
        .await
        .map_err(|e| api_error(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))
}

pub(crate) async fn run_ssh_provision(
    record: &SandboxRecord,
    req: &SshProvisionApiRequest,
) -> Result<SshApiResponse, (StatusCode, Json<ApiError>)> {
    let (username, parsed) =
        runtime::provision_ssh_key(record, req.username.as_deref(), &req.public_key)
            .await
            .map_err(|e| api_error(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    Ok(SshApiResponse {
        success: true,
        username,
        result: parsed,
    })
}

pub(crate) async fn run_ssh_revoke(
    record: &SandboxRecord,
    req: &SshRevokeApiRequest,
) -> Result<SshApiResponse, (StatusCode, Json<ApiError>)> {
    let (username, parsed) =
        runtime::revoke_ssh_key(record, req.username.as_deref(), &req.public_key)
            .await
            .map_err(|e| api_error(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    Ok(SshApiResponse {
        success: true,
        username,
        result: parsed,
    })
}

pub(crate) async fn sandbox_ssh_user_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    require_ssh(&record)?;
    let username = detect_ssh_username(&record).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(SshUserApiResponse {
            success: true,
            username,
        }),
    ))
}

pub(crate) async fn sandbox_ssh_provision_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<SshProvisionApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    require_ssh(&record)?;
    let resp = run_ssh_provision(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

pub(crate) async fn sandbox_ssh_revoke_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<SshRevokeApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    require_ssh(&record)?;
    let resp = run_ssh_revoke(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

pub(crate) async fn instance_ssh_user_handler(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    require_ssh(&record)?;
    let username = detect_ssh_username(&record).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(SshUserApiResponse {
            success: true,
            username,
        }),
    ))
}

pub(crate) async fn instance_ssh_provision_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<SshProvisionApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_instance(&address)?;
    require_ssh(&record)?;
    let resp = run_ssh_provision(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

pub(crate) async fn instance_ssh_revoke_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<SshRevokeApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_instance(&address)?;
    require_ssh(&record)?;
    let resp = run_ssh_revoke(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}
