use super::*;

pub(crate) fn next_sandbox_id() -> String {
    format!("sandbox-{}", uuid::Uuid::new_v4())
}

pub fn get_sandbox_by_id(id: &str) -> Result<SandboxRecord> {
    let mut record = sandboxes()?
        .get(id)?
        .ok_or_else(|| SandboxError::NotFound(format!("Sandbox '{id}' not found")))?;
    unseal_record(&mut record)?;
    Ok(record)
}

pub fn get_sandbox_by_url(sidecar_url: &str) -> Result<SandboxRecord> {
    let url = sidecar_url.to_string();
    let mut record = sandboxes()?
        .find(|record| record.sidecar_url == url)?
        .ok_or_else(|| {
            SandboxError::NotFound(format!("Sandbox not found for URL: {sidecar_url}"))
        })?;
    unseal_record(&mut record)?;
    Ok(record)
}

/// Update `last_activity_at` to now for the given sandbox.
pub fn touch_sandbox(sandbox_id: &str) {
    if let Ok(store) = sandboxes() {
        let now = crate::util::now_ts();
        let _ = store.update(sandbox_id, |r| {
            r.last_activity_at = now;
        });
    }
}

/// Find a sandbox by its sidecar URL, returning `None` instead of an error if not found.
pub fn get_sandbox_by_url_opt(sidecar_url: &str) -> Option<SandboxRecord> {
    let url = sidecar_url.to_string();
    sandboxes().ok().and_then(|store| {
        store
            .find(|record| record.sidecar_url == url)
            .ok()
            .flatten()
            .and_then(|mut r| unseal_record(&mut r).ok().map(|()| r))
    })
}

/// Validate that `caller` owns the sandbox, returning the record on success.
pub fn require_sandbox_owner(sandbox_id: &str, caller: &str) -> Result<SandboxRecord> {
    let record = get_sandbox_by_id(sandbox_id)?;
    if record.owner.is_empty() {
        return Err(SandboxError::Auth(format!(
            "Sandbox '{sandbox_id}' has no owner configured"
        )));
    }
    if record.owner.eq_ignore_ascii_case(caller) {
        Ok(record)
    } else {
        Err(SandboxError::Auth(format!(
            "Caller {caller} does not own sandbox '{sandbox_id}'"
        )))
    }
}

/// Validate that `caller` owns the sandbox at `sidecar_url` AND the token matches.
pub fn require_sidecar_owner_auth(
    sidecar_url: &str,
    token: &str,
    caller: &str,
) -> Result<SandboxRecord> {
    let record = require_sidecar_auth(sidecar_url, token)?;
    if record.owner.is_empty() {
        return Err(SandboxError::Auth("Sandbox has no owner configured".into()));
    }
    if record.owner.eq_ignore_ascii_case(caller) {
        Ok(record)
    } else {
        Err(SandboxError::Auth(format!(
            "Caller {caller} does not own sandbox at '{sidecar_url}'"
        )))
    }
}

/// Validate that `caller` owns the sandbox at `sidecar_url` (no token required).
///
/// Used by job handlers where the on-chain `Caller` extractor provides auth and
/// the sidecar token is looked up from the stored `SandboxRecord`.
pub fn require_sandbox_owner_by_url(sidecar_url: &str, caller: &str) -> Result<SandboxRecord> {
    let record = get_sandbox_by_url(sidecar_url)?;
    if record.owner.is_empty() {
        return Err(SandboxError::Auth("Sandbox has no owner configured".into()));
    }
    if record.owner.eq_ignore_ascii_case(caller) {
        Ok(record)
    } else {
        Err(SandboxError::Auth(format!(
            "Caller {caller} does not own sandbox at '{sidecar_url}'"
        )))
    }
}

/// Validate sidecar token using constant-time comparison to prevent timing attacks.
pub fn require_sidecar_auth(sidecar_url: &str, token: &str) -> Result<SandboxRecord> {
    let record = get_sandbox_by_url(sidecar_url)?;
    if record.token.as_bytes().ct_eq(token.as_bytes()).into() {
        Ok(record)
    } else {
        Err(SandboxError::Auth("Unauthorized sidecar_token".into()))
    }
}
