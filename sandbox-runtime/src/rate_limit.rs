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
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::metrics;

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
        self.buckets.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Clear all tracked buckets (test-only). Allows tests to reset rate limiter
    /// state so that test ordering doesn't cause spurious 429 failures.
    #[cfg(test)]
    pub fn reset(&self) {
        self.buckets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

// ---------------------------------------------------------------------------
// Static limiters
// ---------------------------------------------------------------------------

static READ_LIMITER: once_cell::sync::Lazy<RateLimiter> =
    once_cell::sync::Lazy::new(|| RateLimiter::new(RateLimitConfig::new(120, 60)));

static WRITE_LIMITER: once_cell::sync::Lazy<RateLimiter> =
    once_cell::sync::Lazy::new(|| RateLimiter::new(RateLimitConfig::new(30, 60)));

static AUTH_LIMITER: once_cell::sync::Lazy<RateLimiter> =
    once_cell::sync::Lazy::new(|| RateLimiter::new(RateLimitConfig::new(10, 60)));

/// Access the read-tier (120 req/min) limiter.
pub fn read_limiter() -> &'static RateLimiter {
    &READ_LIMITER
}

/// Access the write-tier (30 req/min) limiter.
pub fn write_limiter() -> &'static RateLimiter {
    &WRITE_LIMITER
}

/// Access the auth-tier (10 req/min) limiter.
pub fn auth_limiter() -> &'static RateLimiter {
    &AUTH_LIMITER
}

// ---------------------------------------------------------------------------
// Axum middleware functions
// ---------------------------------------------------------------------------

/// Extract client IP from ConnectInfo or x-forwarded-for header.
///
/// Security: XFF is only trusted when ConnectInfo shows the connection came
/// from a loopback or private IP (i.e., through a reverse proxy like BPM).
/// Direct connections from public IPs use the socket address directly,
/// preventing XFF spoofing from bypassing rate limits.
fn extract_client_ip(req: &Request) -> Option<IpAddr> {
    let connect_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());

    match connect_ip {
        Some(ip) if is_trusted_proxy(ip) => {
            // Connection from loopback/private IP — trust XFF header
            req.headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next())
                .and_then(|s| s.trim().parse().ok())
                .or(Some(ip))
        }
        Some(ip) => Some(ip), // Direct connection — use socket IP, ignore XFF
        None => {
            // No ConnectInfo (e.g., test/oneshot) — XFF as last resort
            req.headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next())
                .and_then(|s| s.trim().parse().ok())
        }
    }
}

/// Returns true if the IP is a loopback or private address (trusted proxy).
fn is_trusted_proxy(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

/// Sentinel IP used for rate limiting when the client IP cannot be determined.
/// All requests with unknown IPs share this single bucket, preventing bypass.
const UNKNOWN_IP: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

/// Rate-limiting middleware for read (GET) endpoints.
/// Allows 120 requests per minute per IP.
pub async fn read_rate_limit(request: Request, next: Next) -> Response {
    let ip = extract_client_ip(&request).unwrap_or(UNKNOWN_IP);
    if !read_limiter().check(ip) {
        metrics::rate_limit_rejections().fetch_add(1, Ordering::Relaxed);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "60")],
            "Rate limit exceeded",
        )
            .into_response();
    }
    next.run(request).await
}

/// Rate-limiting middleware for write (POST/PUT/DELETE) endpoints.
/// Allows 30 requests per minute per IP.
pub async fn write_rate_limit(request: Request, next: Next) -> Response {
    let ip = extract_client_ip(&request).unwrap_or(UNKNOWN_IP);
    if !write_limiter().check(ip) {
        metrics::rate_limit_rejections().fetch_add(1, Ordering::Relaxed);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "60")],
            "Rate limit exceeded",
        )
            .into_response();
    }
    next.run(request).await
}

/// Rate-limiting middleware for auth endpoints.
/// Allows 10 requests per minute per IP to prevent brute-force attacks.
pub async fn auth_rate_limit(request: Request, next: Next) -> Response {
    let ip = extract_client_ip(&request).unwrap_or(UNKNOWN_IP);
    if !auth_limiter().check(ip) {
        metrics::rate_limit_rejections().fetch_add(1, Ordering::Relaxed);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "60")],
            "Rate limit exceeded",
        )
            .into_response();
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

    #[test]
    fn extract_client_ip_returns_none_for_bare_request() {
        // Build a request with no ConnectInfo extension and no XFF header
        let req = Request::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, None, "should return None when no IP source is present");
    }

    #[test]
    fn extract_client_ip_from_xff_header() {
        let req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "192.168.1.42, 10.0.0.1")
            .body(axum::body::Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(
            ip,
            Some("192.168.1.42".parse().unwrap()),
            "should extract the first IP from XFF"
        );
    }

    #[test]
    fn extract_client_ip_xff_invalid_ip() {
        let req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "not-an-ip")
            .body(axum::body::Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, None, "invalid XFF should return None");
    }

    #[test]
    fn unknown_ip_bucket_rate_limits() {
        // All requests without a discernible IP share the UNKNOWN_IP bucket.
        let limiter = RateLimiter::new(RateLimitConfig::new(2, 60));

        assert!(limiter.check(UNKNOWN_IP));
        assert!(limiter.check(UNKNOWN_IP));
        assert!(
            !limiter.check(UNKNOWN_IP),
            "third request to unknown IP bucket should be rate limited"
        );
    }

    // ── Phase 3B: Rate Limit XFF Trust Tests ────────────────────────────

    #[test]
    fn xff_trusted_from_loopback() {
        let mut req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "203.0.113.50")
            .body(axum::body::Body::empty())
            .unwrap();
        // Add ConnectInfo with loopback address
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(
                "127.0.0.1".parse().unwrap(),
                12345,
            )));
        let ip = extract_client_ip(&req);
        assert_eq!(
            ip,
            Some("203.0.113.50".parse().unwrap()),
            "XFF should be trusted from loopback"
        );
    }

    #[test]
    fn xff_ignored_from_public_ip() {
        let mut req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "203.0.113.50")
            .body(axum::body::Body::empty())
            .unwrap();
        // Add ConnectInfo with a public IP
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(
                "198.51.100.1".parse().unwrap(),
                12345,
            )));
        let ip = extract_client_ip(&req);
        assert_eq!(
            ip,
            Some("198.51.100.1".parse().unwrap()),
            "XFF should be ignored from public IP — use socket IP instead"
        );
    }

    #[test]
    fn xff_trusted_from_private_ip() {
        let mut req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "203.0.113.99")
            .body(axum::body::Body::empty())
            .unwrap();
        // Add ConnectInfo with a private IP (10.0.0.1)
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(
                "10.0.0.1".parse().unwrap(),
                12345,
            )));
        let ip = extract_client_ip(&req);
        assert_eq!(
            ip,
            Some("203.0.113.99".parse().unwrap()),
            "XFF should be trusted from private IP"
        );
    }
}
