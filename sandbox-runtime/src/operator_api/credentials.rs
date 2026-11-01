//! Owner-only sidecar bearer.
//!
//! The create job omits this bearer from the chain result when
//! `capabilities_json` contains `credentials_off_chain`. The owner reads it
//! here, over the existing session, instead of from the public receipt.

use super::*;

#[derive(Serialize)]
pub(crate) struct SandboxCredentialsResponse {
    pub(crate) sandbox_id: String,
    pub(crate) sidecar_url: String,
    pub(crate) token: String,
}

pub(crate) async fn get_sandbox_credentials(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = match secret_provisioning::validate_secret_access(&sandbox_id, &address) {
        Ok(record) => record,
        Err(err) => return api_error(StatusCode::FORBIDDEN, err.to_string()).into_response(),
    };
    (
        StatusCode::OK,
        Json(SandboxCredentialsResponse {
            sandbox_id: record.id,
            sidecar_url: record.sidecar_url,
            token: record.token,
        }),
    )
        .into_response()
}
