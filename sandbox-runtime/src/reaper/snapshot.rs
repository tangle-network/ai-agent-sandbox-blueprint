use super::*;

/// Resolve the snapshot destination URL for a sandbox.
pub(crate) fn resolve_snapshot_destination(
    record: &crate::runtime::SandboxRecord,
    config: &SidecarRuntimeConfig,
) -> Option<String> {
    if let Some(ref dest) = record.snapshot_destination {
        return Some(dest.clone());
    }
    config
        .snapshot_destination_prefix
        .as_ref()
        .map(|prefix| format!("{}{}/snapshot.tar.gz", prefix, record.id))
}

/// Upload a snapshot of the running container's workspace to S3/HTTP via sidecar exec.
pub(crate) async fn upload_s3_snapshot(
    record: &crate::runtime::SandboxRecord,
    destination: &str,
) -> std::result::Result<(), String> {
    let command =
        crate::util::build_snapshot_command(destination, true, true).map_err(|e| e.to_string())?;
    let payload = serde_json::json!({
        "command": format!("sh -c {}", crate::util::shell_escape(&command)),
    });
    crate::http::sidecar_post_json(
        &record.sidecar_url,
        "/terminals/commands",
        &record.token,
        payload,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Check if an S3 URL is operator-managed (not user BYOS3).
pub(crate) fn is_operator_s3(
    s3_url: &str,
    record: &crate::runtime::SandboxRecord,
    config: &SidecarRuntimeConfig,
) -> bool {
    if record.snapshot_destination.is_some() {
        return false;
    }
    if let Some(ref prefix) = config.snapshot_destination_prefix {
        return s3_url.starts_with(prefix.as_str());
    }
    false
}

/// Best-effort DELETE of an S3/HTTP snapshot URL via reqwest.
pub(crate) async fn delete_s3_snapshot(url: &str) -> std::result::Result<(), String> {
    let client = crate::util::http_client().map_err(|e| e.to_string())?;
    let resp = client
        .delete(url)
        .send()
        .await
        .map_err(|e| format!("S3 delete request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("S3 delete returned status {}", resp.status()));
    }
    Ok(())
}
