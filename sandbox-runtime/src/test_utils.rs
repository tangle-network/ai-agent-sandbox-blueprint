//! Shared E2E test utilities for blueprint integration tests.
//!
//! Available when the `test-utils` feature is enabled:
//!
//! ```toml
//! [dev-dependencies]
//! sandbox-runtime = { path = "../sandbox-runtime", features = ["test-utils"] }
//! ```

use anyhow::{Context, Result};
use k256::ecdsa::SigningKey;
use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;

// ─────────────────────────────────────────────────────────────────────────────
// Cryptography
// ─────────────────────────────────────────────────────────────────────────────

/// Keccak-256 hash.
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(data);
    hasher.finalize(&mut output);
    output
}

/// Derive the checksumless EVM address from a hex-encoded private key.
pub fn address_from_key(key_hex: &str) -> String {
    let key_bytes = hex::decode(key_hex).expect("invalid hex key");
    let signing_key = SigningKey::from_bytes((&key_bytes[..]).into()).expect("invalid key bytes");
    let verifying_key = signing_key.verifying_key();
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..];
    let hash = keccak256(pubkey_uncompressed);
    format!("0x{}", hex::encode(&hash[12..]))
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Build a reqwest client with a 30s default timeout.
pub fn http() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client")
}

/// Poll a URL until it returns 2xx or the deadline is exceeded.
pub async fn wait_for_url(url: &str, timeout_secs: u64) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("URL not ready within {timeout_secs}s: {url}");
        }
        if let Ok(r) = http().get(url).send().await {
            if r.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Wait for the operator API health endpoint.
pub async fn wait_for_api(api_url: &str) -> Result<()> {
    wait_for_url(&format!("{api_url}/health"), 10)
        .await
        .context("Operator API not ready")
}

/// Wait for a sidecar to become healthy (up to 90s).
pub async fn wait_for_sidecar(sidecar_url: &str) -> Result<()> {
    wait_for_url(&format!("{sidecar_url}/health"), 90)
        .await
        .context("Sidecar not healthy")
}

// ─────────────────────────────────────────────────────────────────────────────
// JSON API helpers (GET, POST, DELETE with auth)
// ─────────────────────────────────────────────────────────────────────────────

/// Authenticated GET, returning parsed JSON. Fails on non-2xx status.
pub async fn api_get(api_url: &str, path: &str, auth: &str) -> Result<Value> {
    let resp = http()
        .get(format!("{api_url}{path}"))
        .header("authorization", auth)
        .send()
        .await?;
    let status = resp.status();
    let body: Value = resp.json().await?;
    anyhow::ensure!(status.is_success(), "GET {path} returned {status}: {body}");
    Ok(body)
}

/// Unauthenticated GET, returning parsed JSON. Fails on non-2xx status.
pub async fn api_get_unauth(api_url: &str, path: &str) -> Result<Value> {
    let resp = http().get(format!("{api_url}{path}")).send().await?;
    let status = resp.status();
    let body: Value = resp.json().await?;
    anyhow::ensure!(
        status.is_success(),
        "GET {path} (no auth) returned {status}: {body}"
    );
    Ok(body)
}

/// Authenticated POST with JSON body, returning parsed JSON. Fails on non-2xx.
pub async fn api_post(api_url: &str, path: &str, auth: &str, body: Value) -> Result<Value> {
    let resp = http()
        .post(format!("{api_url}{path}"))
        .header("authorization", auth)
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let resp_body: Value = resp.json().await?;
    anyhow::ensure!(
        status.is_success(),
        "POST {path} returned {status}: {resp_body}"
    );
    Ok(resp_body)
}

/// Authenticated DELETE (no body), returning parsed JSON. Fails on non-2xx.
pub async fn api_delete(api_url: &str, path: &str, auth: &str) -> Result<Value> {
    let resp = http()
        .delete(format!("{api_url}{path}"))
        .header("authorization", auth)
        .send()
        .await?;
    let status = resp.status();
    let resp_body: Value = resp.json().await?;
    anyhow::ensure!(
        status.is_success(),
        "DELETE {path} returned {status}: {resp_body}"
    );
    Ok(resp_body)
}

/// Assert that an API call returns a specific HTTP status code.
///
/// Supports GET, POST, and DELETE methods.
pub async fn assert_api_status(
    api_url: &str,
    method: &str,
    path: &str,
    auth: &str,
    body: Value,
    expected_status: u16,
) {
    let url = format!("{api_url}{path}");
    let resp = match method {
        "GET" => http().get(&url).header("authorization", auth).send().await,
        "POST" => {
            http()
                .post(&url)
                .header("authorization", auth)
                .json(&body)
                .send()
                .await
        }
        "DELETE" => {
            http()
                .delete(&url)
                .header("authorization", auth)
                .json(&body)
                .send()
                .await
        }
        _ => panic!("unsupported method: {method}"),
    };
    let resp = resp.unwrap_or_else(|e| panic!("{method} {path} failed: {e}"));
    assert_eq!(
        resp.status().as_u16(),
        expected_status,
        "{method} {path} expected {expected_status}, got {}",
        resp.status()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// EIP-191 Authentication
// ─────────────────────────────────────────────────────────────────────────────

/// Full EIP-191 auth flow: challenge → sign → exchange for PASETO session token.
///
/// Returns `(token, address)`.
pub async fn get_auth_token(api_url: &str, key_hex: &str) -> Result<(String, String)> {
    let key_bytes = hex::decode(key_hex).context("invalid key hex")?;
    let signing_key =
        SigningKey::from_bytes((&key_bytes[..]).into()).context("invalid signing key")?;

    // 1. Request challenge
    let challenge: Value = http()
        .post(format!("{api_url}/api/auth/challenge"))
        .send()
        .await
        .context("challenge request failed")?
        .error_for_status()
        .context("challenge returned error status")?
        .json()
        .await?;

    let nonce = challenge["nonce"]
        .as_str()
        .context("missing nonce in challenge")?;
    let message = challenge["message"]
        .as_str()
        .context("missing message in challenge")?;

    // 2. EIP-191 personal sign
    let prefixed = format!("\x19Ethereum Signed Message:\n{}{}", message.len(), message);
    let digest = keccak256(prefixed.as_bytes());
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .expect("signing failed");
    let mut sig_bytes = Vec::with_capacity(65);
    sig_bytes.extend_from_slice(&signature.to_bytes());
    sig_bytes.push(recovery_id.to_byte() + 27);
    let sig_hex = format!("0x{}", hex::encode(&sig_bytes));

    // 3. Exchange for PASETO session token
    let session: Value = http()
        .post(format!("{api_url}/api/auth/session"))
        .header("content-type", "application/json")
        .json(&json!({ "nonce": nonce, "signature": sig_hex }))
        .send()
        .await
        .context("session request failed")?
        .error_for_status()
        .context("session exchange failed")?
        .json()
        .await?;

    let token = session["token"]
        .as_str()
        .context("missing token")?
        .to_string();
    let address = session["address"]
        .as_str()
        .context("missing address")?
        .to_string();

    assert!(
        token.starts_with("v4.local."),
        "expected PASETO v4 token, got: {token}"
    );
    Ok((token, address))
}

// ─────────────────────────────────────────────────────────────────────────────
// Environment setup
// ─────────────────────────────────────────────────────────────────────────────

/// Configure environment variables for sidecar E2E tests.
///
/// Sets `SIDECAR_IMAGE`, `SIDECAR_PULL_IMAGE`, `SIDECAR_PUBLIC_HOST`,
/// `REQUEST_TIMEOUT_SECS`, `SESSION_AUTH_SECRET`, and `BLUEPRINT_STATE_DIR`.
pub fn setup_sidecar_env() {
    let image =
        std::env::var("SIDECAR_IMAGE").unwrap_or_else(|_| "tangle-sidecar:local".to_string());
    let state_dir = std::env::temp_dir().join(format!(
        "e2e-state-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir_all(&state_dir).ok();
    // SAFETY: test-only, single-threaded env setup before any concurrent work.
    unsafe {
        std::env::set_var("SIDECAR_IMAGE", &image);
        std::env::set_var("SIDECAR_PULL_IMAGE", "false");
        std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
        std::env::set_var("REQUEST_TIMEOUT_SECS", "60");
        std::env::set_var("SESSION_AUTH_SECRET", "e2e-test-secret-key");
        std::env::set_var("BLUEPRINT_STATE_DIR", &state_dir);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Operator API helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Spawn an operator API server on an ephemeral port, returning `(url, join_handle)`.
pub async fn spawn_operator_api() -> Result<(String, tokio::task::JoinHandle<()>)> {
    let app = crate::operator_api::operator_api_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("failed to bind operator API")?;
    let port = listener.local_addr()?.port();
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .ok();
    });
    let url = format!("http://127.0.0.1:{port}");
    wait_for_api(&url).await?;
    Ok((url, handle))
}

/// Read the current sidecar URL for a sandbox from the operator API.
///
/// After stop/resume, Docker assigns new ports. This re-reads the authoritative
/// URL from the operator API instead of using the stale URL from creation.
pub async fn get_sidecar_url(api_url: &str, auth: &str, sandbox_id: &str) -> Result<String> {
    let resp = http()
        .get(format!("{api_url}/api/sandboxes"))
        .header("authorization", auth)
        .send()
        .await?;
    let body: Value = resp.json().await?;
    let sandboxes = body["sandboxes"]
        .as_array()
        .context("expected sandboxes array")?;
    let sb = sandboxes
        .iter()
        .find(|s| s["id"].as_str() == Some(sandbox_id))
        .with_context(|| format!("sandbox {sandbox_id} not found in list"))?;
    sb["sidecar_url"]
        .as_str()
        .context("missing sidecar_url on sandbox record")
        .map(String::from)
}

/// Read the current sidecar URL for the singleton instance from the operator API.
pub async fn get_instance_sidecar_url(api_url: &str, auth: &str) -> Result<String> {
    let resp = http()
        .get(format!("{api_url}/api/sandboxes"))
        .header("authorization", auth)
        .send()
        .await?;
    let body: Value = resp.json().await?;
    let sandboxes = body["sandboxes"]
        .as_array()
        .context("expected sandboxes array")?;
    let sb = sandboxes.first().context("no instance sandbox in list")?;
    sb["sidecar_url"]
        .as_str()
        .context("missing sidecar_url on instance record")
        .map(String::from)
}

// ─────────────────────────────────────────────────────────────────────────────
// Well-known Anvil test accounts
// ─────────────────────────────────────────────────────────────────────────────

/// Anvil account[0] — used as the service owner / job submitter by BlueprintHarness.
pub const OWNER_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// Anvil account[1] — used as a non-owner for cross-tenant isolation tests.
pub const NON_OWNER_KEY: &str = "59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";

/// Logging macro for test steps.
#[macro_export]
macro_rules! e2e_step {
    ($n:expr, $msg:expr) => {
        eprintln!("[Step {: >2}] {}", $n, $msg);
    };
}
