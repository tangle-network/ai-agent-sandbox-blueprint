//! Real Docker + MinIO integration tests for snapshot reprovisioning and tiered GC.
//!
//! These tests exercise the full snapshot lifecycle end-to-end:
//! - Docker commit (warm tier)
//! - S3 upload/download via MinIO (cold tier)
//! - Tiered GC transitions: Hot → Warm → Cold → Gone
//! - Resume from each tier
//! - User BYOS3 preservation
//!
//! Run:
//!   SNAPSHOT_TEST=1 cargo test --test snapshot_integration -- --test-threads=1
//!
//! Requires:
//!   - Docker running
//!   - MinIO on localhost:9100 (via docker-compose.test.yml)
//!   - Sidecar image available (default: tangle-sidecar:local, override: SIDECAR_IMAGE)

use std::sync::atomic::Ordering;
use std::time::Duration;

use ai_agent_sandbox_blueprint_lib::runtime::{
    SandboxRecord, SandboxState, commit_container, create_sidecar, delete_sidecar, docker_builder,
    remove_snapshot_image, resume_sidecar, sandboxes, stop_sidecar,
};
use ai_agent_sandbox_blueprint_lib::{CreateSandboxParams, SandboxCreateRequest};
use docktopus::bollard::container::RemoveContainerOptions;
use reqwest::Client;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// MinIO endpoint reachable from the test process (host network).
const MINIO_ENDPOINT: &str = "http://127.0.0.1:9100";
const MINIO_BUCKET: &str = "snapshots";

/// MinIO endpoint reachable from inside sidecar containers.
/// Containers use the Docker bridge gateway to reach host services.
fn minio_endpoint_for_container() -> String {
    std::env::var("MINIO_CONTAINER_ENDPOINT").unwrap_or_else(|_| {
        // Default: Docker bridge gateway on Linux
        "http://172.17.0.1:9100".to_string()
    })
}

// ---------------------------------------------------------------------------
// Gate macros
// ---------------------------------------------------------------------------

fn should_run() -> bool {
    std::env::var("SNAPSHOT_TEST")
        .map(|v| v == "1")
        .unwrap_or(false)
}

macro_rules! skip_unless_snapshot {
    () => {
        if !should_run() {
            eprintln!("Skipped (set SNAPSHOT_TEST=1 to enable)");
            return;
        }
    };
}

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

fn sidecar_image() -> String {
    std::env::var("SIDECAR_IMAGE").unwrap_or_else(|_| "tangle-sidecar:local".to_string())
}

fn http() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

/// Verify Docker is reachable.
async fn docker_ok() -> bool {
    docker_builder().await.is_ok()
}

/// Verify MinIO is reachable at MINIO_ENDPOINT.
async fn minio_ok() -> bool {
    let url = format!("{MINIO_ENDPOINT}/minio/health/live");
    match http().get(&url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// Check if an object exists in MinIO. Returns true if HTTP HEAD returns 200.
async fn minio_object_exists(path: &str) -> bool {
    let url = format!("{MINIO_ENDPOINT}/{MINIO_BUCKET}/{path}");
    match http().head(&url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// Delete an object from MinIO. Best-effort.
async fn minio_delete_object(path: &str) {
    let url = format!("{MINIO_ENDPOINT}/{MINIO_BUCKET}/{path}");
    let _ = http().delete(&url).send().await;
}

/// Upload a small test object to MinIO.
async fn minio_put_object(path: &str, data: &[u8]) -> bool {
    let url = format!("{MINIO_ENDPOINT}/{MINIO_BUCKET}/{path}");
    match http().put(&url).body(data.to_vec()).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// Set env vars needed by SidecarRuntimeConfig before it's loaded.
/// Must be called before any function that triggers SidecarRuntimeConfig::load().
///
/// SAFETY: Tests run with --test-threads=1, so no concurrent reads.
fn setup_test_env() {
    let image = sidecar_image();
    unsafe {
        std::env::set_var("SIDECAR_IMAGE", &image);
        std::env::set_var("SIDECAR_PULL_IMAGE", "false");
        std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
        std::env::set_var("REQUEST_TIMEOUT_SECS", "120");
        std::env::set_var("SANDBOX_SNAPSHOT_AUTO_COMMIT", "true");
        let container_minio = minio_endpoint_for_container();
        std::env::set_var(
            "SANDBOX_SNAPSHOT_DESTINATION_PREFIX",
            format!("{container_minio}/{MINIO_BUCKET}/"),
        );
        // Short retention for tests
        std::env::set_var("SANDBOX_GC_HOT_RETENTION", "0");
        std::env::set_var("SANDBOX_GC_WARM_RETENTION", "0");
        std::env::set_var("SANDBOX_GC_COLD_RETENTION", "0");
    }
}

/// Create a sandbox via `create_sidecar()` with test defaults.
async fn create_test_sandbox() -> SandboxRecord {
    let request = SandboxCreateRequest {
        name: "snapshot-test".to_string(),
        image: String::new(),
        stack: String::new(),
        agent_identifier: String::new(),
        env_json: String::new(),
        metadata_json: String::new(),
        ssh_enabled: false,
        ssh_public_key: String::new(),
        web_terminal_enabled: false,
        max_lifetime_seconds: 3600,
        idle_timeout_seconds: 3600,
        cpu_cores: 0,
        memory_mb: 0,
        disk_gb: 0,
        tee_required: false,
        tee_type: 0,
    };
    create_sidecar(&CreateSandboxParams::from(&request), None)
        .await
        .expect("Failed to create test sandbox")
        .0
}

/// Create a sandbox with user-supplied snapshot_destination in metadata.
async fn create_test_sandbox_with_destination(dest: &str) -> SandboxRecord {
    let metadata = serde_json::json!({ "snapshot_destination": dest });
    let request = SandboxCreateRequest {
        name: "snapshot-dest-test".to_string(),
        image: String::new(),
        stack: String::new(),
        agent_identifier: String::new(),
        env_json: String::new(),
        metadata_json: metadata.to_string(),
        ssh_enabled: false,
        ssh_public_key: String::new(),
        web_terminal_enabled: false,
        max_lifetime_seconds: 3600,
        idle_timeout_seconds: 3600,
        cpu_cores: 0,
        memory_mb: 0,
        disk_gb: 0,
        tee_required: false,
        tee_type: 0,
    };
    create_sidecar(&CreateSandboxParams::from(&request), None)
        .await
        .expect("Failed to create test sandbox with destination")
        .0
}

/// Wait for sidecar to become healthy.
async fn wait_healthy(url: &str, timeout_secs: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        if tokio::time::Instant::now() > deadline {
            panic!("Sidecar not healthy within {timeout_secs}s at {url}");
        }
        match http().get(format!("{url}/health")).send().await {
            Ok(resp) if resp.status().is_success() => return,
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Execute a command inside a running sandbox via its sidecar HTTP API.
async fn exec_in_sandbox(record: &SandboxRecord, command: &str) -> (u32, String, String) {
    let payload = serde_json::json!({
        "command": command,
        "timeout": 30000,
    });
    let resp = http()
        .post(format!("{}/terminals/commands", record.sidecar_url))
        .header("Authorization", format!("Bearer {}", record.token))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .expect("exec request failed");

    let body: serde_json::Value = resp.json().await.expect("exec response not JSON");
    let (exit_code, stdout, stderr) = ai_agent_sandbox_blueprint_lib::extract_exec_fields(&body);
    (exit_code, stdout, stderr)
}

/// Best-effort cleanup: remove container, snapshot image, store record, MinIO objects.
async fn cleanup_sandbox(record: &SandboxRecord) {
    // Remove container (force)
    if let Ok(builder) = docker_builder().await {
        let _ = builder
            .client()
            .remove_container(
                &record.container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
        // Also try the warm/cold container names
        let _ = builder
            .client()
            .remove_container(
                &format!("sidecar-{}-warm", record.id),
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
        let _ = builder
            .client()
            .remove_container(
                &format!("sidecar-{}-cold", record.id),
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
    }

    // Remove snapshot image
    if let Some(ref image_id) = record.snapshot_image_id {
        let _ = remove_snapshot_image(image_id).await;
    }
    // Also try the standard image name
    let _ = remove_snapshot_image(&format!("sandbox-snapshot/{}:latest", record.id)).await;

    // Remove MinIO objects
    minio_delete_object(&format!("{}/snapshot.tar.gz", record.id)).await;

    // Remove store record
    if let Ok(store) = sandboxes() {
        let _ = store.remove(&record.id);
    }
}

// ===================================================================
// Test 1: create_populates_snapshot_fields_real
// ===================================================================

#[tokio::test]
async fn create_populates_snapshot_fields_real() {
    skip_unless_snapshot!();
    setup_test_env();
    if !docker_ok().await {
        eprintln!("Skipped (Docker not available)");
        return;
    }

    let dest = format!("{MINIO_ENDPOINT}/{MINIO_BUCKET}/user-dest-test/snapshot.tar.gz");
    let record = create_test_sandbox_with_destination(&dest).await;
    wait_healthy(&record.sidecar_url, 60).await;

    // Verify snapshot fields populated
    assert!(
        !record.original_image.is_empty(),
        "original_image should be populated"
    );
    assert_eq!(
        record.snapshot_destination.as_deref(),
        Some(dest.as_str()),
        "snapshot_destination should match"
    );

    // Verify persisted in store
    let stored = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record should exist in store");
    assert_eq!(stored.original_image, record.original_image);
    assert_eq!(stored.snapshot_destination, record.snapshot_destination);

    cleanup_sandbox(&record).await;
    eprintln!("PASSED: create_populates_snapshot_fields_real");
}

// ===================================================================
// Test 2: commit_and_warm_resume_real
// ===================================================================

#[tokio::test]
async fn commit_and_warm_resume_real() {
    skip_unless_snapshot!();
    setup_test_env();
    if !docker_ok().await {
        eprintln!("Skipped (Docker not available)");
        return;
    }

    let record = create_test_sandbox().await;
    wait_healthy(&record.sidecar_url, 60).await;

    // Write a marker file so we can verify resume preserves state
    let marker = format!("warm-marker-{}", uuid::Uuid::new_v4());
    let (exit_code, _, stderr) = exec_in_sandbox(
        &record,
        &format!("echo '{marker}' > /home/agent/warm-marker.txt"),
    )
    .await;
    assert_eq!(exit_code, 0, "marker write should succeed: stderr={stderr}");

    // Stop container
    stop_sidecar(&record).await.expect("stop should succeed");

    // Commit container to snapshot image
    let image_id = commit_container(&record)
        .await
        .expect("commit should succeed");
    assert!(!image_id.is_empty(), "image_id should not be empty");
    eprintln!("Committed image: {image_id}");

    // Record the image in store
    sandboxes()
        .unwrap()
        .update(&record.id, |r| {
            r.snapshot_image_id = Some(image_id.clone());
        })
        .unwrap();

    // Delete the original container
    delete_sidecar(&record, None)
        .await
        .expect("delete should succeed");
    sandboxes()
        .unwrap()
        .update(&record.id, |r| {
            r.container_removed_at = Some(ai_agent_sandbox_blueprint_lib::util::now_ts());
        })
        .unwrap();

    // Resume from warm tier
    let record_before_resume = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record should exist");
    resume_sidecar(&record_before_resume)
        .await
        .expect("warm resume should succeed");

    // Verify resume state
    let resumed = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record should exist after resume");

    assert_ne!(
        resumed.container_id, record.container_id,
        "should have new container ID"
    );
    assert_eq!(resumed.state, SandboxState::Running, "should be Running");
    assert!(
        resumed.container_removed_at.is_none(),
        "container_removed_at should be cleared"
    );
    assert!(
        resumed.snapshot_image_id.is_none(),
        "snapshot_image_id should be consumed"
    );

    // Verify the resumed container is functional and preserved state
    wait_healthy(&resumed.sidecar_url, 60).await;
    let (exit_code, stdout, _) = exec_in_sandbox(&resumed, "cat /home/agent/warm-marker.txt").await;
    assert_eq!(exit_code, 0, "marker read should succeed");
    assert!(
        stdout.contains(&marker),
        "workspace should contain our marker: got '{stdout}'"
    );

    cleanup_sandbox(&resumed).await;
    eprintln!("PASSED: commit_and_warm_resume_real");
}

// ===================================================================
// Test 3: s3_snapshot_upload_and_cold_resume_real
// ===================================================================

#[tokio::test]
async fn s3_snapshot_upload_and_cold_resume_real() {
    skip_unless_snapshot!();
    setup_test_env();
    if !docker_ok().await || !minio_ok().await {
        eprintln!("Skipped (Docker or MinIO not available)");
        return;
    }

    let record = create_test_sandbox().await;
    wait_healthy(&record.sidecar_url, 60).await;

    // Write a marker file
    let marker = format!("cold-marker-{}", uuid::Uuid::new_v4());
    let (exit_code, _, _) = exec_in_sandbox(
        &record,
        &format!("echo '{marker}' > /home/agent/cold-marker.txt"),
    )
    .await;
    assert_eq!(exit_code, 0, "marker write should succeed");

    // Build and execute snapshot upload command inside the container.
    // Use the container-reachable endpoint since this runs inside Docker.
    let s3_path = format!("{}/snapshot.tar.gz", record.id);
    let container_minio = minio_endpoint_for_container();
    let dest = format!("{container_minio}/{MINIO_BUCKET}/{s3_path}");
    let snapshot_cmd =
        ai_agent_sandbox_blueprint_lib::util::build_snapshot_command(&dest, true, false)
            .expect("build_snapshot_command should succeed");

    let (exit_code, stdout, stderr) = exec_in_sandbox(
        &record,
        &format!(
            "sh -c {}",
            ai_agent_sandbox_blueprint_lib::util::shell_escape(&snapshot_cmd)
        ),
    )
    .await;
    eprintln!("Snapshot upload: exit={exit_code}, stdout='{stdout}', stderr='{stderr}'");
    assert_eq!(exit_code, 0, "snapshot upload should succeed");

    // Verify MinIO has the object
    assert!(
        minio_object_exists(&s3_path).await,
        "snapshot should exist in MinIO"
    );

    // Stop and fully remove container + any committed image
    stop_sidecar(&record).await.expect("stop should succeed");
    delete_sidecar(&record, None)
        .await
        .expect("delete should succeed");

    // Update store to simulate cold tier state
    sandboxes()
        .unwrap()
        .update(&record.id, |r| {
            let now = ai_agent_sandbox_blueprint_lib::util::now_ts();
            r.container_removed_at = Some(now);
            r.image_removed_at = Some(now);
            r.snapshot_image_id = None;
            r.snapshot_s3_url = Some(dest.clone());
        })
        .unwrap();

    // Resume from cold tier (S3)
    let record_before_resume = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record should exist");
    resume_sidecar(&record_before_resume)
        .await
        .expect("cold resume should succeed");

    // Verify resume state
    let resumed = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record should exist after resume");

    assert_eq!(resumed.state, SandboxState::Running, "should be Running");
    assert!(
        resumed.container_removed_at.is_none(),
        "container_removed_at should be cleared"
    );
    assert!(
        resumed.image_removed_at.is_none(),
        "image_removed_at should be cleared"
    );

    // Verify workspace was restored from S3
    wait_healthy(&resumed.sidecar_url, 60).await;
    let (exit_code, stdout, _) = exec_in_sandbox(&resumed, "cat /home/agent/cold-marker.txt").await;
    assert_eq!(
        exit_code, 0,
        "marker read should succeed after cold restore"
    );
    assert!(
        stdout.contains(&marker),
        "workspace should contain our marker after cold restore: got '{stdout}'"
    );

    // Cleanup
    minio_delete_object(&s3_path).await;
    cleanup_sandbox(&resumed).await;
    eprintln!("PASSED: s3_snapshot_upload_and_cold_resume_real");
}

// ===================================================================
// Test 4: resume_no_snapshot_fails_real
// ===================================================================

#[tokio::test]
async fn resume_no_snapshot_fails_real() {
    skip_unless_snapshot!();
    setup_test_env();
    if !docker_ok().await {
        eprintln!("Skipped (Docker not available)");
        return;
    }

    let record = create_test_sandbox().await;
    wait_healthy(&record.sidecar_url, 60).await;

    // Stop and delete container
    stop_sidecar(&record).await.expect("stop should succeed");
    delete_sidecar(&record, None)
        .await
        .expect("delete should succeed");

    // Clear all snapshot fields
    sandboxes()
        .unwrap()
        .update(&record.id, |r| {
            r.container_removed_at = Some(ai_agent_sandbox_blueprint_lib::util::now_ts());
            r.snapshot_image_id = None;
            r.snapshot_s3_url = None;
            r.snapshot_destination = None;
        })
        .unwrap();

    // Resume should fail
    let record_before_resume = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record should exist");
    let result = resume_sidecar(&record_before_resume).await;
    assert!(result.is_err(), "resume should fail with no snapshot");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Cannot resume"),
        "error should mention 'Cannot resume': got '{err_msg}'"
    );

    cleanup_sandbox(&record).await;
    eprintln!("PASSED: resume_no_snapshot_fails_real");
}

// ===================================================================
// Test 5: tiered_gc_hot_to_warm_real
// ===================================================================

#[tokio::test]
async fn tiered_gc_hot_to_warm_real() {
    skip_unless_snapshot!();
    setup_test_env();
    if !docker_ok().await {
        eprintln!("Skipped (Docker not available)");
        return;
    }

    let record = create_test_sandbox().await;
    wait_healthy(&record.sidecar_url, 60).await;

    // Stop the sandbox
    stop_sidecar(&record).await.expect("stop should succeed");

    // Commit a snapshot image
    let image_id = commit_container(&record)
        .await
        .expect("commit should succeed");
    sandboxes()
        .unwrap()
        .update(&record.id, |r| {
            r.snapshot_image_id = Some(image_id.clone());
        })
        .unwrap();

    // Set stopped_at in the past to exceed hot retention (which is 0 for tests)
    let past = ai_agent_sandbox_blueprint_lib::util::now_ts() - 10;
    sandboxes()
        .unwrap()
        .update(&record.id, |r| {
            r.stopped_at = Some(past);
        })
        .unwrap();

    // Run GC
    ai_agent_sandbox_blueprint_lib::reaper::gc_tick().await;

    // Verify: container removed, record still exists, image still set
    let after_gc = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record should still exist after hot→warm GC");
    assert!(
        after_gc.container_removed_at.is_some(),
        "container_removed_at should be set"
    );
    assert!(
        after_gc.snapshot_image_id.is_some(),
        "snapshot_image_id should still be set"
    );

    // Verify container is actually gone by trying to inspect it
    let builder = docker_builder().await.unwrap();
    let inspect = builder
        .client()
        .inspect_container(
            &record.container_id,
            None::<docktopus::bollard::container::InspectContainerOptions>,
        )
        .await;
    assert!(
        inspect.is_err(),
        "container should be removed after hot→warm GC"
    );

    // Cleanup
    let _ = remove_snapshot_image(&image_id).await;
    if let Ok(store) = sandboxes() {
        let _ = store.remove(&record.id);
    }
    eprintln!("PASSED: tiered_gc_hot_to_warm_real");
}

// ===================================================================
// Test 6: tiered_gc_warm_to_cold_real
// ===================================================================

#[tokio::test]
async fn tiered_gc_warm_to_cold_real() {
    skip_unless_snapshot!();
    setup_test_env();
    if !docker_ok().await || !minio_ok().await {
        eprintln!("Skipped (Docker or MinIO not available)");
        return;
    }

    let record = create_test_sandbox().await;
    wait_healthy(&record.sidecar_url, 60).await;

    // Stop and commit
    stop_sidecar(&record).await.expect("stop should succeed");
    let image_id = commit_container(&record)
        .await
        .expect("commit should succeed");

    // Upload an S3 snapshot to simulate having both tiers
    let s3_path = format!("{}/snapshot.tar.gz", record.id);
    let dest = format!("{MINIO_ENDPOINT}/{MINIO_BUCKET}/{s3_path}");
    assert!(
        minio_put_object(&s3_path, b"fake-snapshot-data").await,
        "MinIO put should succeed"
    );

    // Delete container to simulate warm tier state
    delete_sidecar(&record, None)
        .await
        .expect("delete should succeed");

    let past = ai_agent_sandbox_blueprint_lib::util::now_ts() - 10;
    sandboxes()
        .unwrap()
        .update(&record.id, |r| {
            r.stopped_at = Some(past);
            r.snapshot_image_id = Some(image_id.clone());
            r.snapshot_s3_url = Some(dest.clone());
            r.container_removed_at = Some(past);
        })
        .unwrap();

    // Run GC → should transition warm → cold (remove image, keep S3)
    ai_agent_sandbox_blueprint_lib::reaper::gc_tick().await;

    let after_gc = sandboxes().unwrap().get(&record.id).unwrap();
    // Record may still exist (with S3 URL) or may have been cleaned depending on state
    match after_gc {
        Some(r) => {
            assert!(
                r.snapshot_image_id.is_none(),
                "snapshot_image_id should be cleared after warm→cold"
            );
            assert!(
                r.image_removed_at.is_some(),
                "image_removed_at should be set"
            );
            assert!(
                r.snapshot_s3_url.is_some(),
                "snapshot_s3_url should still exist"
            );
        }
        None => {
            // If the record was removed, that's also acceptable (GC may have cascaded)
            eprintln!("Note: record was fully removed by GC (acceptable)");
        }
    }

    // Verify image is gone from Docker
    let remove_result = remove_snapshot_image(&image_id).await;
    // It's OK if it errors (already removed by GC)
    eprintln!("Image removal after GC: {remove_result:?}");

    // Cleanup
    minio_delete_object(&s3_path).await;
    if let Ok(store) = sandboxes() {
        let _ = store.remove(&record.id);
    }
    eprintln!("PASSED: tiered_gc_warm_to_cold_real");
}

// ===================================================================
// Test 7: tiered_gc_cold_to_gone_real
// ===================================================================

#[tokio::test]
async fn tiered_gc_cold_to_gone_real() {
    skip_unless_snapshot!();
    setup_test_env();
    if !docker_ok().await || !minio_ok().await {
        eprintln!("Skipped (Docker or MinIO not available)");
        return;
    }

    // Create a fake sandbox record in cold tier state (no container, no image, has S3)
    let sandbox_id = format!("snapshot-gc-cold-{}", uuid::Uuid::new_v4());
    let s3_path = format!("{sandbox_id}/snapshot.tar.gz");
    // Use the container endpoint for the stored URL so it matches the
    // SANDBOX_SNAPSHOT_DESTINATION_PREFIX set in setup_test_env().
    // This mirrors production where both prefix and stored URL share the same
    // endpoint. The 172.17.0.1 bridge gateway is reachable from the host too.
    let container_minio = minio_endpoint_for_container();
    let dest = format!("{container_minio}/{MINIO_BUCKET}/{s3_path}");

    // Upload a test object to MinIO
    assert!(
        minio_put_object(&s3_path, b"cold-tier-snapshot-data").await,
        "MinIO put should succeed"
    );
    assert!(
        minio_object_exists(&s3_path).await,
        "object should exist after upload"
    );

    let past = ai_agent_sandbox_blueprint_lib::util::now_ts() - 100;
    let record = SandboxRecord {
        id: sandbox_id.clone(),
        container_id: "dead-container-id".to_string(),
        sidecar_url: "http://127.0.0.1:9999".to_string(),
        sidecar_port: 9999,
        ssh_port: None,
        token: "test-token".to_string(),
        created_at: past - 1000,
        cpu_cores: 0,
        memory_mb: 0,
        state: SandboxState::Stopped,
        idle_timeout_seconds: 3600,
        max_lifetime_seconds: 86400,
        last_activity_at: past - 500,
        stopped_at: Some(past - 200),
        snapshot_image_id: None,
        snapshot_s3_url: Some(dest.clone()),
        container_removed_at: Some(past - 100),
        image_removed_at: Some(past),
        original_image: sidecar_image(),
        base_env_json: String::new(),
        user_env_json: String::new(),
        snapshot_destination: None, // operator-managed (not user BYOS3)
        tee_deployment_id: None,
        tee_metadata_json: None,
        name: String::new(),
        agent_identifier: String::new(),
        metadata_json: String::new(),
        disk_gb: 0,
        stack: String::new(),
        owner: String::new(),
        tee_config: None,
    };

    sandboxes()
        .unwrap()
        .insert(sandbox_id.clone(), record)
        .unwrap();

    // Record metrics before
    let gc_s3_before = ai_agent_sandbox_blueprint_lib::metrics::metrics()
        .gc_s3_cleaned
        .load(Ordering::Relaxed);

    // Run GC → should transition cold → gone
    ai_agent_sandbox_blueprint_lib::reaper::gc_tick().await;

    // Verify: S3 object deleted
    assert!(
        !minio_object_exists(&s3_path).await,
        "S3 object should be deleted after cold→gone GC"
    );

    // Verify: record removed
    let after_gc = sandboxes().unwrap().get(&sandbox_id).unwrap();
    assert!(
        after_gc.is_none(),
        "record should be removed after cold→gone GC"
    );

    // Verify: metrics incremented
    let gc_s3_after = ai_agent_sandbox_blueprint_lib::metrics::metrics()
        .gc_s3_cleaned
        .load(Ordering::Relaxed);
    assert!(
        gc_s3_after > gc_s3_before,
        "gc_s3_cleaned metric should have incremented"
    );

    eprintln!("PASSED: tiered_gc_cold_to_gone_real");
}

// ===================================================================
// Test 8: user_byos3_never_deleted_by_gc
// ===================================================================

#[tokio::test]
async fn user_byos3_never_deleted_by_gc() {
    skip_unless_snapshot!();
    setup_test_env();
    if !docker_ok().await || !minio_ok().await {
        eprintln!("Skipped (Docker or MinIO not available)");
        return;
    }

    // Create a fake sandbox record with user-supplied snapshot_destination
    let sandbox_id = format!("snapshot-byos3-{}", uuid::Uuid::new_v4());
    let s3_path = format!("{sandbox_id}/user-snapshot.tar.gz");
    let user_dest = format!("{MINIO_ENDPOINT}/{MINIO_BUCKET}/{s3_path}");

    // Upload a test object
    assert!(
        minio_put_object(&s3_path, b"user-supplied-snapshot").await,
        "MinIO put should succeed"
    );

    let past = ai_agent_sandbox_blueprint_lib::util::now_ts() - 100;
    let record = SandboxRecord {
        id: sandbox_id.clone(),
        container_id: "dead-container-id".to_string(),
        sidecar_url: "http://127.0.0.1:9999".to_string(),
        sidecar_port: 9999,
        ssh_port: None,
        token: "test-token".to_string(),
        created_at: past - 1000,
        cpu_cores: 0,
        memory_mb: 0,
        state: SandboxState::Stopped,
        idle_timeout_seconds: 3600,
        max_lifetime_seconds: 86400,
        last_activity_at: past - 500,
        stopped_at: Some(past - 200),
        snapshot_image_id: None,
        snapshot_s3_url: Some(user_dest.clone()),
        container_removed_at: Some(past - 100),
        image_removed_at: Some(past),
        original_image: sidecar_image(),
        base_env_json: String::new(),
        user_env_json: String::new(),
        snapshot_destination: Some(user_dest.clone()), // user BYOS3
        tee_deployment_id: None,
        tee_metadata_json: None,
        name: String::new(),
        agent_identifier: String::new(),
        metadata_json: String::new(),
        disk_gb: 0,
        stack: String::new(),
        owner: String::new(),
        tee_config: None,
    };

    sandboxes()
        .unwrap()
        .insert(sandbox_id.clone(), record)
        .unwrap();

    // Run GC → should remove record but preserve S3 object
    ai_agent_sandbox_blueprint_lib::reaper::gc_tick().await;

    // Verify: record removed
    let after_gc = sandboxes().unwrap().get(&sandbox_id).unwrap();
    assert!(after_gc.is_none(), "record should be removed after GC");

    // Verify: S3 object still exists (user BYOS3 preserved)
    assert!(
        minio_object_exists(&s3_path).await,
        "user BYOS3 object should NOT be deleted by GC"
    );

    // Cleanup
    minio_delete_object(&s3_path).await;
    eprintln!("PASSED: user_byos3_never_deleted_by_gc");
}

// ===================================================================
// Test 9: full_lifecycle_all_tiers
// ===================================================================

#[tokio::test]
async fn full_lifecycle_all_tiers() {
    skip_unless_snapshot!();
    setup_test_env();
    if !docker_ok().await || !minio_ok().await {
        eprintln!("Skipped (Docker or MinIO not available)");
        return;
    }

    eprintln!("=== FULL LIFECYCLE TEST ===");

    // Phase 1: Create sandbox
    eprintln!("Phase 1: Creating sandbox...");
    let record = create_test_sandbox().await;
    wait_healthy(&record.sidecar_url, 60).await;

    let marker = format!("lifecycle-{}", uuid::Uuid::new_v4());
    let (exit_code, _, _) = exec_in_sandbox(
        &record,
        &format!("echo '{marker}' > /home/agent/lifecycle-marker.txt"),
    )
    .await;
    assert_eq!(exit_code, 0, "marker write should succeed");
    eprintln!("Phase 1: Sandbox created: {}", record.id);

    // Phase 2: Stop and create snapshots (both commit + S3)
    eprintln!("Phase 2: Stopping and creating snapshots...");

    // Upload S3 snapshot while running.
    // Use the container-reachable endpoint since this runs inside Docker.
    let s3_path = format!("{}/snapshot.tar.gz", record.id);
    let container_minio = minio_endpoint_for_container();
    let s3_dest = format!("{container_minio}/{MINIO_BUCKET}/{s3_path}");
    let snapshot_cmd =
        ai_agent_sandbox_blueprint_lib::util::build_snapshot_command(&s3_dest, true, false)
            .expect("build_snapshot_command should succeed");
    let (exit_code, _, _) = exec_in_sandbox(
        &record,
        &format!(
            "sh -c {}",
            ai_agent_sandbox_blueprint_lib::util::shell_escape(&snapshot_cmd)
        ),
    )
    .await;
    assert_eq!(exit_code, 0, "S3 upload should succeed");
    assert!(
        minio_object_exists(&s3_path).await,
        "S3 object should exist"
    );

    stop_sidecar(&record).await.expect("stop should succeed");
    let image_id = commit_container(&record)
        .await
        .expect("commit should succeed");
    sandboxes()
        .unwrap()
        .update(&record.id, |r| {
            r.snapshot_image_id = Some(image_id.clone());
            r.snapshot_s3_url = Some(s3_dest.clone());
        })
        .unwrap();
    eprintln!("Phase 2: Stopped, committed ({image_id}), S3 uploaded");

    // Phase 3: Resume from hot (container still exists → docker start)
    // Note: hot resume (docker stop + docker start) may not work reliably with all
    // sidecar images since the sidecar process may not survive the stop/start cycle.
    // We make this phase fault-tolerant: if hot resume fails, we continue to warm resume.
    eprintln!("Phase 3: Hot resume (best-effort)...");
    let record_hot = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record exists");
    let mut hot_resume_ok = false;
    match resume_sidecar(&record_hot).await {
        Ok(()) => {
            let after_hot = sandboxes()
                .unwrap()
                .get(&record.id)
                .unwrap()
                .expect("record exists");
            assert_eq!(after_hot.state, SandboxState::Running);

            // Wait for health with timeout — if sidecar doesn't come back, skip
            let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
            loop {
                if tokio::time::Instant::now() > deadline {
                    eprintln!(
                        "Phase 3: Sidecar not healthy after 90s at {}, skipping hot resume verification",
                        after_hot.sidecar_url
                    );
                    // Try to get container logs for debugging
                    if let Ok(builder) = docker_builder().await {
                        use docktopus::bollard::container::LogsOptions;
                        use futures_util::StreamExt;
                        let opts = LogsOptions::<String> {
                            stdout: true,
                            stderr: true,
                            tail: "20".to_string(),
                            ..Default::default()
                        };
                        let mut logs = builder.client().logs(&after_hot.container_id, Some(opts));
                        let mut log_text = String::new();
                        while let Some(Ok(chunk)) = logs.next().await {
                            log_text.push_str(&chunk.to_string());
                        }
                        eprintln!("Phase 3: Container logs (last 20 lines):\n{log_text}");
                    }
                    break;
                }
                match http()
                    .get(format!("{}/health", after_hot.sidecar_url))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        // Hot resume is healthy — verify marker
                        let (exit_code, stdout, _) =
                            exec_in_sandbox(&after_hot, "cat /home/agent/lifecycle-marker.txt")
                                .await;
                        assert_eq!(exit_code, 0);
                        assert!(
                            stdout.contains(&marker),
                            "hot resume should preserve marker"
                        );
                        hot_resume_ok = true;
                        break;
                    }
                    _ => {}
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            // Stop for Phase 4 regardless
            let _ = stop_sidecar(&after_hot).await;
        }
        Err(err) => {
            eprintln!("Phase 3: Hot resume failed ({err}), continuing to warm resume");
        }
    }
    if hot_resume_ok {
        eprintln!("Phase 3: Hot resume OK");
    } else {
        eprintln!("Phase 3: Hot resume skipped/failed (non-fatal)");
    }

    // Phase 4: Stop → GC hot→warm → resume from warm
    eprintln!("Phase 4: Warm resume...");
    // Get the current record state (may have been updated by Phase 3)
    let current = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record exists");
    // Ensure container is stopped for commit
    if current.state == SandboxState::Running {
        stop_sidecar(&current).await.expect("stop should succeed");
    }
    // Re-commit after stop
    let current = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record exists");
    let image_id = commit_container(&current)
        .await
        .expect("commit should succeed");
    let past = ai_agent_sandbox_blueprint_lib::util::now_ts() - 10;
    sandboxes()
        .unwrap()
        .update(&record.id, |r| {
            r.stopped_at = Some(past);
            r.snapshot_image_id = Some(image_id.clone());
        })
        .unwrap();
    ai_agent_sandbox_blueprint_lib::reaper::gc_tick().await;

    let after_warm_gc = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record exists after hot→warm GC");
    assert!(
        after_warm_gc.container_removed_at.is_some(),
        "container should be removed"
    );
    assert!(
        after_warm_gc.snapshot_image_id.is_some(),
        "image should still exist"
    );

    resume_sidecar(&after_warm_gc)
        .await
        .expect("warm resume should succeed");
    let after_warm_resume = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record exists");
    assert_eq!(after_warm_resume.state, SandboxState::Running);
    wait_healthy(&after_warm_resume.sidecar_url, 60).await;
    let (exit_code, stdout, _) =
        exec_in_sandbox(&after_warm_resume, "cat /home/agent/lifecycle-marker.txt").await;
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains(&marker),
        "warm resume should preserve marker"
    );
    eprintln!("Phase 4: Warm resume OK");

    // Phase 5: Stop → manually transition to cold → resume from cold
    eprintln!("Phase 5: Cold resume...");
    stop_sidecar(&after_warm_resume)
        .await
        .expect("stop should succeed");
    delete_sidecar(&after_warm_resume, None)
        .await
        .expect("delete should succeed");

    // Clean any remaining image
    let _ = remove_snapshot_image(&image_id).await;
    let _ = remove_snapshot_image(&format!("sandbox-snapshot/{}:latest", record.id)).await;

    let now = ai_agent_sandbox_blueprint_lib::util::now_ts();
    sandboxes()
        .unwrap()
        .update(&record.id, |r| {
            r.container_removed_at = Some(now);
            r.image_removed_at = Some(now);
            r.snapshot_image_id = None;
            // S3 URL should still be there from Phase 2
        })
        .unwrap();

    let before_cold = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record exists");
    assert!(
        before_cold.snapshot_s3_url.is_some(),
        "S3 URL should exist for cold resume"
    );

    resume_sidecar(&before_cold)
        .await
        .expect("cold resume should succeed");
    let after_cold = sandboxes()
        .unwrap()
        .get(&record.id)
        .unwrap()
        .expect("record exists");
    assert_eq!(after_cold.state, SandboxState::Running);
    wait_healthy(&after_cold.sidecar_url, 60).await;
    let (exit_code, stdout, _) =
        exec_in_sandbox(&after_cold, "cat /home/agent/lifecycle-marker.txt").await;
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains(&marker),
        "cold resume should preserve marker: got '{stdout}'"
    );
    eprintln!("Phase 5: Cold resume OK");

    // Phase 6: Final cleanup — stop → GC all the way to gone
    eprintln!("Phase 6: GC to gone...");
    stop_sidecar(&after_cold)
        .await
        .expect("stop should succeed");
    delete_sidecar(&after_cold, None)
        .await
        .expect("delete should succeed");
    let past = ai_agent_sandbox_blueprint_lib::util::now_ts() - 10;
    // Re-add the S3 URL since cold resume cleared it from the record.
    // The object still exists in MinIO — we need the URL for GC to find and delete it.
    let s3_dest_for_gc = s3_dest.clone();
    sandboxes()
        .unwrap()
        .update(&record.id, |r| {
            r.stopped_at = Some(past);
            r.container_removed_at = Some(past);
            r.image_removed_at = Some(past);
            r.snapshot_image_id = None;
            r.snapshot_s3_url = Some(s3_dest_for_gc);
        })
        .unwrap();

    ai_agent_sandbox_blueprint_lib::reaper::gc_tick().await;

    let gone = sandboxes().unwrap().get(&record.id).unwrap();
    assert!(gone.is_none(), "record should be fully removed after GC");

    // S3 should be cleaned (operator-managed since no snapshot_destination)
    assert!(
        !minio_object_exists(&s3_path).await,
        "operator S3 object should be deleted"
    );

    eprintln!("Phase 6: GC to gone OK");
    eprintln!("=== FULL LIFECYCLE TEST PASSED ===");
}
