//! EIP-191 challenge-response + PASETO v4.local session tokens.
//!
//! Lightweight alternative to full `blueprint-auth` — avoids rocksdb, tonic,
//! and protobuf deps while providing multi-tenant wallet-based auth.
//!
//! Flow:
//! 1. Client requests a challenge: `POST /api/auth/challenge`
//! 2. Client signs the challenge with their wallet (EIP-191 personal_sign)
//! 3. Client exchanges the signature for a session token: `POST /api/auth/session`
//! 4. Client includes the PASETO token in `Authorization: Bearer <token>` headers

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use rand::RngCore;
use rand::rngs::OsRng;
use zeroize::{Zeroize, Zeroizing};

use crate::error::{Result, SandboxError};

mod challenge;
mod eip191;
mod extractor;
mod session;

pub use challenge::*;
pub use eip191::*;
pub use extractor::*;
pub use session::*;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Challenge TTL in seconds (5 minutes).
pub(crate) const CHALLENGE_TTL_SECS: u64 = 300;
/// Session token TTL in seconds (1 hour).
pub(crate) const SESSION_TTL_SECS: u64 = 3600;
/// Maximum number of pending challenges to prevent memory exhaustion.
pub(crate) const MAX_CHALLENGES: usize = 10_000;
/// Maximum number of active sessions to prevent memory exhaustion.
pub(crate) const MAX_SESSIONS: usize = 50_000;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Challenge {
    pub nonce: String,
    pub message: String,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionToken {
    pub token: String,
    pub address: String,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionClaims {
    pub address: String,
    pub issued_at: u64,
    pub expires_at: u64,
}

// ---------------------------------------------------------------------------
// In-memory stores
// ---------------------------------------------------------------------------

pub(crate) static CHALLENGES: Lazy<Mutex<HashMap<String, Challenge>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub(crate) static SESSIONS: Lazy<Mutex<HashMap<String, SessionClaims>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Revocation blacklist — tokens removed from SESSIONS that must be rejected
/// even when the PASETO fallback would otherwise accept them. Entries are
/// `(token, expires_at)` tuples; GC prunes entries past their expiry since
/// expired tokens are rejected by the PASETO expiration check anyway.
pub(crate) static REVOKED: Lazy<Mutex<HashMap<String, u64>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
