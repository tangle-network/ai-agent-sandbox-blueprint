use super::*;

use crate::error::{Result, SandboxError};

/// Validate a snapshot destination URL against SSRF risks.
///
/// Rejects:
/// - Non-HTTPS/S3 schemes (file://, ftp://, gopher://, etc.)
/// - Private/loopback IP addresses (IPv4 and IPv6)
/// - IPv4-mapped IPv6 addresses (`::ffff:10.0.0.1`)
/// - IPv6 unique-local (`fc00::/7`) and link-local (`fe80::/10`)
/// - `localhost` hostname
const MAX_SNAPSHOT_URL_LEN: usize = 2048;

fn validate_snapshot_destination(destination: &str) -> Result<()> {
    let trimmed = destination.trim();

    if trimmed.len() > MAX_SNAPSHOT_URL_LEN {
        return Err(SandboxError::Validation(format!(
            "Snapshot destination URL too long ({} bytes, max {MAX_SNAPSHOT_URL_LEN})",
            trimmed.len()
        )));
    }

    // Allow s3:// URIs (handled by the sidecar's S3 client, not curl)
    if trimmed.starts_with("s3://") {
        return Ok(());
    }

    // Require https:// scheme
    if !trimmed.starts_with("https://") {
        return Err(SandboxError::Validation(
            "Snapshot destination must use https:// or s3:// scheme".into(),
        ));
    }

    // Extract the host portion. Handle IPv6 bracket notation: [::1]
    let after_scheme = &trimmed["https://".len()..];
    let host = if after_scheme.starts_with('[') {
        // IPv6 bracket notation: [::1]:port/path
        after_scheme
            .find(']')
            .map(|end| &after_scheme[1..end])
            .unwrap_or("")
    } else {
        after_scheme
            .split('/')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
    };

    // Block localhost
    if host.eq_ignore_ascii_case("localhost") {
        return Err(SandboxError::Validation(
            "Snapshot destination must not target localhost".into(),
        ));
    }

    // Require the host to be a valid IP literal. Rejecting DNS hostnames
    // eliminates DNS rebinding attacks where an attacker-controlled name
    // resolves to an internal IP at request time (TOCTOU).
    let ip: std::net::IpAddr = host.parse().map_err(|_| {
        SandboxError::Validation(
            "Snapshot destination must use an IP address, not a hostname (DNS rebinding protection)"
                .into(),
        )
    })?;

    // Block private/link-local/internal IP addresses (IPv4 and IPv6)
    let is_internal = match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified() // 0.0.0.0
                // Cloud metadata: 169.254.x.x
                || v4.octets()[0] == 169
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified() // ::
                // Unique-local (fc00::/7)
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // Link-local (fe80::/10)
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // IPv4-mapped IPv6 (::ffff:x.x.x.x) — check the embedded v4
                || v6.to_ipv4_mapped().is_some_and(|v4| {
                    v4.is_loopback()
                        || v4.is_private()
                        || v4.is_link_local()
                        || v4.is_unspecified()
                        || v4.octets()[0] == 169
                })
        }
    };
    if is_internal {
        return Err(SandboxError::Validation(
            "Snapshot destination must not target private/internal IP addresses".into(),
        ));
    }

    Ok(())
}

pub fn build_snapshot_command(
    destination: &str,
    include_workspace: bool,
    include_state: bool,
) -> Result<String> {
    validate_snapshot_destination(destination)?;

    let mut paths = Vec::new();
    if include_workspace {
        paths.push("/home/agent");
    }
    if include_state {
        paths.push("/var/lib/sidecar");
    }
    if paths.is_empty() {
        return Err(SandboxError::Validation(
            "Snapshot must include workspace or state".into(),
        ));
    }

    let dest = shell_escape(destination);
    let targets = paths.join(" ");
    Ok(format!(
        "set -euo pipefail; tmp=$(mktemp /tmp/snapshot-XXXXXX); \
 tar -czf \"$tmp\" {targets}; \
 curl -fsSL -X PUT --upload-file \"$tmp\" {dest}; \
 rm -f \"$tmp\""
    ))
}
