use super::*;

/// Build a fresh Docker client for each call.
///
/// We intentionally do not cache the builder for the life of the process so
/// Docker Desktop socket or port-mapping state cannot go stale across long-lived
/// operator sessions.
pub async fn docker_builder() -> Result<DockerBuilder> {
    let config = SidecarRuntimeConfig::load();
    match config.docker_host.as_deref() {
        Some(host) => DockerBuilder::with_address(host).await.map_err(|err| {
            SandboxError::Docker(format!("Failed to connect to Docker at {host}: {err}"))
        }),
        None => DockerBuilder::new()
            .await
            .map_err(|err| SandboxError::Docker(format!("Failed to connect to Docker: {err}"))),
    }
}

pub(crate) fn detect_docker_host_fallback() -> Option<String> {
    let default_socket = std::path::Path::new("/var/run/docker.sock");
    if default_socket.exists() {
        return None;
    }

    let home = env::var("HOME").ok()?;
    let docker_desktop_socket = std::path::Path::new(&home).join(".docker/run/docker.sock");
    docker_desktop_socket
        .exists()
        .then(|| format!("unix://{}", docker_desktop_socket.display()))
}

/// Default timeout for Docker operations (seconds).
pub(crate) const DEFAULT_DOCKER_TIMEOUT_SECS: u64 = 60;

/// Wrap a Docker future in a timeout to prevent indefinite hangs.
///
/// Reads `DOCKER_OPERATION_TIMEOUT_SECS` env var (default: 60s).
pub(crate) async fn docker_timeout<F, T, E>(op_name: &str, future: F) -> Result<T>
where
    F: std::future::Future<Output = std::result::Result<T, E>>,
    E: std::fmt::Display,
{
    let timeout_secs = env::var("DOCKER_OPERATION_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_DOCKER_TIMEOUT_SECS);

    let start = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(timeout_secs), future).await;
    let duration_ms = start.elapsed().as_millis();
    tracing::debug!(op = %op_name, duration_ms = %duration_ms, "docker operation completed");

    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(SandboxError::Docker(format!("{op_name} failed: {e}"))),
        Err(_) => Err(SandboxError::Docker(format!(
            "{op_name} timed out after {timeout_secs}s"
        ))),
    }
}

/// Generic retry helper for Docker operations.
///
/// Retries `f` up to `max_retries` times with exponential backoff starting at
/// `backoff_ms`. On each failure (except the last), a warning is logged and the
/// operation is retried after sleeping.
pub(crate) async fn retry_docker<F, Fut, T>(
    op_name: &str,
    max_retries: u32,
    backoff_ms: u64,
    f: F,
) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut last_err = None;
    for attempt in 0..=max_retries {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                if attempt < max_retries {
                    tracing::warn!(
                        op = op_name,
                        attempt = attempt + 1,
                        error = %e,
                        "Docker operation failed, retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(backoff_ms * (attempt as u64 + 1)))
                        .await;
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        crate::error::SandboxError::Docker(format!(
            "{op_name}: all retries exhausted with no error"
        ))
    }))
}

/// Start a container with a single retry on transient failure.
///
/// Container starts occasionally fail due to Docker daemon contention or
/// transient resource issues. A single retry with 500ms backoff handles the
/// common case without adding excessive latency.
pub(crate) async fn start_container_with_retry(container: &mut Container) -> Result<()> {
    match docker_timeout("start_container", container.start(false)).await {
        Ok(()) => Ok(()),
        Err(first_err) => {
            tracing::warn!(
                error = %first_err,
                "Container start failed, retrying after 500ms"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
            docker_timeout("start_container_retry", container.start(false)).await
        }
    }
}

/// Best-effort removal of an orphaned container after a partial creation failure.
pub(crate) async fn cleanup_orphaned_container(builder: &DockerBuilder, container_id: &str) {
    tracing::warn!(
        container_id,
        "Cleaning up orphaned container after creation failure"
    );
    let timeout = std::time::Duration::from_secs(30);
    let result = tokio::time::timeout(timeout, async {
        if let Ok(c) = Container::from_id(builder.client(), container_id).await {
            let _ = c
                .remove(Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }))
                .await;
        }
    })
    .await;
    if result.is_err() {
        tracing::error!(container_id, "Orphan container cleanup timed out after 30s");
    }
}

/// Ensure the sidecar image is available locally. Pulls once on first call
/// if `SIDECAR_PULL_IMAGE` is true. Subsequent calls are no-ops.
///
/// Image pulls are retried up to 2 times with 1-second backoff to handle
/// transient registry errors.
pub(crate) async fn ensure_image_pulled(builder: &DockerBuilder, image: &str) -> Result<()> {
    IMAGE_PULLED
        .get_or_try_init(|| async {
            let config = SidecarRuntimeConfig::load();
            if config.pull_image {
                retry_docker("pull_image", 2, 1000, || {
                    docker_timeout("pull_image", builder.pull_image(image, None))
                })
                .await?;
            }
            Ok::<(), SandboxError>(())
        })
        .await?;
    Ok(())
}
