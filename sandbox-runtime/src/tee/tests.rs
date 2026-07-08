//! tee/mod.rs unit tests.

use super::*;
use std::sync::atomic::Ordering;

#[test]
fn tee_type_serialization_roundtrip() {
    for variant in [TeeType::None, TeeType::Tdx, TeeType::Nitro, TeeType::Sev] {
        let json = serde_json::to_string(&variant).unwrap();
        let decoded: TeeType = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, variant);
    }
}

#[test]
fn attestation_report_serialization() {
    let report = AttestationReport {
        tee_type: TeeType::Tdx,
        evidence: vec![0xDE, 0xAD, 0xBE, 0xEF],
        measurement: vec![0x01, 0x02, 0x03],
        timestamp: 1_700_000_000,
    };
    let json = serde_json::to_string(&report).unwrap();
    let decoded: AttestationReport = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.tee_type, TeeType::Tdx);
    assert_eq!(decoded.evidence, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(decoded.measurement, vec![0x01, 0x02, 0x03]);
    assert_eq!(decoded.timestamp, 1_700_000_000);
}

#[test]
fn sidecar_attestation_response_accepts_raw_report() {
    let body = serde_json::to_string(&AttestationReport {
        tee_type: TeeType::Nitro,
        evidence: vec![1, 2, 3],
        measurement: vec![4, 5, 6],
        timestamp: 1_700_000_000,
    })
    .unwrap();

    let decoded = parse_sidecar_attestation_response(&body).unwrap();

    assert_eq!(decoded.tee_type, TeeType::Nitro);
    assert_eq!(decoded.evidence, vec![1, 2, 3]);
    assert_eq!(decoded.measurement, vec![4, 5, 6]);
}

#[test]
fn sidecar_attestation_response_accepts_wrapped_report() {
    let body = serde_json::json!({
        "sandbox_id": "sb-1",
        "attestation": {
            "tee_type": "Sev",
            "evidence": [7, 8, 9],
            "measurement": [10, 11, 12],
            "timestamp": 1_700_000_000u64,
        }
    })
    .to_string();

    let decoded = parse_sidecar_attestation_response(&body).unwrap();

    assert_eq!(decoded.tee_type, TeeType::Sev);
    assert_eq!(decoded.evidence, vec![7, 8, 9]);
    assert_eq!(decoded.measurement, vec![10, 11, 12]);
}

#[test]
fn tee_deploy_params_from_sandbox_params() {
    let params = crate::runtime::CreateSandboxParams {
        name: "test".into(),
        image: "my-image:latest".into(),
        env_json: r#"{"API_KEY":"secret","COUNT":42,"VERBOSE":true}"#.into(),
        ssh_enabled: true,
        cpu_cores: 4,
        memory_mb: 8192,
        disk_gb: 100,
        ..Default::default()
    };

    let deploy = TeeDeployParams::from_sandbox_params("sb-1", &params, 8080, 2222, "tok-abc");

    assert_eq!(deploy.sandbox_id, "sb-1");
    assert_eq!(deploy.image, "my-image:latest");
    assert_eq!(deploy.http_port, 8080);
    assert_eq!(deploy.ssh_port, Some(2222));
    assert_eq!(deploy.sidecar_token, "tok-abc");
    assert_eq!(deploy.cpu_cores, 4);
    assert_eq!(deploy.memory_mb, 8192);
    assert_eq!(deploy.disk_gb, 100);

    // Check env vars: SIDECAR_PORT + SIDECAR_AUTH_TOKEN + 3 from env_json
    assert_eq!(deploy.env_vars.len(), 5);
    assert!(
        deploy
            .env_vars
            .contains(&("SIDECAR_PORT".into(), "8080".into()))
    );
    assert!(
        deploy
            .env_vars
            .contains(&("SIDECAR_AUTH_TOKEN".into(), "tok-abc".into()))
    );
    assert!(
        deploy
            .env_vars
            .contains(&("API_KEY".into(), "secret".into()))
    );
    assert!(deploy.env_vars.contains(&("COUNT".into(), "42".into())));
    assert!(deploy.env_vars.contains(&("VERBOSE".into(), "true".into())));
}

#[test]
fn tee_deploy_params_ssh_disabled() {
    let params = crate::runtime::CreateSandboxParams {
        ssh_enabled: false,
        ..Default::default()
    };
    let deploy = TeeDeployParams::from_sandbox_params("sb-2", &params, 8080, 2222, "tok");
    assert_eq!(deploy.ssh_port, None);
}

#[test]
fn tee_deploy_params_forwards_computer_use_capability() {
    // Regression: a TEE-routed sandbox booted with capabilities=[
    // "computer_use"] must hand SIDECAR_CAPABILITIES to the
    // deploy params so the in-TEE sidecar boots Xvfb / dbus / MCP.
    // Without this, the capability silently drops on the TEE
    // path and a getMcpAccessToken call later 404s at /mcp.
    let params = crate::runtime::CreateSandboxParams {
        capabilities_json: r#"["computer_use"]"#.into(),
        ..Default::default()
    };
    let deploy = TeeDeployParams::from_sandbox_params("sb-cu", &params, 8080, 22, "t");
    assert!(
        deploy
            .env_vars
            .contains(&("SIDECAR_CAPABILITIES".into(), "computer_use".into())),
        "expected SIDECAR_CAPABILITIES in TEE env vars, got {:?}",
        deploy.env_vars
    );
}

#[test]
fn tee_deploy_params_omits_capabilities_when_unset() {
    let params = crate::runtime::CreateSandboxParams::default();
    let deploy = TeeDeployParams::from_sandbox_params("sb-empty", &params, 8080, 22, "t");
    assert!(
        !deploy
            .env_vars
            .iter()
            .any(|(k, _)| k == "SIDECAR_CAPABILITIES"),
        "expected no SIDECAR_CAPABILITIES env var when capabilities_json is empty",
    );
}

#[test]
fn tee_deploy_params_skips_nested_objects() {
    let params = crate::runtime::CreateSandboxParams {
        env_json: r#"{"SIMPLE":"val","NESTED":{"a":1},"ARR":[1,2]}"#.into(),
        ..Default::default()
    };
    let deploy = TeeDeployParams::from_sandbox_params("sb-3", &params, 8080, 22, "t");
    // Only SIDECAR_PORT + SIDECAR_AUTH_TOKEN + SIMPLE (nested/array skipped)
    assert_eq!(deploy.env_vars.len(), 3);
    assert!(deploy.env_vars.contains(&("SIMPLE".into(), "val".into())));
}

#[tokio::test]
async fn mock_backend_deploy_and_lifecycle() {
    let mock = mock::MockTeeBackend::new(TeeType::Tdx);

    let params = TeeDeployParams {
        sandbox_id: "sb-test".into(),
        image: "test:latest".into(),
        env_vars: vec![],
        cpu_cores: 2,
        memory_mb: 4096,
        disk_gb: 50,
        http_port: 8080,
        ssh_port: Some(2222),
        sidecar_token: "tok".into(),
        extra_ports: vec![],
        attestation_report_data: None,
    };

    // Deploy
    let deployment = mock.deploy(&params).await.unwrap();
    assert_eq!(deployment.deployment_id, "mock-deploy-sb-test");
    assert_eq!(deployment.sidecar_url, "http://mock-tee:8080");
    assert_eq!(deployment.ssh_port, Some(2222));
    assert_eq!(deployment.attestation.tee_type, TeeType::Tdx);
    assert_eq!(mock.deploy_count.load(Ordering::Relaxed), 1);

    // Attestation
    let att = mock.attestation("mock-deploy-sb-test", None).await.unwrap();
    assert_eq!(att.tee_type, TeeType::Tdx);
    assert_eq!(mock.attestation_count.load(Ordering::Relaxed), 1);

    // Stop
    mock.stop("mock-deploy-sb-test").await.unwrap();
    assert_eq!(mock.stop_count.load(Ordering::Relaxed), 1);

    // Destroy
    mock.destroy("mock-deploy-sb-test").await.unwrap();
    assert_eq!(mock.destroy_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn mock_backend_failing_mode() {
    let mock = mock::MockTeeBackend::failing(TeeType::Nitro);

    let params = TeeDeployParams {
        sandbox_id: "sb-fail".into(),
        image: "test:latest".into(),
        env_vars: vec![],
        cpu_cores: 1,
        memory_mb: 1024,
        disk_gb: 10,
        http_port: 8080,
        ssh_port: None,
        sidecar_token: "tok".into(),
        extra_ports: vec![],
        attestation_report_data: None,
    };

    assert!(mock.deploy(&params).await.is_err());
    assert!(mock.attestation("x", None).await.is_err());
    assert!(mock.stop("x").await.is_err());
    assert!(mock.destroy("x").await.is_err());
}

#[tokio::test]
async fn mock_backend_sealed_secrets_supported() {
    let mock = mock::MockTeeBackend::new(TeeType::Tdx);

    let pk = mock.derive_public_key("dep-1").await.unwrap();
    assert_eq!(pk.algorithm, "x25519-hkdf-sha256");
    assert_eq!(mock.derive_pk_count.load(Ordering::Relaxed), 1);

    let sealed = sealed_secrets::SealedSecret {
        algorithm: "x25519-xsalsa20-poly1305".into(),
        ciphertext: vec![0xAA],
        nonce: vec![0xBB],
    };
    let result = mock.inject_sealed_secrets("dep-1", &sealed).await.unwrap();
    assert!(result.success);
    assert_eq!(result.secrets_count, 3);
    assert_eq!(mock.inject_secrets_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn mock_backend_sealed_secrets_unsupported() {
    let mock = mock::MockTeeBackend::new(TeeType::Tdx);
    mock.support_sealed_secrets.store(false, Ordering::Relaxed);

    assert!(mock.derive_public_key("dep-1").await.is_err());
    assert!(
        mock.inject_sealed_secrets(
            "dep-1",
            &sealed_secrets::SealedSecret {
                algorithm: "test".into(),
                ciphertext: vec![],
                nonce: vec![],
            }
        )
        .await
        .is_err()
    );
}

#[test]
fn validate_attestation_report_success() {
    let report = AttestationReport {
        tee_type: TeeType::Tdx,
        evidence: vec![0x01],
        measurement: vec![0x02],
        timestamp: 1_000,
    };
    assert!(validate_attestation_report(&report, &TeeType::Tdx).is_ok());
}

#[test]
fn validate_attestation_report_empty_evidence() {
    let report = AttestationReport {
        tee_type: TeeType::Tdx,
        evidence: vec![],
        measurement: vec![0x02],
        timestamp: 1_000,
    };
    let err = validate_attestation_report(&report, &TeeType::Tdx)
        .unwrap_err()
        .to_string();
    assert!(err.contains("evidence is empty"), "{err}");
}

#[test]
fn validate_attestation_report_type_mismatch() {
    let report = AttestationReport {
        tee_type: TeeType::Sev,
        evidence: vec![0x01],
        measurement: vec![0x02],
        timestamp: 1_000,
    };
    let err = validate_attestation_report(&report, &TeeType::Tdx)
        .unwrap_err()
        .to_string();
    assert!(err.contains("mismatch"), "{err}");
}

#[test]
fn validate_attestation_report_empty_measurement() {
    let report = AttestationReport {
        tee_type: TeeType::Tdx,
        evidence: vec![0x01],
        measurement: vec![],
        timestamp: 1_000,
    };
    let err = validate_attestation_report(&report, &TeeType::Tdx)
        .unwrap_err()
        .to_string();
    assert!(err.contains("measurement is empty"), "{err}");
}

fn sample_report() -> AttestationReport {
    AttestationReport {
        tee_type: TeeType::Tdx,
        evidence: vec![0x01],
        measurement: vec![0xAA, 0xBB],
        timestamp: 1_000,
    }
}

#[test]
fn verify_attestation_is_never_trusted_without_signature_verification() {
    // The P0 guard: a malicious operator can forge a non-empty, well-formed
    // report and pin its forged measurement, but with no verifiable quote
    // signature the verdict MUST stay Unverified. The measurement match is
    // now meaningless without a verified signature (it binds to the
    // hardware-signed measurement, which we don't have), so it reports
    // `false` rather than blessing the operator's forged bytes.
    let report = sample_report();
    let pinned = vec![report.measurement.clone()];
    let v = verify_attestation(&report, &TeeType::Tdx, &pinned, None);
    assert!(!v.signature_verified);
    assert!(v.structural_ok);
    assert!(
        !v.measurement_matched,
        "measurement match must not be claimed without a verified signature"
    );
    assert!(!v.is_trusted());
    assert!(matches!(v.verdict, AttestationVerdict::Unverified { .. }));
}

#[test]
fn verify_attestation_measurement_match_requires_verified_signature() {
    // Without a verified quote signature there is no trustworthy measurement
    // to compare against, so `measurement_matched` is always false — even
    // when the operator-supplied measurement equals the pinned value.
    let report = sample_report();
    assert!(
        !verify_attestation(&report, &TeeType::Tdx, &[vec![0xAA, 0xBB]], None).measurement_matched
    );
    assert!(!verify_attestation(&report, &TeeType::Tdx, &[vec![0x00]], None).measurement_matched);
    assert!(!verify_attestation(&report, &TeeType::Tdx, &[], None).measurement_matched);
}

#[test]
fn verify_attestation_structural_failure_is_unverified() {
    let bad = AttestationReport {
        tee_type: TeeType::Tdx,
        evidence: vec![],
        measurement: vec![0xAA],
        timestamp: 1,
    };
    let v = verify_attestation(&bad, &TeeType::Tdx, &[vec![0xAA]], None);
    assert!(!v.structural_ok);
    assert!(!v.is_trusted());
    assert!(matches!(v.verdict, AttestationVerdict::Unverified { .. }));
}

#[test]
fn expected_measurements_parses_hex_list() {
    unsafe {
        std::env::set_var("SANDBOX_TEE_EXPECTED_MEASUREMENTS", "0xaabb, ccdd");
    }
    let m = expected_measurements_from_env();
    unsafe {
        std::env::remove_var("SANDBOX_TEE_EXPECTED_MEASUREMENTS");
    }
    assert_eq!(m, vec![vec![0xAA, 0xBB], vec![0xCC, 0xDD]]);
}

// ── End-to-end positive/negative against real hardware quotes ─────────────
//
// These exercise the full public `verify_attestation` path with genuine,
// vendored, known-good quotes (provenance: tests/tee_vectors/README.md).
// Gated by `tee-verify` because they pull the real verification crates.
#[cfg(feature = "tee-verify")]
mod e2e {
    use super::super::*;

    const TDX_QUOTE: &[u8] = include_bytes!("../../tests/tee_vectors/tdx_quote.bin");
    const TDX_COLLATERAL_JSON: &[u8] =
        include_bytes!("../../tests/tee_vectors/tdx_quote_collateral.json");

    /// `now` inside the collateral validity window. Shared with the unit
    /// tests in `super::super::verify`.
    fn tdx_now() -> u64 {
        super::super::verify::tests_now_from_collateral(TDX_COLLATERAL_JSON)
    }

    fn tdx_evidence(quote: &[u8]) -> Vec<u8> {
        let collateral: serde_json::Value =
            serde_json::from_slice(TDX_COLLATERAL_JSON).expect("collateral");
        serde_json::to_vec(&serde_json::json!({
            "quote": hex::encode(quote),
            "collateral": collateral,
        }))
        .expect("envelope")
    }

    /// The MRTD the genuine quote signs, obtained by running the real
    /// verifier (the honest source of truth for the signed measurement).
    fn tdx_mr_td() -> Vec<u8> {
        let report = tdx_report();
        verify_quote_signature(&report, tdx_now())
            .expect("known-good quote verifies")
            .measurement
    }

    fn tdx_report() -> AttestationReport {
        AttestationReport {
            tee_type: TeeType::Tdx,
            evidence: tdx_evidence(TDX_QUOTE),
            // Operator-supplied measurement is irrelevant to trust here; the
            // decision binds to the measurement signed inside the quote.
            measurement: vec![0u8; 48],
            timestamp: tdx_now(),
        }
    }

    #[test]
    fn genuine_tdx_quote_with_pinned_measurement_is_verified() {
        // Full public path to a real `Verified` verdict: genuine quote +
        // collateral, signature chained to the Intel SGX Root CA, TCB
        // UpToDate, and the pinned MRTD equals the one signed in the quote.
        // Time is pinned inside the collateral validity window.
        let report = tdx_report();
        let pinned = vec![tdx_mr_td()];
        let v = verify_attestation_at(&report, &TeeType::Tdx, &pinned, None, tdx_now());
        assert_eq!(
            v.verdict,
            AttestationVerdict::Verified,
            "a genuine, up-to-date TDX quote with pinned MRTD must be Verified"
        );
        assert!(v.signature_verified);
        assert!(v.measurement_matched);
        assert!(v.is_trusted());
    }

    #[test]
    fn genuine_tdx_quote_with_wrong_pinned_measurement_is_measurement_mismatch() {
        // Signature verifies, but the pinned measurement is wrong -> the
        // verdict is MeasurementMismatch, never trusted.
        let report = tdx_report();
        let wrong = vec![vec![0u8; 48]];
        let v = verify_attestation_at(&report, &TeeType::Tdx, &wrong, None, tdx_now());
        assert!(v.signature_verified, "signature still verifies");
        assert!(
            !v.is_trusted(),
            "wrong pinned measurement must not be trusted"
        );
        assert_eq!(v.verdict, AttestationVerdict::MeasurementMismatch);
    }

    #[test]
    fn tampered_tdx_quote_is_unverified_end_to_end() {
        // Flip a byte in the signed body -> signature fails -> Unverified
        // through the full public entry point, even with the MRTD pinned.
        let pinned = tdx_mr_td();
        let mut quote = TDX_QUOTE.to_vec();
        let idx = quote.len() / 2;
        quote[idx] ^= 0xFF;
        let report = AttestationReport {
            tee_type: TeeType::Tdx,
            evidence: tdx_evidence(&quote),
            measurement: pinned.clone(),
            timestamp: tdx_now(),
        };
        let v = verify_attestation_at(&report, &TeeType::Tdx, &[pinned], None, tdx_now());
        assert!(!v.signature_verified);
        assert!(!v.is_trusted());
        assert!(matches!(v.verdict, AttestationVerdict::Unverified { .. }));
    }

    /// The 64-byte report data the genuine quote actually signed.
    fn tdx_signed_report_data() -> [u8; 64] {
        verify_quote_signature(&tdx_report(), tdx_now())
            .expect("known-good quote verifies")
            .report_data
    }

    #[test]
    fn genuine_tdx_quote_binds_matching_nonce() {
        // Replay binding: supplying the exact report data the hardware signed
        // keeps the verdict Verified.
        let report = tdx_report();
        let pinned = vec![tdx_mr_td()];
        let nonce = tdx_signed_report_data();
        let v = verify_attestation_at(&report, &TeeType::Tdx, &pinned, Some(&nonce), tdx_now());
        assert!(v.report_data_matched);
        assert_eq!(v.verdict, AttestationVerdict::Verified);
    }

    #[test]
    fn genuine_tdx_quote_without_nonce_is_rejected_when_stale() {
        // Defense-in-depth: an un-challenged genuine quote whose timestamp is
        // older than the max-age bound is rejected as a possible replay, even
        // though the signature and measurement are otherwise valid. (With a
        // nonce the report_data binding would carry freshness instead.)
        let mut report = tdx_report();
        // Backdate the report well beyond the staleness window relative to
        // `tdx_now()`, keeping the verification clock at `tdx_now()`.
        report.timestamp = tdx_now() - (MAX_ATTESTATION_AGE_SECS + 60);
        let pinned = vec![tdx_mr_td()];
        let v = verify_attestation_at(&report, &TeeType::Tdx, &pinned, None, tdx_now());
        assert!(v.signature_verified, "signature still verifies");
        assert!(
            !v.is_trusted(),
            "a stale un-challenged quote must not be trusted"
        );
        match v.verdict {
            AttestationVerdict::Unverified { reason } => {
                assert!(reason.contains("stale"), "{reason}");
            }
            other => panic!("expected Unverified (stale), got {other:?}"),
        }
    }

    #[test]
    fn genuine_tdx_quote_rejects_wrong_nonce_as_replay() {
        // A challenge nonce that the quote did NOT sign must fail closed,
        // even though the signature and measurement are otherwise valid.
        let report = tdx_report();
        let pinned = vec![tdx_mr_td()];
        let mut wrong_nonce = tdx_signed_report_data();
        wrong_nonce[0] ^= 0xFF;
        let v = verify_attestation_at(
            &report,
            &TeeType::Tdx,
            &pinned,
            Some(&wrong_nonce),
            tdx_now(),
        );
        assert!(v.signature_verified, "signature still verifies");
        assert!(!v.report_data_matched, "wrong nonce must not bind");
        assert!(!v.is_trusted());
        match v.verdict {
            AttestationVerdict::Unverified { reason } => {
                assert!(
                    reason.contains("nonce") || reason.contains("replay"),
                    "{reason}"
                );
            }
            other => panic!("expected Unverified (replay), got {other:?}"),
        }
    }
}
