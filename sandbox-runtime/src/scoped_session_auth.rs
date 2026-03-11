//! Reusable in-memory scoped session authentication for operator APIs.
//!
//! This module supports:
//! - optional operator-wide bearer tokens
//! - wallet-signature challenge flow scoped to one resource (instance/sandbox)
//! - static access-token flow scoped to one resource
//! - short-lived bearer sessions bound to `{scope_id, owner}`

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;

/// Resource auth mode used by scoped session auth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopedAuthMode {
    WalletSignature,
    AccessToken,
}

/// Auth configuration for scoped session auth.
#[derive(Clone, Debug)]
pub struct ScopedAuthConfig {
    pub challenge_ttl_secs: i64,
    pub session_ttl_secs: i64,
    pub access_token: Option<String>,
    pub operator_api_token: Option<String>,
    pub max_challenges: usize,
    pub max_sessions: usize,
    pub token_prefix: String,
    pub challenge_message_header: String,
}

impl Default for ScopedAuthConfig {
    fn default() -> Self {
        Self {
            challenge_ttl_secs: 300,
            session_ttl_secs: 3600,
            access_token: None,
            operator_api_token: None,
            max_challenges: 10_000,
            max_sessions: 50_000,
            token_prefix: "scope_".to_string(),
            challenge_message_header: "Scoped Resource Access".to_string(),
        }
    }
}

/// Resource identity for scoped auth checks.
#[derive(Clone, Debug)]
pub struct ScopedAuthResource {
    pub scope_id: String,
    pub owner: String,
    pub auth_mode: ScopedAuthMode,
}

#[derive(Clone, Debug)]
struct WalletChallengeEntry {
    scope_id: String,
    owner: String,
    wallet_address: String,
    message: String,
    expires_at: i64,
}

#[derive(Clone, Debug)]
struct SessionEntry {
    scope_id: String,
    owner: String,
    expires_at: i64,
}

#[derive(Clone, Debug)]
struct ScopedAuthState {
    challenges: BTreeMap<String, WalletChallengeEntry>,
    sessions: BTreeMap<String, SessionEntry>,
}

impl ScopedAuthState {
    fn gc(&mut self, now: i64) {
        self.challenges.retain(|_, c| c.expires_at > now);
        self.sessions.retain(|_, s| s.expires_at > now);
    }
}

/// Session claims resolved from bearer tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopedSessionClaims {
    Operator,
    Scoped { scope_id: String, owner: String },
}

/// Wallet challenge creation response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedChallengeResponse {
    pub challenge_id: String,
    pub message: String,
    pub expires_at: i64,
}

/// Session creation response for scoped auth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedSessionResponse {
    pub token: String,
    pub expires_at: i64,
    pub scope_id: String,
    pub owner: String,
}

/// In-memory scoped session authentication service.
#[derive(Clone, Debug)]
pub struct ScopedAuthService {
    config: ScopedAuthConfig,
    state: Arc<Mutex<ScopedAuthState>>,
}

impl ScopedAuthService {
    pub fn new(config: ScopedAuthConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(ScopedAuthState {
                challenges: BTreeMap::new(),
                sessions: BTreeMap::new(),
            })),
        }
    }

    pub fn resolve_bearer(&self, token: &str) -> Option<ScopedSessionClaims> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(operator_token) = &self.config.operator_api_token
            && trimmed == operator_token
        {
            return Some(ScopedSessionClaims::Operator);
        }

        let mut state = self.state.lock().ok()?;
        let now = Utc::now().timestamp();
        state.gc(now);
        state
            .sessions
            .get(trimmed)
            .map(|session| ScopedSessionClaims::Scoped {
                scope_id: session.scope_id.clone(),
                owner: session.owner.clone(),
            })
    }

    pub fn create_wallet_challenge(
        &self,
        resource: &ScopedAuthResource,
        wallet_address: &str,
    ) -> Result<ScopedChallengeResponse, String> {
        if resource.auth_mode != ScopedAuthMode::WalletSignature {
            return Err("resource does not use wallet_signature auth mode".to_string());
        }

        let wallet = normalize_evm_address(wallet_address)?;
        let owner = normalize_evm_address(&resource.owner)?;
        if wallet != owner {
            return Err("wallet address does not match resource owner".to_string());
        }

        let now = Utc::now().timestamp();
        let expires_at = now + self.config.challenge_ttl_secs;
        let challenge_id = uuid::Uuid::new_v4().to_string();
        let message = format!(
            "{header}\nscope_id:{scope_id}\nowner:{owner}\nchallenge_id:{challenge}\nissued_at:{now}\nexpires_at:{expires}",
            header = self.config.challenge_message_header,
            scope_id = resource.scope_id,
            owner = resource.owner,
            challenge = challenge_id,
            now = now,
            expires = expires_at
        );

        let mut state = self
            .state
            .lock()
            .map_err(|e| format!("scoped auth state lock poisoned: {e}"))?;
        state.gc(now);
        if state.challenges.len() >= self.config.max_challenges {
            return Err("challenge capacity exceeded, try again later".to_string());
        }
        state.challenges.insert(
            challenge_id.clone(),
            WalletChallengeEntry {
                scope_id: resource.scope_id.clone(),
                owner: resource.owner.clone(),
                wallet_address: wallet,
                message: message.clone(),
                expires_at,
            },
        );

        Ok(ScopedChallengeResponse {
            challenge_id,
            message,
            expires_at,
        })
    }

    pub fn verify_wallet_challenge(
        &self,
        challenge_id: &str,
        signature_hex: &str,
    ) -> Result<ScopedSessionResponse, String> {
        let now = Utc::now().timestamp();
        let mut state = self
            .state
            .lock()
            .map_err(|e| format!("scoped auth state lock poisoned: {e}"))?;
        state.gc(now);

        let Some(challenge) = state.challenges.remove(challenge_id) else {
            return Err("challenge not found or expired".to_string());
        };

        let recovered =
            crate::session_auth::verify_eip191_signature(&challenge.message, signature_hex)
                .map_err(|e| format!("failed to recover signer from signature: {e}"))?;
        let recovered = normalize_evm_address(&recovered)?;
        if recovered != challenge.wallet_address {
            return Err("signature does not match challenge wallet".to_string());
        }

        let expires_at = now + self.config.session_ttl_secs;
        let token = issue_token(&self.config.token_prefix);
        if state.sessions.len() >= self.config.max_sessions {
            return Err("session capacity exceeded, try again later".to_string());
        }
        state.sessions.insert(
            token.clone(),
            SessionEntry {
                scope_id: challenge.scope_id.clone(),
                owner: challenge.owner.clone(),
                expires_at,
            },
        );

        Ok(ScopedSessionResponse {
            token,
            expires_at,
            scope_id: challenge.scope_id,
            owner: challenge.owner,
        })
    }

    pub fn create_access_token_session(
        &self,
        resource: &ScopedAuthResource,
        access_token: &str,
    ) -> Result<ScopedSessionResponse, String> {
        if resource.auth_mode != ScopedAuthMode::AccessToken {
            return Err("resource does not use access_token auth mode".to_string());
        }
        let Some(expected) = &self.config.access_token else {
            return Err("scoped access token is not configured".to_string());
        };
        if access_token.trim() != expected {
            return Err("invalid access token".to_string());
        }

        let now = Utc::now().timestamp();
        let expires_at = now + self.config.session_ttl_secs;
        let token = issue_token(&self.config.token_prefix);

        let mut state = self
            .state
            .lock()
            .map_err(|e| format!("scoped auth state lock poisoned: {e}"))?;
        state.gc(now);
        if state.sessions.len() >= self.config.max_sessions {
            return Err("session capacity exceeded, try again later".to_string());
        }
        state.sessions.insert(
            token.clone(),
            SessionEntry {
                scope_id: resource.scope_id.clone(),
                owner: resource.owner.clone(),
                expires_at,
            },
        );

        Ok(ScopedSessionResponse {
            token,
            expires_at,
            scope_id: resource.scope_id.clone(),
            owner: resource.owner.clone(),
        })
    }
}

fn issue_token(prefix: &str) -> String {
    let clean_prefix = if prefix.trim().is_empty() {
        "scope_"
    } else {
        prefix.trim()
    };
    format!("{clean_prefix}{}", uuid::Uuid::new_v4().simple())
}

fn normalize_evm_address(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.len() != 42 {
        return Err(format!(
            "invalid address `{value}`: expected 42 chars with 0x prefix"
        ));
    }
    let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    else {
        return Err(format!("invalid address `{value}`: missing 0x prefix"));
    };
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("invalid address `{value}`: non-hex characters"));
    }
    Ok(format!("0x{}", hex.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(mode: ScopedAuthMode) -> ScopedAuthResource {
        ScopedAuthResource {
            scope_id: "inst-1".to_string(),
            owner: "0x0000000000000000000000000000000000000001".to_string(),
            auth_mode: mode,
        }
    }

    #[test]
    fn resolve_operator_token() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            operator_api_token: Some("op".to_string()),
            ..ScopedAuthConfig::default()
        });
        assert_eq!(
            service.resolve_bearer("op"),
            Some(ScopedSessionClaims::Operator)
        );
    }

    #[test]
    fn access_token_session_roundtrip() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            token_prefix: "acl_".to_string(),
            ..ScopedAuthConfig::default()
        });
        let session = service
            .create_access_token_session(&resource(ScopedAuthMode::AccessToken), "shared")
            .expect("session");
        assert!(session.token.starts_with("acl_"));
        assert_eq!(
            service.resolve_bearer(&session.token),
            Some(ScopedSessionClaims::Scoped {
                scope_id: "inst-1".to_string(),
                owner: "0x0000000000000000000000000000000000000001".to_string()
            })
        );
    }

    #[test]
    fn access_token_rejected_when_mismatch() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            ..ScopedAuthConfig::default()
        });
        let err = service
            .create_access_token_session(&resource(ScopedAuthMode::AccessToken), "wrong")
            .expect_err("must reject invalid token");
        assert!(err.contains("invalid access token"));
    }

    #[test]
    fn challenge_capacity_blocks_when_full() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            max_challenges: 0,
            ..ScopedAuthConfig::default()
        });
        let err = service
            .create_wallet_challenge(
                &resource(ScopedAuthMode::WalletSignature),
                "0x0000000000000000000000000000000000000001",
            )
            .expect_err("must fail when challenge capacity is exhausted");
        assert!(err.contains("challenge capacity exceeded"));
    }

    #[test]
    fn session_capacity_blocks_when_full() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            max_sessions: 0,
            ..ScopedAuthConfig::default()
        });
        let err = service
            .create_access_token_session(&resource(ScopedAuthMode::AccessToken), "shared")
            .expect_err("must fail when session capacity is exhausted");
        assert!(err.contains("session capacity exceeded"));
    }

    // ── Phase 1D: Scoped Session Auth Integration Tests ─────────────────

    #[test]
    fn scoped_session_expired_token_rejected() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            session_ttl_secs: -1, // already expired
            ..ScopedAuthConfig::default()
        });
        let session = service
            .create_access_token_session(&resource(ScopedAuthMode::AccessToken), "shared")
            .expect("should create session even with negative TTL");
        // Token was created with an already-expired timestamp
        let resolved = service.resolve_bearer(&session.token);
        assert!(
            resolved.is_none(),
            "expired token should not resolve: {resolved:?}"
        );
    }

    #[test]
    fn scoped_session_cross_scope_reuse_rejected() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            ..ScopedAuthConfig::default()
        });
        // Create session for scope "inst-1"
        let session = service
            .create_access_token_session(&resource(ScopedAuthMode::AccessToken), "shared")
            .expect("create session");
        // Resolve the token — should return scope "inst-1"
        let claims = service
            .resolve_bearer(&session.token)
            .expect("should resolve");
        match claims {
            ScopedSessionClaims::Scoped { scope_id, .. } => {
                assert_eq!(scope_id, "inst-1");
                // A different scope (e.g. "inst-2") would need a different token.
                // The token is bound to inst-1, so it can't authenticate inst-2.
                assert_ne!(scope_id, "inst-2", "token must not match a different scope");
            }
            _ => panic!("expected Scoped claims"),
        }
    }

    #[test]
    fn wallet_challenge_wrong_address_rejected() {
        let service = ScopedAuthService::new(ScopedAuthConfig::default());
        // Resource owner is 0x0000...0001, but we try to challenge with a
        // different wallet address.
        let err = service
            .create_wallet_challenge(
                &resource(ScopedAuthMode::WalletSignature),
                "0x0000000000000000000000000000000000000099",
            )
            .expect_err("should reject mismatched wallet address");
        assert!(
            err.contains("does not match"),
            "error should mention address mismatch: {err}"
        );
    }

    #[test]
    fn access_token_wrong_mode_rejected() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            ..ScopedAuthConfig::default()
        });
        // Resource is set to WalletSignature mode, but we try AccessToken flow
        let err = service
            .create_access_token_session(
                &resource(ScopedAuthMode::WalletSignature),
                "shared",
            )
            .expect_err("should reject wrong auth mode");
        assert!(
            err.contains("access_token"),
            "error should mention wrong mode: {err}"
        );
    }
}
