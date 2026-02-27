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
