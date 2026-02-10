//! Lightweight metrics tracker for on-chain reporting via QoS.
//!
//! Stores atomic counters that can be read by the QoS integration in the
//! binary crate and pushed as on-chain metrics via `add_on_chain_metric()`.

use std::sync::atomic::{AtomicU64, Ordering};

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
    ///
    /// Guarantees `session_end` is called even on early returns, panics, or
    /// task cancellation — fixing the session counter leak audit finding.
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
        ]
    }
}

/// RAII guard that decrements `active_sessions` when dropped.
///
/// Prevents session counter leaks on early returns, panics, or task cancellation.
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
