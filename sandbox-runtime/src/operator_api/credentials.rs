//! Owner-only delivery of credentials that must never appear in chain results.

use super::*;

pub(crate) async fn sandbox_credentials_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    // Re-read the authoritative record on every request: resume/recreation may
    // change the endpoint or credential. Reuse the existing session and owner
    // checks, including rejection of records without an owner.
    let record = resolve_sandbox(&sandbox_id, &address)?;
    // The router adds Cache-Control: no-store, including on errors.
    // Never log this response or cache it across lifecycle changes.
    Ok::<_, (StatusCode, Json<ApiError>)>(Json(json!({
        "sandbox_id": record.id,
        "sidecar_url": record.sidecar_url,
        "token": record.token,
    })))
}
