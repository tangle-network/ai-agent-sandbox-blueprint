//! Per-endpoint HTTP request metrics (latency + counters).

use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};

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
