//! Lightweight metrics tracker for on-chain reporting via QoS.
//!
//! Stores atomic counters that can be read by the QoS integration in the
//! binary crate and pushed as on-chain metrics via `add_on_chain_metric()`.

use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Global metrics tracker using atomic counters.
///
/// All counters use relaxed ordering — they are approximate gauges/counters
/// read periodically by a background task, so strict ordering isn't needed.
pub struct OnChainMetrics {
    /// Total job executions (prompt + task + exec + batch) since startup.
    pub total_jobs: AtomicU64,
    /// Cumulative execution time across all jobs (milliseconds).
    pub total_duration_ms: AtomicU64,
    /// Total input tokens consumed across all jobs.
    pub total_input_tokens: AtomicU64,
    /// Total output tokens produced across all jobs.
    pub total_output_tokens: AtomicU64,
    /// Current number of active sandboxes (created - deleted).
    pub active_sandboxes: AtomicU64,
    /// Peak concurrent sandboxes observed.
    pub peak_sandboxes: AtomicU64,
    /// Current number of active sessions (approximated by in-flight jobs).
    pub active_sessions: AtomicU64,
    /// CPU cores allocated across all active sandboxes.
    pub allocated_cpu_cores: AtomicU64,
    /// Memory (MB) allocated across all active sandboxes.
    pub allocated_memory_mb: AtomicU64,
    /// Total failed jobs.
    pub failed_jobs: AtomicU64,
    /// Sandboxes reaped due to idle timeout.
    pub reaped_idle: AtomicU64,
    /// Sandboxes reaped due to max lifetime exceeded.
    pub reaped_lifetime: AtomicU64,
    /// Stopped sandboxes garbage collected past retention.
    pub garbage_collected: AtomicU64,
    /// Docker commits (snapshots) performed.
    pub snapshots_committed: AtomicU64,
    /// S3 snapshot uploads performed.
    pub snapshots_uploaded: AtomicU64,
    /// Hot->Warm GC transitions (containers removed).
    pub gc_containers_removed: AtomicU64,
    /// Warm->Cold GC transitions (images removed).
    pub gc_images_removed: AtomicU64,
    /// Cold->Gone GC transitions (S3 snapshots cleaned).
    pub gc_s3_cleaned: AtomicU64,
}

impl Default for OnChainMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl OnChainMetrics {
    pub const fn new() -> Self {
        Self {
            total_jobs: AtomicU64::new(0),
            total_duration_ms: AtomicU64::new(0),
            total_input_tokens: AtomicU64::new(0),
            total_output_tokens: AtomicU64::new(0),
            active_sandboxes: AtomicU64::new(0),
            peak_sandboxes: AtomicU64::new(0),
            active_sessions: AtomicU64::new(0),
            allocated_cpu_cores: AtomicU64::new(0),
            allocated_memory_mb: AtomicU64::new(0),
            failed_jobs: AtomicU64::new(0),
            reaped_idle: AtomicU64::new(0),
            reaped_lifetime: AtomicU64::new(0),
            garbage_collected: AtomicU64::new(0),
            snapshots_committed: AtomicU64::new(0),
            snapshots_uploaded: AtomicU64::new(0),
            gc_containers_removed: AtomicU64::new(0),
            gc_images_removed: AtomicU64::new(0),
            gc_s3_cleaned: AtomicU64::new(0),
        }
    }

    /// Record a completed job execution with token usage.
    pub fn record_job(&self, duration_ms: u64, input_tokens: u32, output_tokens: u32) {
        self.total_jobs.fetch_add(1, Ordering::Relaxed);
        self.total_duration_ms
            .fetch_add(duration_ms, Ordering::Relaxed);
        self.total_input_tokens
            .fetch_add(u64::from(input_tokens), Ordering::Relaxed);
        self.total_output_tokens
            .fetch_add(u64::from(output_tokens), Ordering::Relaxed);
    }

    /// Record a failed job.
    pub fn record_failure(&self) {
        self.failed_jobs.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a sandbox reaped due to idle timeout.
    pub fn record_reaped_idle(&self) {
        self.reaped_idle.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a sandbox reaped due to max lifetime exceeded.
    pub fn record_reaped_lifetime(&self) {
        self.reaped_lifetime.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a stopped sandbox garbage collected.
    pub fn record_garbage_collected(&self) {
        self.garbage_collected.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a docker commit (snapshot) performed.
    pub fn record_snapshot_committed(&self) {
        self.snapshots_committed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an S3 snapshot upload performed.
    pub fn record_snapshot_uploaded(&self) {
        self.snapshots_uploaded.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a Hot->Warm GC transition (container removed).
    pub fn record_gc_container_removed(&self) {
        self.gc_containers_removed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a Warm->Cold GC transition (image removed).
    pub fn record_gc_image_removed(&self) {
        self.gc_images_removed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a Cold->Gone GC transition (S3 snapshot cleaned).
    pub fn record_gc_s3_cleaned(&self) {
        self.gc_s3_cleaned.fetch_add(1, Ordering::Relaxed);
    }

    /// Record sandbox creation with its resource allocation.
    pub fn record_sandbox_created(&self, cpu_cores: u64, memory_mb: u64) {
        let current = self.active_sandboxes.fetch_add(1, Ordering::Relaxed) + 1;
        self.peak_sandboxes.fetch_max(current, Ordering::Relaxed);
        self.allocated_cpu_cores
            .fetch_add(cpu_cores, Ordering::Relaxed);
        self.allocated_memory_mb
            .fetch_add(memory_mb, Ordering::Relaxed);
    }

    /// Record sandbox deletion, releasing its resources.
    pub fn record_sandbox_deleted(&self, cpu_cores: u64, memory_mb: u64) {
        let _ = self
            .active_sandboxes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_sub(1))
            });
        let _ = self
            .allocated_cpu_cores
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_sub(cpu_cores))
            });
        let _ = self
            .allocated_memory_mb
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_sub(memory_mb))
            });
    }

    /// Start a session and return a guard that decrements on drop.
    pub fn session_guard(&'static self) -> SessionGuard {
        self.active_sessions.fetch_add(1, Ordering::Relaxed);
        SessionGuard(self)
    }

    /// Decrement active sessions.
    fn session_end(&self) {
        let _ = self
            .active_sessions
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_sub(1))
            });
    }

    /// Snapshot all metrics as key-value pairs for on-chain reporting.
    pub fn snapshot(&self) -> Vec<(String, u64)> {
        let total_jobs = self.total_jobs.load(Ordering::Relaxed);
        let avg_duration_ms = if total_jobs > 0 {
            self.total_duration_ms.load(Ordering::Relaxed) / total_jobs
        } else {
            0
        };

        vec![
            ("total_jobs".into(), total_jobs),
            ("avg_duration_ms".into(), avg_duration_ms),
            (
                "total_input_tokens".into(),
                self.total_input_tokens.load(Ordering::Relaxed),
            ),
            (
                "total_output_tokens".into(),
                self.total_output_tokens.load(Ordering::Relaxed),
            ),
            (
                "active_sandboxes".into(),
                self.active_sandboxes.load(Ordering::Relaxed),
            ),
            (
                "peak_sandboxes".into(),
                self.peak_sandboxes.load(Ordering::Relaxed),
            ),
            (
                "active_sessions".into(),
                self.active_sessions.load(Ordering::Relaxed),
            ),
            (
                "allocated_cpu_cores".into(),
                self.allocated_cpu_cores.load(Ordering::Relaxed),
            ),
            (
                "allocated_memory_mb".into(),
                self.allocated_memory_mb.load(Ordering::Relaxed),
            ),
            (
                "failed_jobs".into(),
                self.failed_jobs.load(Ordering::Relaxed),
            ),
            (
                "reaped_idle".into(),
                self.reaped_idle.load(Ordering::Relaxed),
            ),
            (
                "reaped_lifetime".into(),
                self.reaped_lifetime.load(Ordering::Relaxed),
            ),
            (
                "garbage_collected".into(),
                self.garbage_collected.load(Ordering::Relaxed),
            ),
            (
                "snapshots_committed".into(),
                self.snapshots_committed.load(Ordering::Relaxed),
            ),
            (
                "snapshots_uploaded".into(),
                self.snapshots_uploaded.load(Ordering::Relaxed),
            ),
            (
                "gc_containers_removed".into(),
                self.gc_containers_removed.load(Ordering::Relaxed),
            ),
            (
                "gc_images_removed".into(),
                self.gc_images_removed.load(Ordering::Relaxed),
            ),
            (
                "gc_s3_cleaned".into(),
                self.gc_s3_cleaned.load(Ordering::Relaxed),
            ),
        ]
    }

    /// Render all metrics in Prometheus text exposition format.
    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(2048);
        for (name, value) in self.snapshot() {
            let prom_name = format!("sandbox_{name}");
            let mtype = if name.starts_with("active_")
                || name.starts_with("allocated_")
                || name.starts_with("peak_")
            {
                "gauge"
            } else {
                "counter"
            };
            let _ = writeln!(out, "# TYPE {prom_name} {mtype}");
            let _ = writeln!(out, "{prom_name} {value}");
        }
        out
    }
}

/// Seconds since the process started (for health endpoint).
pub fn uptime_secs() -> u64 {
    static START: once_cell::sync::Lazy<Instant> = once_cell::sync::Lazy::new(Instant::now);
    START.elapsed().as_secs()
}

/// RAII guard that decrements `active_sessions` when dropped.
pub struct SessionGuard(&'static OnChainMetrics);

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.0.session_end();
    }
}

/// Global metrics instance.
static METRICS: OnChainMetrics = OnChainMetrics::new();

/// Returns the global metrics tracker.
pub fn metrics() -> &'static OnChainMetrics {
    &METRICS
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-endpoint HTTP metrics
// ─────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;
use std::sync::Mutex;

/// Predefined histogram bucket upper bounds (milliseconds).
///
/// Standard Prometheus-style buckets covering sub-millisecond to multi-second
/// latencies. The final `u64::MAX` bucket captures everything above 5000ms.
pub const HISTOGRAM_BUCKETS: [u64; 11] = [1, 5, 10, 25, 50, 100, 250, 500, 1000, 5000, u64::MAX];

/// Human-readable labels for Prometheus `le` tag on each bucket.
const BUCKET_LABELS: [&str; 11] = [
    "1", "5", "10", "25", "50", "100", "250", "500", "1000", "5000", "+Inf",
];

/// Per-endpoint request count and cumulative duration.
#[derive(Clone)]
pub struct EndpointStats {
    pub count: u64,
    pub total_ms: u64,
    /// Count of 5xx server errors.
    pub errors: u64,
    /// Count of 4xx client errors.
    pub client_errors: u64,
    /// Minimum observed request duration in milliseconds.
    pub min_duration_ms: u64,
    /// Maximum observed request duration in milliseconds.
    pub max_duration_ms: u64,
    /// Histogram bucket counters aligned with [`HISTOGRAM_BUCKETS`].
    pub histogram: [u64; 11],
}

impl Default for EndpointStats {
    fn default() -> Self {
        Self {
            count: 0,
            total_ms: 0,
            errors: 0,
            client_errors: 0,
            min_duration_ms: u64::MAX,
            max_duration_ms: 0,
            histogram: [0; 11],
        }
    }
}

/// Tracks per-endpoint HTTP latency and request counts.
pub struct HttpMetrics {
    endpoints: Mutex<HashMap<String, EndpointStats>>,
}

impl Default for HttpMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpMetrics {
    pub fn new() -> Self {
        Self {
            endpoints: Mutex::new(HashMap::new()),
        }
    }

    /// Record a request for `path` with given duration and error classification.
    pub fn record(
        &self,
        path: &str,
        duration_ms: u64,
        is_server_error: bool,
        is_client_error: bool,
    ) {
        let mut map = self.endpoints.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map.entry(path.to_string()).or_default();
        entry.count += 1;
        entry.total_ms += duration_ms;
        entry.min_duration_ms = std::cmp::min(entry.min_duration_ms, duration_ms);
        entry.max_duration_ms = std::cmp::max(entry.max_duration_ms, duration_ms);
        // Increment the first histogram bucket whose upper bound >= duration_ms.
        for (i, &bound) in HISTOGRAM_BUCKETS.iter().enumerate() {
            if duration_ms <= bound {
                entry.histogram[i] += 1;
                break;
            }
        }
        if is_server_error {
            entry.errors += 1;
        }
        if is_client_error {
            entry.client_errors += 1;
        }
    }

    /// Snapshot all endpoint stats for Prometheus rendering.
    pub fn snapshot(&self) -> Vec<(String, EndpointStats)> {
        let map = self.endpoints.lock().unwrap_or_else(|e| e.into_inner());
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Render per-endpoint metrics in Prometheus text exposition format.
    pub fn render_prometheus(&self) -> String {
        let snap = self.snapshot();
        if snap.is_empty() {
            return String::new();
        }
        let mut out = String::with_capacity(2048);
        let _ = writeln!(out, "# TYPE http_requests_total counter");
        let _ = writeln!(out, "# TYPE http_request_duration_ms_total counter");
        let _ = writeln!(out, "# TYPE http_request_errors_total counter");
        let _ = writeln!(out, "# TYPE http_request_client_errors_total counter");
        let _ = writeln!(out, "# TYPE http_request_duration_min_ms gauge");
        let _ = writeln!(out, "# TYPE http_request_duration_max_ms gauge");
        let _ = writeln!(out, "# TYPE http_request_duration_ms histogram");
        for (path, stats) in &snap {
            let _ = writeln!(
                out,
                "http_requests_total{{path=\"{path}\"}} {}",
                stats.count
            );
            let _ = writeln!(
                out,
                "http_request_duration_ms_total{{path=\"{path}\"}} {}",
                stats.total_ms
            );
            let min_val = if stats.count == 0 {
                0
            } else {
                stats.min_duration_ms
            };
            let _ = writeln!(
                out,
                "http_request_duration_min_ms{{path=\"{path}\"}} {min_val}",
            );
            let _ = writeln!(
                out,
                "http_request_duration_max_ms{{path=\"{path}\"}} {}",
                stats.max_duration_ms
            );
            // Histogram buckets (cumulative, as per Prometheus convention).
            let mut cumulative = 0u64;
            for (i, label) in BUCKET_LABELS.iter().enumerate() {
                cumulative += stats.histogram[i];
                let _ = writeln!(
                    out,
                    "http_request_duration_ms_bucket{{le=\"{label}\",path=\"{path}\"}} {cumulative}",
                );
            }
            let _ = writeln!(
                out,
                "http_request_duration_ms_sum{{path=\"{path}\"}} {}",
                stats.total_ms
            );
            let _ = writeln!(
                out,
                "http_request_duration_ms_count{{path=\"{path}\"}} {}",
                stats.count
            );
            if stats.errors > 0 {
                let _ = writeln!(
                    out,
                    "http_request_errors_total{{path=\"{path}\"}} {}",
                    stats.errors
                );
            }
            if stats.client_errors > 0 {
                let _ = writeln!(
                    out,
                    "http_request_client_errors_total{{path=\"{path}\"}} {}",
                    stats.client_errors
                );
            }
        }
        // Rate-limit rejection counter (global, not per-endpoint)
        let rl = rate_limit_rejections().load(Ordering::Relaxed);
        let _ = writeln!(out, "# TYPE rate_limit_rejections_total counter");
        let _ = writeln!(out, "rate_limit_rejections_total {rl}");
        out
    }
}

static HTTP_METRICS: once_cell::sync::Lazy<HttpMetrics> =
    once_cell::sync::Lazy::new(HttpMetrics::new);

/// Returns the global HTTP metrics tracker.
pub fn http_metrics() -> &'static HttpMetrics {
    &HTTP_METRICS
}

// ─────────────────────────────────────────────────────────────────────────────
// Rate-limit rejection counter
// ─────────────────────────────────────────────────────────────────────────────

/// Global counter of requests rejected by rate limiting.
static RATE_LIMIT_REJECTIONS: AtomicU64 = AtomicU64::new(0);

/// Returns the global rate-limit rejection counter.
pub fn rate_limit_rejections() -> &'static AtomicU64 {
    &RATE_LIMIT_REJECTIONS
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── OnChainMetrics ──────────────────────────────────────────────────

    #[test]
    fn snapshot_returns_all_zero_initially() {
        let m = OnChainMetrics::new();
        let snap = m.snapshot();
        for (name, value) in &snap {
            assert_eq!(*value, 0, "expected 0 for {name}");
        }
    }

    #[test]
    fn record_sandbox_created_increments_active_and_peak() {
        let m = OnChainMetrics::new();
        m.record_sandbox_created(2, 1024);
        m.record_sandbox_created(4, 2048);

        assert_eq!(m.active_sandboxes.load(Ordering::Relaxed), 2);
        assert_eq!(m.peak_sandboxes.load(Ordering::Relaxed), 2);
        assert_eq!(m.allocated_cpu_cores.load(Ordering::Relaxed), 6);
        assert_eq!(m.allocated_memory_mb.load(Ordering::Relaxed), 3072);
    }

    #[test]
    fn record_sandbox_deleted_decrements_active() {
        let m = OnChainMetrics::new();
        m.record_sandbox_created(2, 1024);
        m.record_sandbox_created(4, 2048);
        m.record_sandbox_deleted(2, 1024);

        assert_eq!(m.active_sandboxes.load(Ordering::Relaxed), 1);
        assert_eq!(m.allocated_cpu_cores.load(Ordering::Relaxed), 4);
        assert_eq!(m.allocated_memory_mb.load(Ordering::Relaxed), 2048);
        // Peak should remain at 2
        assert_eq!(m.peak_sandboxes.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn record_sandbox_deleted_saturates_at_zero() {
        let m = OnChainMetrics::new();
        // Delete without any creation — should not underflow
        m.record_sandbox_deleted(4, 2048);

        assert_eq!(m.active_sandboxes.load(Ordering::Relaxed), 0);
        assert_eq!(m.allocated_cpu_cores.load(Ordering::Relaxed), 0);
        assert_eq!(m.allocated_memory_mb.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn record_sandbox_deleted_saturates_resources() {
        let m = OnChainMetrics::new();
        m.record_sandbox_created(2, 512);
        // Delete more resources than were allocated
        m.record_sandbox_deleted(100, 9999);

        assert_eq!(m.active_sandboxes.load(Ordering::Relaxed), 0);
        assert_eq!(m.allocated_cpu_cores.load(Ordering::Relaxed), 0);
        assert_eq!(m.allocated_memory_mb.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn snapshot_after_jobs() {
        let m = OnChainMetrics::new();
        m.record_job(100, 500, 200);
        m.record_job(200, 300, 100);

        let snap: std::collections::HashMap<String, u64> = m.snapshot().into_iter().collect();

        assert_eq!(snap["total_jobs"], 2);
        assert_eq!(snap["avg_duration_ms"], 150); // (100 + 200) / 2
        assert_eq!(snap["total_input_tokens"], 800);
        assert_eq!(snap["total_output_tokens"], 300);
    }

    #[test]
    fn record_failure_increments() {
        let m = OnChainMetrics::new();
        m.record_failure();
        m.record_failure();
        assert_eq!(m.failed_jobs.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn record_reaper_and_gc_metrics() {
        let m = OnChainMetrics::new();
        m.record_reaped_idle();
        m.record_reaped_lifetime();
        m.record_garbage_collected();
        m.record_snapshot_committed();
        m.record_snapshot_uploaded();
        m.record_gc_container_removed();
        m.record_gc_image_removed();
        m.record_gc_s3_cleaned();

        assert_eq!(m.reaped_idle.load(Ordering::Relaxed), 1);
        assert_eq!(m.reaped_lifetime.load(Ordering::Relaxed), 1);
        assert_eq!(m.garbage_collected.load(Ordering::Relaxed), 1);
        assert_eq!(m.snapshots_committed.load(Ordering::Relaxed), 1);
        assert_eq!(m.snapshots_uploaded.load(Ordering::Relaxed), 1);
        assert_eq!(m.gc_containers_removed.load(Ordering::Relaxed), 1);
        assert_eq!(m.gc_images_removed.load(Ordering::Relaxed), 1);
        assert_eq!(m.gc_s3_cleaned.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn render_prometheus_on_chain_metrics() {
        let m = OnChainMetrics::new();
        m.record_sandbox_created(2, 1024);
        m.record_job(50, 100, 50);

        let output = m.render_prometheus();

        // Should contain TYPE declarations
        assert!(output.contains("# TYPE sandbox_total_jobs counter"));
        assert!(output.contains("# TYPE sandbox_active_sandboxes gauge"));
        assert!(output.contains("# TYPE sandbox_allocated_cpu_cores gauge"));
        assert!(output.contains("# TYPE sandbox_peak_sandboxes gauge"));

        // Should contain actual values
        assert!(output.contains("sandbox_total_jobs 1"));
        assert!(output.contains("sandbox_active_sandboxes 1"));
        assert!(output.contains("sandbox_allocated_cpu_cores 2"));
        assert!(output.contains("sandbox_allocated_memory_mb 1024"));
    }

    // ── HttpMetrics ─────────────────────────────────────────────────────

    #[test]
    fn http_metrics_record_increments() {
        let hm = HttpMetrics::new();
        hm.record("/api/test", 10, false, false);
        hm.record("/api/test", 20, false, false);

        let snap = hm.snapshot();
        assert_eq!(snap.len(), 1);
        let (path, stats) = &snap[0];
        assert_eq!(path, "/api/test");
        assert_eq!(stats.count, 2);
        assert_eq!(stats.total_ms, 30);
    }

    #[test]
    fn http_metrics_tracks_min_max() {
        let hm = HttpMetrics::new();
        hm.record("/api/foo", 50, false, false);
        hm.record("/api/foo", 10, false, false);
        hm.record("/api/foo", 200, false, false);

        let snap = hm.snapshot();
        let (_, stats) = &snap[0];
        assert_eq!(stats.min_duration_ms, 10);
        assert_eq!(stats.max_duration_ms, 200);
    }

    #[test]
    fn http_metrics_histogram_bucketing() {
        let hm = HttpMetrics::new();
        // Duration 1ms -> bucket[0] (le=1)
        hm.record("/api/h", 1, false, false);
        // Duration 50ms -> bucket[4] (le=50)
        hm.record("/api/h", 50, false, false);
        // Duration 999ms -> bucket[8] (le=1000)
        hm.record("/api/h", 999, false, false);
        // Duration 10000ms -> bucket[10] (le=+Inf / u64::MAX)
        hm.record("/api/h", 10000, false, false);

        let snap = hm.snapshot();
        let (_, stats) = &snap[0];
        assert_eq!(stats.histogram[0], 1); // le=1
        assert_eq!(stats.histogram[1], 0); // le=5
        assert_eq!(stats.histogram[2], 0); // le=10
        assert_eq!(stats.histogram[3], 0); // le=25
        assert_eq!(stats.histogram[4], 1); // le=50
        assert_eq!(stats.histogram[5], 0); // le=100
        assert_eq!(stats.histogram[6], 0); // le=250
        assert_eq!(stats.histogram[7], 0); // le=500
        assert_eq!(stats.histogram[8], 1); // le=1000
        assert_eq!(stats.histogram[9], 0); // le=5000
        assert_eq!(stats.histogram[10], 1); // le=+Inf
    }

    #[test]
    fn http_metrics_error_tracking() {
        let hm = HttpMetrics::new();
        hm.record("/api/err", 10, true, false);
        hm.record("/api/err", 10, false, true);
        hm.record("/api/err", 10, false, false);

        let snap = hm.snapshot();
        let (_, stats) = &snap[0];
        assert_eq!(stats.count, 3);
        assert_eq!(stats.errors, 1);
        assert_eq!(stats.client_errors, 1);
    }

    #[test]
    fn http_metrics_multiple_endpoints() {
        let hm = HttpMetrics::new();
        hm.record("/api/a", 10, false, false);
        hm.record("/api/b", 20, false, false);
        hm.record("/api/a", 30, false, false);

        let snap = hm.snapshot();
        assert_eq!(snap.len(), 2);

        let map: std::collections::HashMap<String, EndpointStats> = snap.into_iter().collect();
        assert_eq!(map["/api/a"].count, 2);
        assert_eq!(map["/api/b"].count, 1);
    }

    #[test]
    fn http_metrics_render_prometheus_empty() {
        let hm = HttpMetrics::new();
        let output = hm.render_prometheus();
        assert!(output.is_empty());
    }

    #[test]
    fn http_metrics_render_prometheus_format() {
        let hm = HttpMetrics::new();
        hm.record("/api/test", 42, true, false);

        let output = hm.render_prometheus();

        // TYPE declarations
        assert!(output.contains("# TYPE http_requests_total counter"));
        assert!(output.contains("# TYPE http_request_duration_ms histogram"));
        assert!(output.contains("# TYPE http_request_duration_min_ms gauge"));
        assert!(output.contains("# TYPE http_request_duration_max_ms gauge"));

        // Per-path metrics
        assert!(output.contains("http_requests_total{path=\"/api/test\"} 1"));
        assert!(output.contains("http_request_duration_ms_total{path=\"/api/test\"} 42"));
        assert!(output.contains("http_request_duration_min_ms{path=\"/api/test\"} 42"));
        assert!(output.contains("http_request_duration_max_ms{path=\"/api/test\"} 42"));

        // Histogram buckets (cumulative)
        assert!(output.contains("http_request_duration_ms_bucket{le=\"50\",path=\"/api/test\"} 1"));
        assert!(
            output.contains("http_request_duration_ms_bucket{le=\"+Inf\",path=\"/api/test\"} 1")
        );

        // Sum and count
        assert!(output.contains("http_request_duration_ms_sum{path=\"/api/test\"} 42"));
        assert!(output.contains("http_request_duration_ms_count{path=\"/api/test\"} 1"));

        // Server errors
        assert!(output.contains("http_request_errors_total{path=\"/api/test\"} 1"));

        // Rate limit counter
        assert!(output.contains("# TYPE rate_limit_rejections_total counter"));
        assert!(output.contains("rate_limit_rejections_total"));
    }

    #[test]
    fn http_metrics_snapshot_after_recording() {
        let hm = HttpMetrics::new();
        assert!(hm.snapshot().is_empty());

        hm.record("/health", 5, false, false);
        let snap = hm.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].0, "/health");
        assert_eq!(snap[0].1.count, 1);
        assert_eq!(snap[0].1.total_ms, 5);
    }

    #[test]
    fn endpoint_stats_default_min_is_max_u64() {
        let stats = EndpointStats::default();
        assert_eq!(stats.min_duration_ms, u64::MAX);
        assert_eq!(stats.max_duration_ms, 0);
        assert_eq!(stats.count, 0);
    }
}
