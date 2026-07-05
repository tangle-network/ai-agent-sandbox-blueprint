//! Shared sandbox/instance resolution helpers.

use super::*;

// ---------------------------------------------------------------------------
// Sandbox operation endpoints (exec, prompt, task, stop, resume, snapshot, SSH)
// ---------------------------------------------------------------------------

/// Look up a sandbox by ID and validate caller ownership.
pub(crate) fn resolve_sandbox(
    sandbox_id: &str,
    caller: &str,
) -> Result<SandboxRecord, (StatusCode, Json<ApiError>)> {
    runtime::require_sandbox_owner(sandbox_id, caller).map_err(|e| {
        let status = match &e {
            crate::SandboxError::NotFound(_) => StatusCode::NOT_FOUND,
            crate::SandboxError::Auth(_) => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        api_error(status, e.to_string())
    })
}

/// Look up the singleton instance sandbox and validate ownership.
pub(crate) fn resolve_instance(
    caller: &str,
) -> Result<SandboxRecord, (StatusCode, Json<ApiError>)> {
    let record = runtime::get_instance_sandbox()
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Instance not provisioned"))?;

    if record.owner.is_empty() {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "Instance has no owner configured",
        ));
    }
    if !record.owner.eq_ignore_ascii_case(caller) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "Not authorized for this instance",
        ));
    }
    Ok(record)
}

pub(crate) fn require_running(record: &SandboxRecord) -> Result<(), (StatusCode, Json<ApiError>)> {
    if record.state == SandboxState::Running {
        return Ok(());
    }

    Err(api_error(
        StatusCode::CONFLICT,
        format!("Sandbox {} is stopped; resume it first", record.id),
    ))
}
