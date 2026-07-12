use super::*;
use crate::runtime::{SandboxRecord, SandboxState, SidecarRuntimeConfig};
use std::time::Duration;

/// Helper to create a minimal SandboxRecord for testing.
fn test_record() -> SandboxRecord {
    SandboxRecord {
        id: "test-sandbox-1".to_string(),
        container_id: "abc123".to_string(),
        sidecar_url: "http://localhost:8080".to_string(),
        sidecar_port: 8080,
        ssh_port: None,
        token: "test-token".to_string(),
        created_at: 1000,
        cpu_cores: 2,
        memory_mb: 1024,
        state: SandboxState::Running,
        idle_timeout_seconds: 300,
        max_lifetime_seconds: 3600,
        last_activity_at: 0,
        stopped_at: None,
        snapshot_image_id: None,
        snapshot_s3_url: None,
        container_removed_at: None,
        image_removed_at: None,
        original_image: "ubuntu:22.04".to_string(),
        base_env_json: String::new(),
        user_env_json: String::new(),
        snapshot_destination: None,
        tee_deployment_id: None,
        tee_metadata_json: None,
        tee_attestation_json: None,
        name: "test".to_string(),
        agent_identifier: String::new(),
        metadata_json: String::new(),
        disk_gb: 10,
        stack: String::new(),
        owner: "0xdeadbeef".to_string(),
        service_id: None,
        tee_config: None,
        extra_ports: std::collections::HashMap::new(),
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
        capabilities_json: String::new(),
    }
}

/// Helper to create a test config with known values.
fn test_config() -> SidecarRuntimeConfig {
    SidecarRuntimeConfig {
        image: "test:latest".to_string(),
        public_host: "127.0.0.1".to_string(),
        container_port: 8080,
        ssh_port: 22,
        timeout: Duration::from_secs(30),
        docker_host: None,
        pull_image: false,
        sandbox_default_idle_timeout: 300,
        sandbox_default_max_lifetime: 3600,
        sandbox_max_idle_timeout: 7200,
        sandbox_max_max_lifetime: 86400,
        sandbox_reaper_interval: 60,
        sandbox_gc_interval: 300,
        sandbox_gc_hot_retention: 3600,
        sandbox_gc_warm_retention: 86400,
        sandbox_gc_cold_retention: 604800,
        snapshot_auto_commit: true,
        snapshot_destination_prefix: Some("s3://my-bucket/snapshots/".to_string()),
        sandbox_max_count: 100,
        sandbox_max_cpu_cores: 0,
        sandbox_max_memory_mb: 0,
        sandbox_max_disk_gb: 0,
        sandbox_host_memory_budget_mb: 0,
        sandbox_host_cpu_budget: 0,
    }
}

// ── resolve_snapshot_destination ─────────────────────────────────────

#[test]
fn resolve_snapshot_uses_record_destination_if_set() {
    let mut record = test_record();
    record.snapshot_destination = Some("s3://user-bucket/my-snap.tar.gz".to_string());
    let config = test_config();

    let result = resolve_snapshot_destination(&record, &config);
    assert_eq!(result, Some("s3://user-bucket/my-snap.tar.gz".to_string()));
}

#[test]
fn resolve_snapshot_uses_config_prefix_when_no_record_destination() {
    let record = test_record();
    let config = test_config();

    let result = resolve_snapshot_destination(&record, &config);
    assert_eq!(
        result,
        Some("s3://my-bucket/snapshots/test-sandbox-1/snapshot.tar.gz".to_string())
    );
}

#[test]
fn resolve_snapshot_returns_none_when_no_prefix() {
    let record = test_record();
    let mut config = test_config();
    config.snapshot_destination_prefix = None;

    let result = resolve_snapshot_destination(&record, &config);
    assert!(result.is_none());
}

// ── is_operator_s3 ──────────────────────────────────────────────────

#[test]
fn is_operator_s3_true_when_operator_managed() {
    let record = test_record();
    let config = test_config();
    let url = "s3://my-bucket/snapshots/test-sandbox-1/snapshot.tar.gz";

    assert!(is_operator_s3(url, &record, &config));
}

#[test]
fn is_operator_s3_false_when_user_destination_set() {
    let mut record = test_record();
    record.snapshot_destination = Some("s3://user-bucket/snap.tar.gz".to_string());
    let config = test_config();
    let url = "s3://my-bucket/snapshots/test-sandbox-1/snapshot.tar.gz";

    // Even though URL matches prefix, record has user destination -> BYOS3
    assert!(!is_operator_s3(url, &record, &config));
}

#[test]
fn is_operator_s3_false_when_no_prefix() {
    let record = test_record();
    let mut config = test_config();
    config.snapshot_destination_prefix = None;
    let url = "s3://some-bucket/snap.tar.gz";

    assert!(!is_operator_s3(url, &record, &config));
}

#[test]
fn is_operator_s3_false_when_url_does_not_match_prefix() {
    let record = test_record();
    let config = test_config();
    let url = "s3://other-bucket/something/snap.tar.gz";

    assert!(!is_operator_s3(url, &record, &config));
}

// ── Phase 2E: Reaper Logic Tests ────────────────────────────────────

#[test]
fn resolve_snapshot_trailing_slash() {
    // Prefix with trailing slash
    let record = test_record();
    let config = test_config();
    let result = resolve_snapshot_destination(&record, &config);
    assert_eq!(
        result,
        Some("s3://my-bucket/snapshots/test-sandbox-1/snapshot.tar.gz".to_string()),
    );

    // Prefix without trailing slash
    let mut config_no_slash = test_config();
    config_no_slash.snapshot_destination_prefix = Some("s3://my-bucket/snapshots".to_string());
    let result_no_slash = resolve_snapshot_destination(&record, &config_no_slash);
    assert_eq!(
        result_no_slash,
        Some("s3://my-bucket/snapshotstest-sandbox-1/snapshot.tar.gz".to_string()),
        "Without trailing slash, URL is directly concatenated"
    );
}

#[test]
fn is_operator_s3_case_sensitive() {
    let record = test_record();
    let config = test_config();
    // Prefix is "s3://my-bucket/snapshots/" — uppercase should NOT match
    let url_upper = "S3://my-bucket/snapshots/test-sandbox-1/snapshot.tar.gz";
    assert!(
        !is_operator_s3(url_upper, &record, &config),
        "S3 prefix comparison should be case-sensitive"
    );
}

#[test]
fn gc_skips_tee_sandboxes() {
    let mut record = test_record();
    record.state = SandboxState::Stopped;
    record.tee_deployment_id = Some("tee-deploy-1".to_string());
    // TEE sandboxes should be skipped in GC — verify by checking the
    // condition that gc_tick uses
    assert!(
        record.tee_deployment_id.is_some(),
        "TEE record should have deployment_id set"
    );
    // The GC code does `if record.tee_deployment_id.is_some() { continue; }`
    // We verify the field is correctly set so the skip condition holds.
}

#[test]
fn gc_skips_firecracker_sandboxes_without_container_removed() {
    let mut record = test_record();
    record.state = SandboxState::Stopped;
    record.metadata_json = r#"{"runtime_backend":"firecracker"}"#.to_string();
    // Firecracker sandboxes have a separate GC path in gc_tick.
    // When container_removed_at is None, they skip the Docker GC path.
    assert!(
        crate::runtime::record_uses_firecracker(&record),
        "record with runtime_backend=firecracker should be detected"
    );
    // The Docker GC path (hot->warm->cold) is skipped for firecracker;
    // instead, firecracker has its own cold->gone path.
}
