//! sealed_secrets_api unit tests.

use super::*;

#[cfg(test)]
mod cases {
    // These serial tests hold TEST_ENV_GUARD (a std Mutex) across the
    // `enforce_release_gate(...).await` on purpose: the guard must span the await
    // so no other test mutates the process env (EXPECTED_ENV / REQUIRE_PINNED_ENV)
    // while the gate under test reads it. Dropping the guard before the await
    // would reintroduce the cross-test env race these tests exist to rule out.
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::tee::TeeType;
    use crate::tee::mock::MockTeeBackend;

    const EXPECTED_ENV: &str = "SANDBOX_TEE_EXPECTED_MEASUREMENTS";

    /// With a pinned allowlist, the gate enforces server-side: a mock backend
    /// cannot produce a hardware-verified quote (no `tee-verify` here, and the
    /// dummy report has no real quote), so the verdict is never `Verified` and
    /// release MUST be refused (HTTP 403). This proves the verifier is wired to
    /// an actual trust decision, not decorative.
    #[tokio::test]
    #[serial_test::serial]
    async fn release_gate_refuses_unverified_report_when_pinned() {
        // Snapshot the pinned allowlist under the env guard, then drop the guard
        // before the async gate runs: `enforce_release_gate` takes the snapshot
        // by value, so the std mutex is never held across an `.await`.
        let expected = {
            let _g = crate::TEST_ENV_GUARD
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            unsafe {
                std::env::set_var(EXPECTED_ENV, "0xdeadbeef");
            }
            let snapshot = expected_measurements_from_env();
            unsafe {
                std::env::remove_var(EXPECTED_ENV);
            }
            snapshot
        };
        assert!(!expected.is_empty(), "allowlist snapshot must be pinned");
        let backend = MockTeeBackend::new(TeeType::Tdx);
        let resp = enforce_release_gate(&backend, "mock-deploy-1", &expected)
            .await
            .expect_err("unverified report must be refused");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    /// When the allowlist is pinned but the backend cannot bind a freshness
    /// nonce into the report data, the gate must fail closed (HTTP 403) rather
    /// than release against a quote with no replay protection.
    #[tokio::test]
    #[serial_test::serial]
    async fn release_gate_refuses_when_backend_cannot_bind_report_data() {
        use std::sync::atomic::Ordering;

        let expected = {
            let _g = crate::TEST_ENV_GUARD
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            unsafe {
                std::env::set_var(EXPECTED_ENV, "0xdeadbeef");
            }
            let snapshot = expected_measurements_from_env();
            unsafe {
                std::env::remove_var(EXPECTED_ENV);
            }
            snapshot
        };
        let backend = MockTeeBackend::new(TeeType::Tdx);
        backend.support_report_data.store(false, Ordering::Relaxed);
        let resp = enforce_release_gate(&backend, "mock-deploy-1", &expected)
            .await
            .expect_err("must refuse without replay protection");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        // The gate must not even fetch an attestation it cannot bind.
        assert_eq!(backend.attestation_count.load(Ordering::Relaxed), 0);
    }

    /// Out of the box (no allowlist, requirement left at its default), the gate
    /// FAILS CLOSED: trust-granting release is refused with HTTP 403 rather than
    /// silently proceeding unenforced.
    #[tokio::test]
    #[serial_test::serial]
    async fn release_gate_refuses_when_unpinned_by_default() {
        let _g = crate::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        unsafe {
            std::env::remove_var(EXPECTED_ENV);
            std::env::remove_var(REQUIRE_PINNED_ENV);
        }
        let expected: Vec<Vec<u8>> = Vec::new();
        let backend = MockTeeBackend::new(TeeType::Tdx);
        let resp = enforce_release_gate(&backend, "mock-deploy-1", &expected)
            .await
            .expect_err("default config must refuse unpinned release");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        // No attestation is fetched: the gate refuses before touching the backend.
        assert_eq!(
            backend
                .attestation_count
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    /// Only with the explicit opt-out does the gate defer to the client-side
    /// verification boundary — and it reports `server_enforced == false` so the
    /// caller can surface the unenforced state instead of pretending it verified.
    #[tokio::test]
    #[serial_test::serial]
    async fn release_gate_defers_to_client_when_explicitly_opted_out() {
        let _g = crate::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        unsafe {
            std::env::remove_var(EXPECTED_ENV);
            std::env::set_var(REQUIRE_PINNED_ENV, "false");
        }
        let expected: Vec<Vec<u8>> = Vec::new();
        let backend = MockTeeBackend::new(TeeType::Tdx);
        let enforced = enforce_release_gate(&backend, "mock-deploy-1", &expected)
            .await
            .expect("explicit opt-out lets release proceed");
        unsafe {
            std::env::remove_var(REQUIRE_PINNED_ENV);
        }
        assert!(
            !enforced,
            "an unpinned release must report server_enforced == false"
        );
    }

    /// The startup guard mirrors the runtime gate: routes stay mounted only when
    /// a pin exists or the operator opted out.
    #[test]
    #[serial_test::serial]
    fn release_routes_enabled_tracks_config() {
        let _g = crate::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        unsafe {
            std::env::remove_var(EXPECTED_ENV);
            std::env::remove_var(REQUIRE_PINNED_ENV);
        }
        assert!(
            !release_routes_enabled(),
            "default + no allowlist must not serve trust-granting routes"
        );
        unsafe {
            std::env::set_var(EXPECTED_ENV, "0xdeadbeef");
        }
        assert!(
            release_routes_enabled(),
            "a pinned allowlist enables routes"
        );
        unsafe {
            std::env::remove_var(EXPECTED_ENV);
            std::env::set_var(REQUIRE_PINNED_ENV, "false");
        }
        assert!(
            release_routes_enabled(),
            "explicit opt-out enables routes without a pin"
        );
        unsafe {
            std::env::remove_var(REQUIRE_PINNED_ENV);
        }
    }
}
