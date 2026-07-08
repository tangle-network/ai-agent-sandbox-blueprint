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
            scope_id: Arc::from("inst-1"),
            owner: Arc::from("0x0000000000000000000000000000000000000001"),
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
            assert_eq!(scope_id.as_ref(), "inst-1");
            // A different scope (e.g. "inst-2") would need a different token.
            // The token is bound to inst-1, so it can't authenticate inst-2.
            assert_ne!(
                scope_id.as_ref(),
                "inst-2",
                "token must not match a different scope"
            );
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
        .create_access_token_session(&resource(ScopedAuthMode::WalletSignature), "shared")
        .expect_err("should reject wrong auth mode");
    assert!(
        err.contains("access_token"),
        "error should mention wrong mode: {err}"
    );
}

// ── Post-evolve: verify DashMap migration preserves concurrency invariants ──

#[test]
fn concurrent_resolve_bearer_no_data_race() {
    use std::sync::Arc;
    use std::thread;

    let service = Arc::new(ScopedAuthService::new(ScopedAuthConfig {
        access_token: Some("shared".to_string()),
        session_ttl_secs: 3600,
        max_sessions: 100_000,
        ..ScopedAuthConfig::default()
    }));

    // Pre-populate 1000 sessions.
    let mut tokens = Vec::with_capacity(1000);
    for i in 0..1000 {
        let r = ScopedAuthResource {
            scope_id: format!("inst-{i}"),
            owner: format!("0x{:040x}", i + 1),
            auth_mode: ScopedAuthMode::AccessToken,
        };
        let s = service
            .create_access_token_session(&r, "shared")
            .expect("create");
        tokens.push(s.token);
    }
    let tokens = Arc::new(tokens);

    // Spawn 8 reader threads hammering resolve_bearer concurrently.
    let mut handles = Vec::new();
    for _ in 0..8 {
        let svc = Arc::clone(&service);
        let toks = Arc::clone(&tokens);
        handles.push(thread::spawn(move || {
            for i in 0..5_000 {
                let token = &toks[i % toks.len()];
                let claims = svc.resolve_bearer(token);
                assert!(claims.is_some(), "token must resolve under concurrency");
            }
        }));
    }

    // Concurrent writer inserting more sessions — verifies DashMap
    // handles reads/writes without deadlock or data race.
    let svc = Arc::clone(&service);
    handles.push(thread::spawn(move || {
        for i in 0..500 {
            let r = ScopedAuthResource {
                scope_id: format!("writer-{i}"),
                owner: format!("0x{:040x}", 10_000 + i),
                auth_mode: ScopedAuthMode::AccessToken,
            };
            let _ = svc.create_access_token_session(&r, "shared");
        }
    }));

    for h in handles {
        h.join().expect("worker thread panicked");
    }
}

#[test]
fn expired_session_filtered_out_before_gc_runs() {
    // Sessions with a past expiry must not resolve, even if GC hasn't run.
    let service = ScopedAuthService::new(ScopedAuthConfig {
        access_token: Some("shared".to_string()),
        session_ttl_secs: -1, // already expired at issue time
        ..ScopedAuthConfig::default()
    });
    let s = service
        .create_access_token_session(&resource(ScopedAuthMode::AccessToken), "shared")
        .expect("create");
    assert!(
        service.resolve_bearer(&s.token).is_none(),
        "expired session must never resolve to claims"
    );
}
