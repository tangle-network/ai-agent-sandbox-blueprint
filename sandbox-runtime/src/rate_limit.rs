//! Simple sliding-window rate limiter for the operator API.
//!
//! Uses an in-memory sliding window per client IP. Old entries are cleaned up
//! periodically to avoid unbounded memory growth.
//!
//! Two static limiters are provided:
//! - `read_limiter()`: 120 req/min — for GET endpoints
//! - `write_limiter()`: 30 req/min — for POST/DELETE endpoints
//!
//! Usage in operator_api router:
//! ```ignore
//! use axum::middleware;
//! router.layer(middleware::from_fn(rate_limit::read_rate_limit))
//! ```

use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Configuration for a rate limiter.
#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    /// Maximum requests allowed in the window.
    pub max_requests: u32,
    /// Window duration in seconds.
    pub window_secs: u64,
}

impl RateLimitConfig {
    pub const fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            max_requests,
            window_secs,
        }
    }
}

/// Per-IP request tracker.
struct Bucket {
    timestamps: Vec<Instant>,
}

impl Bucket {
    fn new() -> Self {
        Self {
            timestamps: Vec::new(),
        }
    }

    /// Prune timestamps older than the window, then check if a new request is allowed.
    fn check_and_record(&mut self, window_secs: u64, max_requests: u32) -> bool {
        let now = Instant::now();
        let cutoff = now - Duration::from_secs(window_secs);
        self.timestamps.retain(|t| *t > cutoff);

        if (self.timestamps.len() as u32) < max_requests {
            self.timestamps.push(now);
            true
        } else {
            false
        }
    }
}

/// Shared rate limiter state.
pub struct RateLimiter {
    config: RateLimitConfig,
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
    last_gc: Mutex<Instant>,
}

/// GC interval: clean up stale IPs every 5 minutes.
const GC_INTERVAL_SECS: u64 = 300;

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: Mutex::new(HashMap::new()),
            last_gc: Mutex::new(Instant::now()),
        }
    }

    /// Check whether a request from `ip` is allowed.
    pub fn check(&self, ip: IpAddr) -> bool {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());

        // Periodic GC of stale entries
        {
            let mut last_gc = self.last_gc.lock().unwrap_or_else(|e| e.into_inner());
            if last_gc.elapsed().as_secs() >= GC_INTERVAL_SECS {
                let cutoff = Instant::now() - Duration::from_secs(self.config.window_secs * 2);
                buckets.retain(|_, b| b.timestamps.last().is_some_and(|t| *t > cutoff));
                *last_gc = Instant::now();
            }
        }

        let bucket = buckets.entry(ip).or_insert_with(Bucket::new);
        bucket.check_and_record(self.config.window_secs, self.config.max_requests)
    }

    /// Number of tracked IPs (for metrics/debugging).
    pub fn tracked_ips(&self) -> usize {
        self.buckets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }
}

// ---------------------------------------------------------------------------
// Static limiters
// ---------------------------------------------------------------------------

static READ_LIMITER: once_cell::sync::Lazy<RateLimiter> =
    once_cell::sync::Lazy::new(|| RateLimiter::new(RateLimitConfig::new(120, 60)));

static WRITE_LIMITER: once_cell::sync::Lazy<RateLimiter> =
    once_cell::sync::Lazy::new(|| RateLimiter::new(RateLimitConfig::new(30, 60)));

/// Access the read-tier (120 req/min) limiter.
pub fn read_limiter() -> &'static RateLimiter {
    &READ_LIMITER
}

/// Access the write-tier (30 req/min) limiter.
pub fn write_limiter() -> &'static RateLimiter {
    &WRITE_LIMITER
}

// ---------------------------------------------------------------------------
// Axum middleware functions
// ---------------------------------------------------------------------------

/// Extract client IP from ConnectInfo or x-forwarded-for header.
fn extract_client_ip(req: &Request) -> Option<IpAddr> {
    // Try ConnectInfo first (direct connections)
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .or_else(|| {
            // Fall back to x-forwarded-for (behind reverse proxy)
            req.headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next())
                .and_then(|s| s.trim().parse().ok())
        })
}

/// Rate-limiting middleware for read (GET) endpoints.
/// Allows 120 requests per minute per IP.
pub async fn read_rate_limit(request: Request, next: Next) -> Response {
    if let Some(ip) = extract_client_ip(&request) {
        if !read_limiter().check(ip) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "60")],
                "Rate limit exceeded",
            )
                .into_response();
        }
    }
    next.run(request).await
}

/// Rate-limiting middleware for write (POST/PUT/DELETE) endpoints.
/// Allows 30 requests per minute per IP.
pub async fn write_rate_limit(request: Request, next: Next) -> Response {
    if let Some(ip) = extract_client_ip(&request) {
        if !write_limiter().check(ip) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "60")],
                "Rate limit exceeded",
            )
                .into_response();
        }
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_within_limit() {
        let limiter = RateLimiter::new(RateLimitConfig::new(3, 60));
        let ip: IpAddr = "127.0.0.1".parse().unwrap();

        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
        assert!(!limiter.check(ip)); // 4th request blocked
    }

    #[test]
    fn separate_ips_independent() {
        let limiter = RateLimiter::new(RateLimitConfig::new(1, 60));
        let ip1: IpAddr = "10.0.0.1".parse().unwrap();
        let ip2: IpAddr = "10.0.0.2".parse().unwrap();

        assert!(limiter.check(ip1));
        assert!(!limiter.check(ip1)); // ip1 exhausted
        assert!(limiter.check(ip2)); // ip2 still has quota
    }

    #[test]
    fn gc_removes_stale_entries() {
        let limiter = RateLimiter::new(RateLimitConfig::new(100, 1)); // 1-second window
        let ip: IpAddr = "10.0.0.1".parse().unwrap();

        limiter.check(ip);
        assert_eq!(limiter.tracked_ips(), 1);

        // Force GC by setting last_gc to the past
        *limiter.last_gc.lock().unwrap() =
            Instant::now() - Duration::from_secs(GC_INTERVAL_SECS + 1);

        // Sleep briefly to push the timestamp outside 2x window
        std::thread::sleep(Duration::from_millis(2100));

        // Next check triggers GC and should prune the stale IP
        let other: IpAddr = "10.0.0.2".parse().unwrap();
        limiter.check(other);
        // ip1 entry should have been GC'd — only ip2 remains
        assert_eq!(limiter.tracked_ips(), 1);
    }
}
