//! Lightweight metrics tracker for on-chain reporting via QoS.
//!
//! Stores atomic counters that can be read by the QoS integration in the
//! binary crate and pushed as on-chain metrics via `add_on_chain_metric()`.

mod http;
mod onchain;

pub use http::*;
pub use onchain::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;

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
