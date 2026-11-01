//! In-memory scoped session auth. `resolve_bearer` must not take a global lock.
//!
//! GC runs on both elapsed time and session-map load. Time alone lets a hot
//! map grow between sweeps; load alone never expires a cold aged map.

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
