use chrono::Utc;

/// Current UTC timestamp as seconds since epoch.
pub fn now_ts() -> u64 {
    Utc::now().timestamp().max(0) as u64
}
