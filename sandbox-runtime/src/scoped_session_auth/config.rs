use super::*;

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

/// Session claims resolved from bearer tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopedSessionClaims {
    Operator,
    Scoped { scope_id: Arc<str>, owner: Arc<str> },
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
