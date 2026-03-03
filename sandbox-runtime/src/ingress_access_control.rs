//! Reusable ingress access-control primitives for sandboxed app UIs.
//!
//! Products can issue per-instance bearer credentials and inject canonical
//! container env vars, with optional product-specific alias env keys.

use std::collections::BTreeSet;

use rand::RngCore;

/// Canonical auth mode env key injected into runtime containers.
pub const INGRESS_UI_AUTH_MODE_ENV: &str = "SANDBOX_UI_AUTH_MODE";
/// Canonical bearer token env key injected into runtime containers.
pub const INGRESS_UI_BEARER_TOKEN_ENV: &str = "SANDBOX_UI_BEARER_TOKEN";
/// Canonical bearer auth mode value.
pub const AUTH_MODE_BEARER: &str = "bearer";
/// Default token prefix for generated ingress credentials.
pub const DEFAULT_TOKEN_PREFIX: &str = "sbx_ui_";

const DEFAULT_TOKEN_BYTES: usize = 32;

/// Per-instance UI ingress bearer credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiBearerCredential {
    pub auth_scheme: String,
    pub token: String,
}

impl UiBearerCredential {
    /// Generate a random credential with default token prefix.
    pub fn generate() -> Self {
        Self::generate_with_prefix(DEFAULT_TOKEN_PREFIX)
    }

    /// Generate a random credential with a caller-supplied token prefix.
    pub fn generate_with_prefix(prefix: &str) -> Self {
        let mut bytes = [0_u8; DEFAULT_TOKEN_BYTES];
        rand::rngs::OsRng.fill_bytes(&mut bytes);

        let clean_prefix = if prefix.trim().is_empty() {
            DEFAULT_TOKEN_PREFIX
        } else {
            prefix
        };

        Self {
            auth_scheme: AUTH_MODE_BEARER.to_string(),
            token: format!("{clean_prefix}{}", hex::encode(bytes)),
        }
    }

    /// Build canonical env bindings.
    pub fn container_env_bindings(&self) -> Vec<(String, String)> {
        vec![
            (
                INGRESS_UI_AUTH_MODE_ENV.to_string(),
                self.auth_scheme.clone(),
            ),
            (INGRESS_UI_BEARER_TOKEN_ENV.to_string(), self.token.clone()),
        ]
    }

    /// Build env bindings with canonical keys plus alias token keys.
    pub fn container_env_bindings_with_aliases<'a, I>(
        &self,
        alias_token_env_keys: I,
    ) -> Vec<(String, String)>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut envs = self.container_env_bindings();
        let mut seen = BTreeSet::new();
        seen.insert(INGRESS_UI_BEARER_TOKEN_ENV.to_string());

        for key in alias_token_env_keys {
            let trimmed = key.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !seen.insert(trimmed.to_string()) {
                continue;
            }
            envs.push((trimmed.to_string(), self.token.clone()));
        }
        envs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_credential_uses_default_prefix() {
        let credential = UiBearerCredential::generate();
        assert_eq!(credential.auth_scheme, AUTH_MODE_BEARER);
        assert!(credential.token.starts_with(DEFAULT_TOKEN_PREFIX));
        assert!(credential.token.len() > DEFAULT_TOKEN_PREFIX.len());
    }

    #[test]
    fn generated_credential_uses_custom_prefix() {
        let credential = UiBearerCredential::generate_with_prefix("claw_ui_");
        assert!(credential.token.starts_with("claw_ui_"));
    }

    #[test]
    fn blank_prefix_falls_back_to_default() {
        let credential = UiBearerCredential::generate_with_prefix("  ");
        assert!(credential.token.starts_with(DEFAULT_TOKEN_PREFIX));
    }

    #[test]
    fn env_bindings_include_canonical_and_aliases() {
        let credential = UiBearerCredential {
            auth_scheme: AUTH_MODE_BEARER.to_string(),
            token: "tok".to_string(),
        };

        let envs = credential.container_env_bindings_with_aliases([
            "OPENCLAW_GATEWAY_TOKEN",
            INGRESS_UI_BEARER_TOKEN_ENV,
            "",
            "OPENCLAW_GATEWAY_TOKEN",
            "NANOCLAW_UI_BEARER_TOKEN",
        ]);

        assert!(
            envs.iter()
                .any(|(k, v)| k == INGRESS_UI_AUTH_MODE_ENV && v == AUTH_MODE_BEARER)
        );
        assert!(
            envs.iter()
                .any(|(k, v)| k == INGRESS_UI_BEARER_TOKEN_ENV && v == "tok")
        );
        assert!(
            envs.iter()
                .any(|(k, v)| k == "OPENCLAW_GATEWAY_TOKEN" && v == "tok")
        );
        assert!(
            envs.iter()
                .any(|(k, v)| k == "NANOCLAW_UI_BEARER_TOKEN" && v == "tok")
        );

        let canonical_count = envs
            .iter()
            .filter(|(k, _)| k == INGRESS_UI_BEARER_TOKEN_ENV)
            .count();
        assert_eq!(canonical_count, 1);
    }
}
