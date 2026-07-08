//! Reusable in-memory scoped session authentication for operator APIs.
//!
//! This module supports:
//! - optional operator-wide bearer tokens
//! - wallet-signature challenge flow scoped to one resource (instance/sandbox)
//! - static access-token flow scoped to one resource
//! - short-lived bearer sessions bound to `{scope_id, owner}`
//!
//! ## Data structure choice
//!
//! Uses `DashMap` (sharded concurrent hashmap) for both challenges and sessions
//! so `resolve_bearer` — called on every instance API request — can read without
//! acquiring a global mutex. GC is gated on (a) wall-clock elapsed since the
//! last sweep and (b) load factor of the sessions map; this mirrors the
//! pattern used by [`crate::rate_limit::RateLimiter`].
//!
//! ## Baseline numbers (criterion, sandbox-runtime/benches/scoped_session_bench.rs)
//!
//! Pre-evolve (unconditional `BTreeMap::retain` on every resolve_bearer call):
//! - 1 session:      116 ns
//! - 100 sessions:   252 ns
//! - 1 000 sessions: 1 386 ns
//! - 10 000:         22 847 ns  (196× degradation, per
//!   `.evolve/pursuits/2026-04-15-bench-infra.md`)
//!
//! Post-evolve (DashMap + load-factor + time-gated GC, this file):
//! - target: <1 µs at 10 000 sessions on the same hardware.
//!
//! The dual-trigger is deliberate: a purely time-based gate lets a hot map
//! grow arbitrarily large between sweeps (memory pressure under burst load);
//! a purely capacity-based gate skips sweeps entirely on cold-but-aged maps
//! where TTL expiries dominate.
//!
//! ## Module layout
//!
//! - [`config`] — public request/response types + service configuration
//! - [`state`] — internal challenge/session store and the time+load-gated GC
//! - [`service`] — the `ScopedAuthService` entrypoint and crypto/token helpers

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;

mod config;
mod service;
mod state;

pub use config::*;
pub use service::*;
pub(crate) use state::*;

#[cfg(test)]
mod tests;
