use super::*;

/// Per-stage wall-clock timings for one sandbox create, measured with
/// `std::time::Instant` on the real path (no sampling, no estimation).
///
/// Additive API: existing callers keep using [`create_sidecar`] and never see
/// this struct; [`create_sidecar_timed`] returns it for benches and operators
/// that want the breakdown. Every successful create also logs the breakdown
/// once via `tracing` (see [`CreateTimings::log`]).
///
/// Stages a backend does not pass through stay `None` — today the Docker
/// backend fills all of its stages, while Firecracker/TEE creates fill only
/// the permit/admission/total fields measured in the shared entry point.
///
/// NOTE: `total` is create-return time, NOT "ready". Unless `ssh_enabled` is
/// set, the create path returns before the sidecar's `/health` endpoint
/// responds; callers that need readiness must poll
/// [`wait_for_sidecar_health`] themselves (the lifecycle bench in
/// `tests/lifecycle_bench.rs` does exactly that and reports the two numbers
/// separately).
#[derive(Clone, Debug, Default)]
pub struct CreateTimings {
    /// Wait to acquire the global `CREATION_PERMIT` (queueing behind other
    /// concurrent creates). ~0 when the host is idle.
    pub permit_wait: Option<Duration>,
    /// Admission control: per-sandbox maxima + the store admission scan.
    pub admission: Option<Duration>,
    /// Docker client connect + ping (`docker_builder`).
    pub docker_connect: Option<Duration>,
    /// `ensure_image_pulled` — real pull cost on the first create of the
    /// process when `SIDECAR_PULL_IMAGE=true`, ~0 afterwards (process-wide
    /// once-cell).
    pub image_pull: Option<Duration>,
    /// Docker-side container create (`Container::create`).
    pub container_create: Option<Duration>,
    /// Docker-side container start (`Container::start`).
    pub container_start: Option<Duration>,
    /// Host-port mapping resolution (container inspect retry loop).
    pub port_mapping: Option<Duration>,
    /// Post-start workspace bootstrap exec round-trips (chown/mkdir).
    pub bootstrap_exec: Option<Duration>,
    /// Record seal + persistent-store insert.
    pub store_insert: Option<Duration>,
    /// SSH readiness bootstrap; only present when `ssh_enabled`.
    pub ssh_ready: Option<Duration>,
    /// End-to-end create as observed by the caller, including permit wait.
    pub total: Duration,
}

fn push_stage(out: &mut String, name: &str, stage: Option<Duration>) {
    if let Some(d) = stage {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(name);
        out.push('=');
        out.push_str(&format!("{:.1}ms", d.as_secs_f64() * 1e3));
    }
}

impl CreateTimings {
    /// Compact `stage=12.3ms` summary of the populated stages, in path order.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        push_stage(&mut out, "permit_wait", self.permit_wait);
        push_stage(&mut out, "admission", self.admission);
        push_stage(&mut out, "docker_connect", self.docker_connect);
        push_stage(&mut out, "image_pull", self.image_pull);
        push_stage(&mut out, "container_create", self.container_create);
        push_stage(&mut out, "container_start", self.container_start);
        push_stage(&mut out, "port_mapping", self.port_mapping);
        push_stage(&mut out, "bootstrap_exec", self.bootstrap_exec);
        push_stage(&mut out, "store_insert", self.store_insert);
        push_stage(&mut out, "ssh_ready", self.ssh_ready);
        out
    }

    /// Emit the one-line create breakdown through the existing log facility.
    pub(crate) fn log(&self, sandbox_id: &str, backend: RuntimeBackend) {
        tracing::info!(
            sandbox_id,
            backend = ?backend,
            total_ms = format!("{:.1}", self.total.as_secs_f64() * 1e3).as_str(),
            stages = %self.summary(),
            "sandbox create stage timings"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_skips_absent_stages_and_orders_present_ones() {
        let timings = CreateTimings {
            docker_connect: Some(Duration::from_millis(5)),
            container_start: Some(Duration::from_millis(250)),
            total: Duration::from_millis(300),
            ..Default::default()
        };
        let summary = timings.summary();
        assert_eq!(summary, "docker_connect=5.0ms container_start=250.0ms");
    }

    #[test]
    fn summary_empty_when_no_stages_recorded() {
        assert_eq!(CreateTimings::default().summary(), "");
    }
}
