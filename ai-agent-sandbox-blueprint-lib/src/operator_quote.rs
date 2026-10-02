//! Operator-side RFQ quotes: the pricing surface of tnt-core's `JobsRFQ`.
//!
//! `GET /api/quote` lets a caller ask this operator "what would you charge to
//! run job `job_index` with these inputs for me?" and receive an EIP-712-signed
//! [`JobQuoteDetails`] redeemable on-chain via `submitJobFromQuote`. The
//! signature comes from the SDK's [`JobQuoteSigner`] (blueprint-sdk
//! `tangle-extra`), so on-chain verification is guaranteed to match — this
//! module owns pricing policy, never cryptography.
//!
//! Pricing is a composable policy, not a constant:
//!
//! ```text
//! price = base × tee_multiplier × utilization_multiplier
//! ```
//!
//! - `base` — the operator's floor price for the job (`SANDBOX_QUOTE_BASE_PRICE_WEI`).
//! - `tee_multiplier` — premium when the request demands a TEE
//!   (`SANDBOX_QUOTE_TEE_MULTIPLIER_BPS`, default 15000 = 1.5×).
//! - `utilization_multiplier` — the liquid part: interpolated between
//!   `SANDBOX_QUOTE_IDLE_MULTIPLIER_BPS` (default 8000 = 0.8×, idle compute
//!   sells at a discount) and `SANDBOX_QUOTE_FULL_MULTIPLIER_BPS` (default
//!   15000) by the operator's live utilization. An idle operator
//!   automatically underbids a busy one.
//!
//! Quotes are bound to the exact caller (`requester`) and inputs
//! (`keccak256(inputs)`) — tnt-core rejects a mismatch on redemption — and
//! expire after `SANDBOX_QUOTE_TTL_SECS` (default 120s) so prices track load.

use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use blueprint_sdk::crypto::BytesEncoding;
use blueprint_sdk::crypto::k256::K256Ecdsa;
use blueprint_sdk::keystore::Keystore;
use blueprint_sdk::keystore::backends::Backend;
use blueprint_sdk::tangle::job_quote::{JobQuoteDetails, JobQuoteSigner, QuoteSigningDomain};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::runtime;

/// Quote-serving state: one signer (the operator key) plus the pricing policy.
pub struct QuoteService {
    signer: Mutex<JobQuoteSigner>,
    policy: QuotePolicy,
    service_id: u64,
}

/// Env-driven pricing policy. Every knob is a multiply, so defaults compose
/// sanely and an operator can pin any of them to 10000 (1.0×) to disable it.
#[derive(Debug, Clone)]
pub struct QuotePolicy {
    /// Floor price for a sandbox job, in the chain's native unit's smallest
    /// denomination (wei). Default: 0.001 native.
    pub base_price_wei: U256,
    /// Multiplier in basis points applied to TEE-demanding requests.
    pub tee_multiplier_bps: u64,
    /// Multiplier at 0% utilization — idle compute sells below floor.
    pub idle_multiplier_bps: u64,
    /// Multiplier at 100% utilization.
    pub full_multiplier_bps: u64,
    /// Seconds until a quote expires. Short by default: prices track load.
    pub ttl_secs: u64,
}

impl Default for QuotePolicy {
    fn default() -> Self {
        Self {
            base_price_wei: U256::from(1_000_000_000_000_000u64), // 0.001 native
            tee_multiplier_bps: 15_000,
            idle_multiplier_bps: 8_000,
            full_multiplier_bps: 15_000,
            ttl_secs: 120,
        }
    }
}

impl QuotePolicy {
    /// Load the policy from `SANDBOX_QUOTE_*` env vars, falling back to
    /// defaults per knob.
    pub fn from_env() -> Self {
        fn var_u64(name: &str) -> Option<u64> {
            std::env::var(name).ok().and_then(|v| v.trim().parse().ok())
        }
        let d = Self::default();
        Self {
            base_price_wei: var_u64("SANDBOX_QUOTE_BASE_PRICE_WEI")
                .map(U256::from)
                .unwrap_or(d.base_price_wei),
            tee_multiplier_bps: var_u64("SANDBOX_QUOTE_TEE_MULTIPLIER_BPS")
                .unwrap_or(d.tee_multiplier_bps),
            idle_multiplier_bps: var_u64("SANDBOX_QUOTE_IDLE_MULTIPLIER_BPS")
                .unwrap_or(d.idle_multiplier_bps),
            full_multiplier_bps: var_u64("SANDBOX_QUOTE_FULL_MULTIPLIER_BPS")
                .unwrap_or(d.full_multiplier_bps),
            ttl_secs: var_u64("SANDBOX_QUOTE_TTL_SECS").unwrap_or(d.ttl_secs),
        }
    }

    /// utilization ∈ [0,1] → multiplier in bps, linearly interpolated between
    /// the idle and full multipliers.
    pub fn utilization_multiplier_bps(&self, utilization: f64) -> u64 {
        let u = utilization.clamp(0.0, 1.0);
        let lo = self.idle_multiplier_bps as f64;
        let hi = self.full_multiplier_bps as f64;
        (lo + (hi - lo) * u).round() as u64
    }

    /// The quoted price for one job under this policy.
    pub fn price_wei(&self, tee_demanded: bool, utilization: f64) -> U256 {
        let mut bps = self.utilization_multiplier_bps(utilization);
        if tee_demanded {
            bps = bps.saturating_mul(self.tee_multiplier_bps) / 10_000;
        }
        // Multiply in U256; bps is bounded by the product of two u64s, which
        // cannot overflow U256.
        self.base_price_wei.saturating_mul(U256::from(bps)) / U256::from(10_000u64)
    }
}

#[derive(Debug, Deserialize)]
pub struct QuoteQuery {
    /// The address that will redeem the quote on-chain. Required — tnt-core
    /// rejects `address(0)` wildcards, so a quote is useless without a buyer.
    pub requester: String,
    pub job_index: u8,
    /// Either the raw ABI-encoded job inputs (hashed server-side) or a
    /// precomputed keccak256 hash of them.
    pub inputs: Option<String>,
    pub inputs_hash: Option<String>,
    /// 0 = Any (default), 1 = TEE required, 2 = TEE preferred. TEE-preferred
    /// prices at the premium too: the caller asked for it.
    pub confidentiality: Option<u8>,
}

#[derive(Debug, Serialize)]
pub struct QuoteResponse {
    /// The EIP-712 typed-data fields, exactly as they will be verified
    /// on-chain. Field names match `JobQuoteDetails` in tnt-core.
    pub quote: serde_json::Value,
    /// 65-byte `r ‖ s ‖ v` (v = 27 + recovery id), hex-encoded — the exact
    /// bytes `submitJobFromQuote` expects.
    pub signature: String,
    pub operator: String,
    /// The operator's live capacity alongside the price, so a buyer can weigh
    /// availability without a second round trip.
    pub capacity: CapacityReport,
}

#[derive(Debug, Serialize)]
pub struct CapacityReport {
    pub active: u32,
    pub max: u32,
}

fn parse_address(s: &str) -> Result<Address, String> {
    s.trim()
        .strip_prefix("0x")
        .ok_or_else(|| "address must be 0x-prefixed".to_string())?
        .parse::<Address>()
        .map_err(|e| format!("invalid address: {e}"))
}

fn parse_hash(s: &str) -> Result<B256, String> {
    s.trim()
        .strip_prefix("0x")
        .ok_or_else(|| "hash must be 0x-prefixed".to_string())?
        .parse::<B256>()
        .map_err(|e| format!("invalid hash: {e}"))
}

impl QuoteService {
    /// Build the service from the operator's keystore and chain context.
    ///
    /// The signing key is the keystore's **sole** local ECDSA key — the same
    /// fail-closed selection the operator identity uses — so quotes are signed
    /// by exactly the key that registers on-chain. An ambiguous keystore is a
    /// configuration error, not a guess we make per request.
    pub fn new(
        keystore: &Keystore,
        domain: QuoteSigningDomain,
        policy: QuotePolicy,
        service_id: u64,
    ) -> Result<Self, Box<blueprint_sdk::Error>> {
        let public = keystore.sole_local::<K256Ecdsa>().map_err(|e| {
            Box::new(blueprint_sdk::Error::Other(format!(
                "quote key selection: {e}"
            )))
        })?;
        let secret = keystore
            .get_secret::<K256Ecdsa>(&public)
            .map_err(|e| Box::new(blueprint_sdk::Error::Other(format!("quote key fetch: {e}"))))?;
        let keypair = secret;
        let signer = JobQuoteSigner::new(keypair, domain).map_err(|e| {
            Box::new(blueprint_sdk::Error::Other(format!(
                "quote signer init: {e}"
            )))
        })?;
        Ok(Self {
            signer: Mutex::new(signer),
            policy,
            service_id,
        })
    }

    pub fn operator(&self) -> Address {
        // Address computation is immutable after construction; go through the
        // lock only because JobQuoteSigner keeps interior state for signing.
        self.signer.blocking_lock().operator()
    }
}

/// `GET /api/quote` — price and sign one job for one buyer.
pub async fn serve_quote(
    axum::Extension(svc): axum::Extension<Arc<QuoteService>>,
    Query(query): Query<QuoteQuery>,
) -> impl IntoResponse {
    let requester = match parse_address(&query.requester) {
        Ok(a) => a,
        Err(e) => return quote_error(StatusCode::BAD_REQUEST, e),
    };
    if requester == Address::ZERO {
        return quote_error(
            StatusCode::BAD_REQUEST,
            "requester must be a real address — tnt-core rejects wildcard quotes".to_string(),
        );
    }

    let inputs_hash = if let Some(raw) = &query.inputs_hash {
        match parse_hash(raw) {
            Ok(h) => h,
            Err(e) => return quote_error(StatusCode::BAD_REQUEST, e),
        }
    } else if let Some(raw) = &query.inputs {
        let bytes = match hex::decode(raw.trim_start_matches("0x")) {
            Ok(b) => b,
            Err(e) => {
                return quote_error(StatusCode::BAD_REQUEST, format!("invalid inputs hex: {e}"));
            }
        };
        alloy_primitives::keccak256(bytes)
    } else {
        return quote_error(
            StatusCode::BAD_REQUEST,
            "provide either `inputs` (0x-hex ABI blob) or `inputs_hash` (0x-hex keccak256)"
                .to_string(),
        );
    };

    let confidentiality = query.confidentiality.unwrap_or(0);
    if confidentiality > 2 {
        return quote_error(
            StatusCode::BAD_REQUEST,
            "confidentiality must be 0 (any), 1 (TEE required) or 2 (TEE preferred)".to_string(),
        );
    }
    let tee_demanded = confidentiality != 0;

    // Live utilization drives the liquid part of the price.
    let (active, max) = current_capacity();
    let utilization = if max == 0 {
        1.0
    } else {
        f64::from(active) / f64::from(max)
    };
    let price = svc.policy.price_wei(tee_demanded, utilization);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let details = JobQuoteDetails {
        requester,
        service_id: svc.service_id,
        job_index: query.job_index,
        price,
        timestamp: now,
        expiry: now.saturating_add(svc.policy.ttl_secs),
        confidentiality,
        inputs_hash,
    };

    let signed = {
        let mut signer = svc.signer.lock().await;
        match signer.sign(&details) {
            Ok(s) => s,
            Err(e) => {
                return quote_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("signing failed: {e}"),
                );
            }
        }
    };

    // r ‖ s ‖ v, matching the SDK's on-chain conversion (v = 27 + recovery).
    let mut sig65 = signed.signature.to_bytes();
    sig65.push(27 + signed.recovery_id);

    let quote = serde_json::json!({
        "requester": format!("{:?}", details.requester),
        "serviceId": details.service_id,
        "jobIndex": details.job_index,
        "price": price.to_string(),
        "timestamp": details.timestamp,
        "expiry": details.expiry,
        "confidentiality": details.confidentiality,
        "inputsHash": format!("{:?}", details.inputs_hash),
    });

    (
        StatusCode::OK,
        Json(QuoteResponse {
            quote,
            signature: format!("0x{}", hex::encode(sig65)),
            operator: format!("{:?}", signed.operator),
            capacity: CapacityReport { active, max },
        }),
    )
        .into_response()
}

/// `GET /api/capabilities` — what this operator offers, for discovery indexes.
#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub tee: bool,
    /// `tdx` | `nitro` | `sev-snp` | `phala-dstack`, when TEE is on.
    pub tee_type: Option<String>,
    pub gpu: bool,
    pub max_capacity: u32,
}

pub async fn serve_capabilities(
    axum::Extension(_svc): axum::Extension<Arc<QuoteService>>,
) -> impl IntoResponse {
    let (active, max) = current_capacity();
    let tee_backend = std::env::var("TEE_BACKEND").ok();
    let tee_type = tee_backend.clone();
    (
        StatusCode::OK,
        Json(Capabilities {
            tee: tee_backend.is_some(),
            tee_type,
            gpu: std::env::var("SANDBOX_OPERATOR_GPU")
                .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                .unwrap_or(false),
            max_capacity: max.max(active),
        }),
    )
}

fn current_capacity() -> (u32, u32) {
    // The blueprint contract holds the authoritative max; the runtime holds the
    // live count. Fall back to (0, u32::MAX=unknown) rather than refusing —
    // a quote with a stale capacity hint is still valid on-chain.
    let active = runtime::sandboxes()
        .and_then(|store| store.values().map(|v| v.len() as u32))
        .unwrap_or(0);
    let max = std::env::var("SANDBOX_OPERATOR_MAX_CAPACITY")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(u32::MAX);
    (active, max)
}

fn quote_error(status: StatusCode, message: String) -> axum::response::Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// The quote + capabilities router, merged into the operator API via
/// `operator_api_router_with_tee_and_routes`.
pub fn quote_router(service: Arc<QuoteService>) -> Router {
    Router::new()
        .route("/api/quote", get(serve_quote))
        .route("/api/capabilities", get(serve_capabilities))
        .layer(axum::Extension(service))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utilization_interpolates_between_idle_and_full() {
        let p = QuotePolicy {
            idle_multiplier_bps: 8_000,
            full_multiplier_bps: 15_000,
            ..QuotePolicy::default()
        };
        assert_eq!(p.utilization_multiplier_bps(0.0), 8_000);
        assert_eq!(p.utilization_multiplier_bps(1.0), 15_000);
        assert_eq!(p.utilization_multiplier_bps(0.5), 11_500);
        // Clamped, not extrapolated.
        assert_eq!(p.utilization_multiplier_bps(2.0), 15_000);
        assert_eq!(p.utilization_multiplier_bps(-1.0), 8_000);
    }

    #[test]
    fn idle_compute_underbids_busy_compute() {
        let p = QuotePolicy::default();
        let idle = p.price_wei(false, 0.0);
        let busy = p.price_wei(false, 1.0);
        assert!(
            idle < busy,
            "idle price {idle} must undercut busy price {busy}"
        );
    }

    #[test]
    fn tee_demand_applies_premium_after_utilization() {
        let p = QuotePolicy {
            tee_multiplier_bps: 15_000,
            ..QuotePolicy::default()
        };
        // 0.8 × 1.5 = 1.2× floor.
        let tee_idle = p.price_wei(true, 0.0);
        let floor = p.base_price_wei;
        assert_eq!(
            tee_idle,
            floor * U256::from(12_000u64) / U256::from(10_000u64)
        );
    }

    #[test]
    fn parse_helpers_reject_non_prefixed_and_garbage() {
        assert!(parse_address("1234").is_err());
        assert!(parse_address("0xdeadbeef").is_err());
        assert!(parse_hash("0x00").is_err());
        // A hash must be exactly 32 bytes.
        assert!(parse_hash("0xabcd").is_err());
        assert!(parse_hash(&format!("0x{}", "ab".repeat(32))).is_ok());
    }
}
