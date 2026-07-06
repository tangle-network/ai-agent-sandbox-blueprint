use super::*;

/// In-memory scoped session authentication service.
///
/// `resolve_bearer` is O(1) (DashMap lookup) + amortized O(0) GC (time-gated).
/// Write paths (`create_*`, `verify_*`) call `gc_now` to keep capacity checks
/// accurate at insert time.
#[derive(Clone, Debug)]
pub struct ScopedAuthService {
    config: ScopedAuthConfig,
    state: Arc<ScopedAuthState>,
}

impl ScopedAuthService {
    pub fn new(config: ScopedAuthConfig) -> Self {
        Self {
            config,
            state: Arc::new(ScopedAuthState::new()),
        }
    }

    /// Resolve a bearer token to its claims. Hot path — called on every
    /// instance-mode API request.
    ///
    /// Steady-state cost is dominated by the `DashMap::get` shard lookup
    /// and the `Arc<str>` refcount bumps on the returned claims. We
    /// deliberately do **not** call `SystemTime::now()` here: that vDSO
    /// `clock_gettime` round-trip is ~150–500 ns on shared CI hardware
    /// and would by itself blow the 1.5 µs / 10k-session perf budget
    /// regression-tested in `tests/bench_regression.rs`.
    ///
    /// Instead, the wall-clock used for the expiry comparison comes from
    /// `cached_now_secs`, refreshed (a) by every write path (which has
    /// already paid for a syscall) and (b) by the sampled cold path
    /// below, which fires roughly every 256th call. Worst-case staleness
    /// is the GC interval (60 s) — orders of magnitude smaller than
    /// session TTLs.
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

        // Sampled cold path: refresh the wall-clock cache, then maybe
        // sweep. CAS in `maybe_gc` keeps the actual sweep single-threaded.
        let calls = self.state.resolve_calls.fetch_add(1, Ordering::Relaxed);
        if calls & GC_LOAD_SAMPLE_MASK == 0 {
            let now_ms_value = now_ms();
            let now_secs = (now_ms_value / 1_000) as i64;
            self.state
                .cached_now_secs
                .store(now_secs, Ordering::Relaxed);
            let last = self.state.last_gc_ms.load(Ordering::Relaxed);
            if self.state.should_gc(now_ms_value, last) {
                self.state.maybe_gc(now_ms_value, now_secs);
            }
        }

        let session = self.state.sessions.get(trimmed)?;
        let now_secs = self.state.cached_now_secs.load(Ordering::Relaxed);
        if session.expires_at <= now_secs {
            return None;
        }
        Some(ScopedSessionClaims::Scoped {
            scope_id: Arc::clone(&session.scope_id),
            owner: Arc::clone(&session.owner),
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

        let now = now_secs();
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

        // Write path: run GC synchronously so the capacity check is accurate.
        self.state.gc_now(now_ms(), now);
        if self.state.challenges.len() >= self.config.max_challenges {
            return Err("challenge capacity exceeded, try again later".to_string());
        }
        self.state.challenges.insert(
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
        let now = now_secs();
        self.state.gc_now(now_ms(), now);

        // DashMap::remove returns Option<(K, V)>.
        let Some((_, challenge)) = self.state.challenges.remove(challenge_id) else {
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
        if self.state.sessions.len() >= self.config.max_sessions {
            return Err("session capacity exceeded, try again later".to_string());
        }
        self.state.sessions.insert(
            token.clone(),
            SessionEntry {
                scope_id: Arc::<str>::from(challenge.scope_id.as_str()),
                owner: Arc::<str>::from(challenge.owner.as_str()),
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

        let now = now_secs();
        let expires_at = now + self.config.session_ttl_secs;
        let token = issue_token(&self.config.token_prefix);

        self.state.gc_now(now_ms(), now);
        if self.state.sessions.len() >= self.config.max_sessions {
            return Err("session capacity exceeded, try again later".to_string());
        }
        self.state.sessions.insert(
            token.clone(),
            SessionEntry {
                scope_id: Arc::<str>::from(resource.scope_id.as_str()),
                owner: Arc::<str>::from(resource.owner.as_str()),
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
