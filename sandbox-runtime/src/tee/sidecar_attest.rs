//! Fetch + structurally validate the guest sidecar's attestation report.

use super::*;

/// Look up the sidecar URL and auth token for a TEE deployment by its deployment ID.
///
/// Scans the sandbox store for a record whose `tee_deployment_id` matches.
#[allow(dead_code)] // Used by TEE backends (phala, gcp, azure)
pub(crate) fn sidecar_info_for_deployment(
    deployment_id: &str,
) -> crate::error::Result<(String, String)> {
    let store = crate::runtime::sandboxes()?;
    let record = store
        .find(|r| r.tee_deployment_id.as_deref() == Some(deployment_id))?
        .ok_or_else(|| {
            crate::error::SandboxError::NotFound(format!(
                "No sandbox found for TEE deployment '{deployment_id}'"
            ))
        })?;
    Ok((record.sidecar_url.clone(), record.token.clone()))
}

/// Fetch fresh attestation from a running sidecar's `/tee/attestation` endpoint.
#[allow(dead_code)] // Used by TEE backends
pub(crate) async fn fetch_sidecar_attestation(
    sidecar_url: &str,
    token: &str,
) -> crate::error::Result<AttestationReport> {
    fetch_sidecar_attestation_with_report_data(sidecar_url, token, None).await
}

/// Fetch fresh attestation from a running sidecar, optionally bound to caller report data.
#[allow(dead_code)] // Used by TEE backends
pub(crate) async fn fetch_sidecar_attestation_with_report_data(
    sidecar_url: &str,
    token: &str,
    report_data: Option<[u8; 64]>,
) -> crate::error::Result<AttestationReport> {
    let url = crate::http::build_url(sidecar_url, "/tee/attestation")?;
    let headers = crate::http::auth_headers(token)?;
    let method = if report_data.is_some() {
        reqwest::Method::POST
    } else {
        reqwest::Method::GET
    };
    let body = report_data.map(|data| {
        serde_json::json!({
            "attestation_nonce": hex::encode(data),
        })
    });
    let (_status, body) = crate::http::send_json(method, url, body, headers).await?;
    let report = parse_sidecar_attestation_response(&body)?;

    // Basic sanity check — callers should also validate the TEE type matches.
    if report.evidence.is_empty() {
        return Err(crate::error::SandboxError::CloudProvider(
            "Sidecar returned empty attestation evidence".into(),
        ));
    }
    if report.measurement.is_empty() {
        return Err(crate::error::SandboxError::CloudProvider(
            "Sidecar returned empty attestation measurement".into(),
        ));
    }

    Ok(report)
}

pub(crate) fn parse_sidecar_attestation_response(
    body: &str,
) -> crate::error::Result<AttestationReport> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|e| {
        crate::error::SandboxError::Http(format!("Invalid attestation response: {e}"))
    })?;
    let report_value = value.get("attestation").cloned().unwrap_or(value);
    serde_json::from_value(report_value)
        .map_err(|e| crate::error::SandboxError::Http(format!("Invalid attestation response: {e}")))
}

/// Validate an attestation report for completeness and type correctness.
///
/// Called after every attestation fetch (sidecar or native) to catch silent
/// failures where the sidecar returns 200 with empty/wrong data.
#[allow(dead_code)] // Used by TEE backends (attestation.rs)
pub(crate) fn validate_attestation_report(
    report: &AttestationReport,
    expected_type: &TeeType,
) -> crate::error::Result<()> {
    if report.evidence.is_empty() {
        return Err(crate::error::SandboxError::CloudProvider(
            "Attestation evidence is empty — TEE device may not be available".into(),
        ));
    }
    if &report.tee_type != expected_type {
        return Err(crate::error::SandboxError::CloudProvider(format!(
            "Attestation type mismatch: expected {expected_type:?}, got {:?}",
            report.tee_type
        )));
    }
    if report.measurement.is_empty() {
        return Err(crate::error::SandboxError::CloudProvider(
            "Attestation measurement is empty — report may be malformed".into(),
        ));
    }
    Ok(())
}
