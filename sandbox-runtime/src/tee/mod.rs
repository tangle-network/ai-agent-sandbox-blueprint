//! TEE (Trusted Execution Environment) types and backend trait.
//!
//! This module defines the configuration, attestation types, and the async
//! `TeeBackend` trait used to deploy sandboxes inside trusted execution
//! environments. Backend implementations live in feature-gated submodules.

#[cfg(feature = "tee-phala")]
pub mod phala;

#[cfg(feature = "tee-direct")]
pub mod attestation;

#[cfg(feature = "tee-direct")]
pub mod direct;

#[cfg(feature = "tee-aws-nitro")]
pub mod aws_nitro;

#[cfg(feature = "tee-gcp")]
pub mod gcp;

#[cfg(feature = "tee-azure")]
pub mod azure;

pub mod backend_factory;
pub mod sealed_secrets;
pub mod sealed_secrets_api;

/// Real cryptographic quote verification (Intel TDX DCAP, AMD SEV-SNP, AWS
/// Nitro). Gated so the default and non-TEE builds never pull the heavier
/// X.509/ECDSA crates.
#[cfg(feature = "tee-verify")]
mod verify;

/// Supported TEE backend types.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TeeType {
    /// No TEE — standard Docker container (default).
    #[default]
    None,
    /// Intel TDX — VM-level isolation (Phala dstack, GCP C3, Azure DCesv5).
    Tdx,
    /// AWS Nitro Enclaves.
    Nitro,
    /// AMD SEV-SNP confidential VMs (Azure DCasv5, GCP N2D).
    Sev,
}

/// TEE configuration for sandbox creation.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct TeeConfig {
    /// Whether TEE execution is required. If true and the operator cannot
    /// provide TEE, sandbox creation fails.
    pub required: bool,
    /// Preferred TEE backend. If `None` (default), the operator chooses.
    pub tee_type: TeeType,
    /// Optional caller-supplied attestation nonce/report data.
    ///
    /// TDX and SEV-SNP reports take exactly 64 bytes of report data. Callers
    /// may supply 32-64 bytes; shorter values are right-padded with zeros.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_nonce: Option<Vec<u8>>,
}

/// Attestation report produced by a TEE runtime.
///
/// Returned to the customer so they can verify the sandbox is running
/// inside a genuine enclave with the expected code measurement.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AttestationReport {
    /// The TEE backend that produced this report.
    pub tee_type: TeeType,
    /// Raw attestation evidence (TDX report, Nitro attestation document, etc.).
    pub evidence: Vec<u8>,
    /// Enclave measurement (MRTD for TDX, PCR values for Nitro, etc.).
    pub measurement: Vec<u8>,
    /// Unix timestamp when the attestation was generated.
    pub timestamp: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// TeeBackend trait
// ─────────────────────────────────────────────────────────────────────────────

/// Parameters for deploying a container inside a TEE.
///
/// Constructed from `CreateSandboxParams` — see `TeeDeployParams::from_sandbox_params`.
#[derive(Clone, Debug)]
pub struct TeeDeployParams {
    pub sandbox_id: String,
    pub image: String,
    pub env_vars: Vec<(String, String)>,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub disk_gb: u64,
    pub http_port: u16,
    pub ssh_port: Option<u16>,
    pub sidecar_token: String,
    /// Extra container ports to expose (e.g. user web server on 3000).
    pub extra_ports: Vec<u16>,
    /// Optional caller-supplied report data for deploy-time attestation.
    pub attestation_report_data: Option<[u8; 64]>,
}

impl TeeDeployParams {
    /// Build TEE deploy params from a sandbox creation request.
    pub fn from_sandbox_params(
        sandbox_id: &str,
        params: &crate::runtime::CreateSandboxParams,
        container_port: u16,
        ssh_port: u16,
        token: &str,
    ) -> Self {
        let mut env_vars = vec![
            ("SIDECAR_PORT".to_string(), container_port.to_string()),
            ("SIDECAR_AUTH_TOKEN".to_string(), token.to_string()),
        ];

        if let Some(caps) = crate::runtime::parse_sidecar_capabilities(&params.capabilities_json) {
            env_vars.push(("SIDECAR_CAPABILITIES".to_string(), caps));
        }

        // Parse env_json into env var pairs.
        if !params.env_json.trim().is_empty()
            && let Ok(Some(serde_json::Value::Object(map))) =
                crate::util::parse_json_object(&params.env_json, "env_json")
        {
            for (key, value) in map {
                let val = match value {
                    serde_json::Value::String(v) => v,
                    serde_json::Value::Number(v) => v.to_string(),
                    serde_json::Value::Bool(v) => v.to_string(),
                    _ => continue,
                };
                env_vars.push((key, val));
            }
        }

        Self {
            sandbox_id: sandbox_id.to_string(),
            image: params.image.clone(),
            env_vars,
            cpu_cores: params.cpu_cores,
            memory_mb: params.memory_mb,
            disk_gb: params.disk_gb,
            http_port: container_port,
            ssh_port: if params.ssh_enabled {
                Some(ssh_port)
            } else {
                None
            },
            sidecar_token: token.to_string(),
            extra_ports: params.port_mappings.clone(),
            attestation_report_data: params
                .tee_config
                .as_ref()
                .and_then(|cfg| cfg.attestation_report_data()),
        }
    }
}

impl TeeConfig {
    /// Normalize caller-supplied nonce bytes into 64-byte report data.
    pub fn attestation_report_data(&self) -> Option<[u8; 64]> {
        match self.attestation_nonce.as_ref() {
            Some(nonce) => pad_attestation_nonce(nonce).ok().flatten(),
            None => None,
        }
    }

    /// Set attestation nonce bytes after validating length.
    pub fn with_attestation_nonce(mut self, nonce: Option<Vec<u8>>) -> crate::error::Result<Self> {
        if let Some(ref value) = nonce {
            validate_attestation_nonce(value)?;
        }
        self.attestation_nonce = nonce;
        Ok(self)
    }
}

/// Decode a hex-encoded caller nonce. Accepts optional `0x` prefix.
pub fn decode_attestation_nonce_hex(value: &str) -> crate::error::Result<Vec<u8>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let hex = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    if !hex.len().is_multiple_of(2) {
        return Err(crate::error::SandboxError::Validation(
            "attestation_nonce must be even-length hex".into(),
        ));
    }
    let bytes = hex::decode(hex).map_err(|e| {
        crate::error::SandboxError::Validation(format!("attestation_nonce must be hex: {e}"))
    })?;
    validate_attestation_nonce(&bytes)?;
    Ok(bytes)
}

/// Validate caller nonce size. Empty means "not supplied".
pub fn validate_attestation_nonce(nonce: &[u8]) -> crate::error::Result<()> {
    if nonce.is_empty() {
        return Ok(());
    }
    if !(32..=64).contains(&nonce.len()) {
        return Err(crate::error::SandboxError::Validation(format!(
            "attestation_nonce must be 32-64 bytes, got {}",
            nonce.len()
        )));
    }
    Ok(())
}

/// Convert caller nonce bytes into fixed-size TEE report data.
pub fn pad_attestation_nonce(nonce: &[u8]) -> crate::error::Result<Option<[u8; 64]>> {
    validate_attestation_nonce(nonce)?;
    if nonce.is_empty() {
        return Ok(None);
    }
    let mut report_data = [0u8; 64];
    report_data[..nonce.len()].copy_from_slice(nonce);
    Ok(Some(report_data))
}

/// Result of a successful TEE deployment.
#[derive(Clone, Debug)]
pub struct TeeDeployment {
    /// Backend-specific deployment ID (e.g. Phala app_id).
    pub deployment_id: String,
    /// Reachable URL for the sidecar HTTP API.
    pub sidecar_url: String,
    /// Host-mapped SSH port, if SSH was requested.
    pub ssh_port: Option<u16>,
    /// Attestation report from the TEE.
    pub attestation: AttestationReport,
    /// Opaque backend state, stored in SandboxRecord for later lifecycle ops.
    pub metadata_json: String,
    /// Extra port mappings: container_port → host_port.
    pub extra_ports: std::collections::HashMap<u16, u16>,
}

/// Async trait for TEE backend implementations.
///
/// Each backend (Phala dstack, operator-managed TDX/SEV hardware, cloud TEE, etc.)
/// implements this trait to handle the full lifecycle of a TEE deployment.
#[async_trait::async_trait]
pub trait TeeBackend: Send + Sync {
    /// Deploy a container inside a TEE.
    async fn deploy(&self, params: &TeeDeployParams) -> crate::error::Result<TeeDeployment>;

    /// Retrieve fresh attestation for a running deployment.
    async fn attestation(
        &self,
        deployment_id: &str,
        report_data: Option<[u8; 64]>,
    ) -> crate::error::Result<AttestationReport>;

    /// Stop a TEE deployment (may be resumable depending on backend).
    async fn stop(&self, deployment_id: &str) -> crate::error::Result<()>;

    /// Destroy a TEE deployment and clean up all resources.
    async fn destroy(&self, deployment_id: &str) -> crate::error::Result<()>;

    /// Which TEE type this backend provides.
    fn tee_type(&self) -> TeeType;

    /// Whether this backend can embed caller-supplied report data in fresh
    /// attestations. Freshness challenges must fail closed when unsupported.
    fn supports_attestation_report_data(&self) -> bool {
        false
    }

    // ── Sealed secrets (optional, default: not supported) ────────────────

    /// Derive a TEE-bound public key for sealed secret encryption.
    ///
    /// The returned key is bound to the enclave measurement via attestation.
    /// Clients verify the attestation before encrypting secrets to this key.
    ///
    /// Default: returns an error indicating sealed secrets are not supported.
    async fn derive_public_key(
        &self,
        deployment_id: &str,
    ) -> crate::error::Result<sealed_secrets::TeePublicKey> {
        let _ = deployment_id;
        Err(crate::error::SandboxError::Validation(format!(
            "Sealed secrets not supported by {:?} backend",
            self.tee_type()
        )))
    }

    /// Inject sealed (encrypted) secrets into a TEE deployment.
    ///
    /// The operator calls this to forward the client's encrypted blob to the
    /// sidecar running inside the TEE. Only the TEE can decrypt.
    ///
    /// Default: returns an error indicating sealed secrets are not supported.
    async fn inject_sealed_secrets(
        &self,
        deployment_id: &str,
        sealed: &sealed_secrets::SealedSecret,
    ) -> crate::error::Result<sealed_secrets::SealedSecretResult> {
        let _ = (deployment_id, sealed);
        Err(crate::error::SandboxError::Validation(format!(
            "Sealed secrets not supported by {:?} backend",
            self.tee_type()
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Global TEE backend singleton
// ─────────────────────────────────────────────────────────────────────────────

static TEE_BACKEND: once_cell::sync::OnceCell<std::sync::Arc<dyn TeeBackend>> =
    once_cell::sync::OnceCell::new();

/// Initialize the global TEE backend. Call once at startup.
pub fn init_tee_backend(backend: std::sync::Arc<dyn TeeBackend>) {
    if TEE_BACKEND.set(backend).is_err() {
        tracing::warn!("TEE backend already initialized, ignoring duplicate init");
    }
}

/// Get the global TEE backend.
///
/// Returns an error if the backend has not been initialized via
/// [`init_tee_backend`]. Prefer [`try_tee_backend`] when absence is
/// expected (e.g. non-TEE operators).
pub fn tee_backend() -> crate::error::Result<&'static std::sync::Arc<dyn TeeBackend>> {
    TEE_BACKEND.get().ok_or_else(|| {
        crate::error::SandboxError::Validation(
            "TEE backend not initialized — call init_tee_backend() first".into(),
        )
    })
}

/// Try to get the global TEE backend, returning `None` if not initialized.
///
/// Use this in shared code paths (e.g. instance operator API) that need to
/// auto-detect whether TEE is available without panicking.
pub fn try_tee_backend() -> Option<&'static std::sync::Arc<dyn TeeBackend>> {
    TEE_BACKEND.get()
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers for cloud TEE backends
// ─────────────────────────────────────────────────────────────────────────────

/// Look up the sidecar URL and auth token for a TEE deployment by its deployment ID.
///
/// Scans the sandbox store for a record whose `tee_deployment_id` matches.
#[allow(dead_code)] // Used by TEE backends (phala, gcp, azure)
pub(crate) fn sidecar_info_for_deployment(
    deployment_id: &str,
) -> crate::error::Result<(String, String)> {
    let store = crate::runtime::sandboxes()?;
    let record = store
        .find(|r| r.tee_deployment_id.as_deref() == Some(deployment_id))?
        .ok_or_else(|| {
            crate::error::SandboxError::NotFound(format!(
                "No sandbox found for TEE deployment '{deployment_id}'"
            ))
        })?;
    Ok((record.sidecar_url.clone(), record.token.clone()))
}

/// Fetch fresh attestation from a running sidecar's `/tee/attestation` endpoint.
#[allow(dead_code)] // Used by TEE backends
pub(crate) async fn fetch_sidecar_attestation(
    sidecar_url: &str,
    token: &str,
) -> crate::error::Result<AttestationReport> {
    fetch_sidecar_attestation_with_report_data(sidecar_url, token, None).await
}

/// Fetch fresh attestation from a running sidecar, optionally bound to caller report data.
#[allow(dead_code)] // Used by TEE backends
pub(crate) async fn fetch_sidecar_attestation_with_report_data(
    sidecar_url: &str,
    token: &str,
    report_data: Option<[u8; 64]>,
) -> crate::error::Result<AttestationReport> {
    let url = crate::http::build_url(sidecar_url, "/tee/attestation")?;
    let headers = crate::http::auth_headers(token)?;
    let method = if report_data.is_some() {
        reqwest::Method::POST
    } else {
        reqwest::Method::GET
    };
    let body = report_data.map(|data| {
        serde_json::json!({
            "attestation_nonce": hex::encode(data),
        })
    });
    let (_status, body) = crate::http::send_json(method, url, body, headers).await?;
    let report = parse_sidecar_attestation_response(&body)?;

    // Basic sanity check — callers should also validate the TEE type matches.
    if report.evidence.is_empty() {
        return Err(crate::error::SandboxError::CloudProvider(
            "Sidecar returned empty attestation evidence".into(),
        ));
    }
    if report.measurement.is_empty() {
        return Err(crate::error::SandboxError::CloudProvider(
            "Sidecar returned empty attestation measurement".into(),
        ));
    }

    Ok(report)
}

fn parse_sidecar_attestation_response(body: &str) -> crate::error::Result<AttestationReport> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|e| {
        crate::error::SandboxError::Http(format!("Invalid attestation response: {e}"))
    })?;
    let report_value = value.get("attestation").cloned().unwrap_or(value);
    serde_json::from_value(report_value)
        .map_err(|e| crate::error::SandboxError::Http(format!("Invalid attestation response: {e}")))
}

/// Validate an attestation report for completeness and type correctness.
///
/// Called after every attestation fetch (sidecar or native) to catch silent
/// failures where the sidecar returns 200 with empty/wrong data.
#[allow(dead_code)] // Used by TEE backends (attestation.rs)
pub(crate) fn validate_attestation_report(
    report: &AttestationReport,
    expected_type: &TeeType,
) -> crate::error::Result<()> {
    if report.evidence.is_empty() {
        return Err(crate::error::SandboxError::CloudProvider(
            "Attestation evidence is empty — TEE device may not be available".into(),
        ));
    }
    if &report.tee_type != expected_type {
        return Err(crate::error::SandboxError::CloudProvider(format!(
            "Attestation type mismatch: expected {expected_type:?}, got {:?}",
            report.tee_type
        )));
    }
    if report.measurement.is_empty() {
        return Err(crate::error::SandboxError::CloudProvider(
            "Attestation measurement is empty — report may be malformed".into(),
        ));
    }
    Ok(())
}

/// Outcome of cryptographically verifying an [`AttestationReport`].
///
/// IMPORTANT: this encodes the *real* verification state. Under the `tee-verify`
/// feature the quote signature chain IS verified against a hardware root (see
/// [`verify_quote_signature`] / [`verify`]), so [`AttestationVerdict::Verified`]
/// is reachable for genuine TDX/SEV-SNP quotes. Without that feature the chain
/// is never verified and `Verified` is unreachable. Either way, callers and UIs
/// MUST treat anything other than `Verified` as untrusted and must not present
/// the workload as hardware-attested.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum AttestationVerdict {
    /// Quote signature chained to a hardware root AND the measurement signed
    /// inside the quote matched a pinned expected value AND (if a freshness
    /// nonce was supplied) the signed report data carried it.
    Verified,
    /// Structurally well-formed but NOT cryptographically verified (bad/absent
    /// signature chain, expired/insufficient TCB, or replayed report data).
    Unverified { reason: String },
    /// Signature verified, but the measurement signed inside the quote matched
    /// none of the pinned expected measurements.
    MeasurementMismatch,
}

/// Detailed result of [`verify_attestation`], suitable for serialising to the
/// UI / on-chain so the *honest* trust state travels with the attestation.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttestationVerification {
    pub verdict: AttestationVerdict,
    /// Whether the quote signature was verified against a hardware root of trust
    /// (Intel PCS/PCCS for TDX, AMD KDS for SEV-SNP, NSM for Nitro).
    pub signature_verified: bool,
    /// Whether the measurement matched a pinned expected value.
    pub measurement_matched: bool,
    /// Whether the caller-supplied freshness nonce matched the report data the
    /// hardware signed. `true` when no nonce was requested (nothing to bind);
    /// `false` when a nonce was requested but the signed report data didn't
    /// carry it (replay / mismatch), which forces a non-`Verified` verdict.
    pub report_data_matched: bool,
    /// Whether the report passed structural/type checks.
    pub structural_ok: bool,
}

impl AttestationVerification {
    /// True only when the attestation is cryptographically trustworthy.
    pub fn is_trusted(&self) -> bool {
        matches!(self.verdict, AttestationVerdict::Verified)
    }
}

/// The measurement a verified quote actually carried (signed by hardware).
///
/// Returned by [`verify_quote_signature`] so the trust decision binds the
/// measurement the *hardware* signed, not the operator-supplied
/// `AttestationReport.measurement`.
#[cfg_attr(not(feature = "tee-verify"), allow(dead_code))]
struct SignedQuoteFacts {
    /// Measurement extracted from inside the cryptographically verified quote.
    measurement: Vec<u8>,
    /// 64-byte report data the hardware signed (caller nonce binding).
    report_data: [u8; 64],
}

/// Verify a TEE quote's signature against the appropriate hardware root of
/// trust.
///
/// Returns the measurement + report_data the hardware signed on success, so the
/// caller can bind them. Fails closed: any backend that cannot chain the quote
/// to a hardware root the operator does not control returns `Err(reason)`.
///
/// The genuine per-backend implementations live in [`verify`] (Intel TDX via
/// `dcap-qvl` to the Intel SGX Root CA; AMD SEV-SNP via the `sev` crate to the
/// AMD ARK). They are compiled only under the `tee-verify` feature. Without that
/// feature this is the honest fail-closed stub: nothing can be hardware-verified
/// and [`verify_attestation`] therefore never reports `Verified`.
///
/// `now_secs` is the trusted current time used for collateral/TCB/CRL freshness
/// checks; production callers pass the system clock.
#[cfg(feature = "tee-verify")]
fn verify_quote_signature(
    report: &AttestationReport,
    now_secs: u64,
) -> Result<SignedQuoteFacts, String> {
    let verified = verify::verify_quote_signature(report, now_secs)?;
    Ok(SignedQuoteFacts {
        measurement: verified.measurement,
        report_data: verified.report_data,
    })
}

/// Fail-closed stub used when `tee-verify` is disabled. Verifying a TDX/SEV-SNP
/// quote requires validating the platform certificate chain (Intel PCS/PCCS
/// DCAP collateral; AMD KDS VCEK) against a hardware root; that machinery lives
/// behind the `tee-verify` feature. Without it, no attestation can be trusted.
#[cfg(not(feature = "tee-verify"))]
fn verify_quote_signature(
    report: &AttestationReport,
    _now_secs: u64,
) -> Result<SignedQuoteFacts, String> {
    Err(format!(
        "quote-signature verification unavailable for {:?}: build with the `tee-verify` feature to chain the quote to a hardware root of trust",
        report.tee_type
    ))
}

/// Operator-independent allowlist of expected enclave measurements, read from
/// `SANDBOX_TEE_EXPECTED_MEASUREMENTS` (comma/whitespace-separated hex).
///
/// Measurement pinning only adds security when the expected value comes from a
/// source the operator does NOT control (a verifying client, or on-chain
/// config) — otherwise a malicious operator forges both the measurement and the
/// expected value. An empty allowlist means "no expected measurement
/// configured", which can never match.
pub fn expected_measurements_from_env() -> Vec<Vec<u8>> {
    std::env::var("SANDBOX_TEE_EXPECTED_MEASUREMENTS")
        .ok()
        .map(|raw| {
            raw.split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .filter_map(|s| hex::decode(s.trim().trim_start_matches("0x")).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Maximum age (seconds) accepted for an attestation that was NOT bound to a
/// freshness nonce. The nonce binding ([`verify_attestation`] with
/// `expected_report_data`) is the durable replay defense; this bound is
/// defense-in-depth for paths that legitimately cannot challenge (e.g. a
/// deploy-time report surfaced for display). 10 minutes is generous enough for
/// clock skew and slow attestation fetches while still rejecting a quote
/// captured hours/days earlier and replayed.
const MAX_ATTESTATION_AGE_SECS: u64 = 600;

/// Cryptographically verify an attestation report, returning the *honest*
/// verification state.
///
/// The verdict is [`AttestationVerdict::Verified`] only when BOTH the quote
/// signature is verified against a hardware root AND the measurement signed
/// inside the quote matches a pinned expected value. Under the `tee-verify`
/// feature this can genuinely reach `Verified`; without it, [`verify_quote_signature`]
/// always errs and the verdict can never be `Verified` — by design, so the
/// product never claims a guarantee it cannot back. This is the single entry
/// point any trust decision (UI badge, on-chain gate, sealed-secret release)
/// must consult; structural checks alone ([`validate_attestation_report`]) are
/// NOT a trust decision.
pub fn verify_attestation(
    report: &AttestationReport,
    expected_type: &TeeType,
    expected_measurements: &[Vec<u8>],
    expected_report_data: Option<&[u8; 64]>,
) -> AttestationVerification {
    verify_attestation_at(
        report,
        expected_type,
        expected_measurements,
        expected_report_data,
        crate::util::now_ts(),
    )
}

/// [`verify_attestation`] with an explicit trusted `now_secs`, so collateral/TCB
/// freshness is evaluated against a caller-controlled clock. Used by tests to
/// pin the time to a vendored quote's validity window; production uses the
/// system clock via [`verify_attestation`].
pub(crate) fn verify_attestation_at(
    report: &AttestationReport,
    expected_type: &TeeType,
    expected_measurements: &[Vec<u8>],
    expected_report_data: Option<&[u8; 64]>,
    now_secs: u64,
) -> AttestationVerification {
    let structural_ok = validate_attestation_report(report, expected_type).is_ok();
    let signature_result = verify_quote_signature(report, now_secs);
    let signature_verified = signature_result.is_ok();

    // Bind the measurement to the value the HARDWARE signed inside the quote,
    // not the operator-supplied `report.measurement`. A malicious operator can
    // forge `report.measurement` freely, but cannot forge the signed quote.
    // When the signature didn't verify we have no trustworthy measurement, so
    // the match is meaningless and we report `false`.
    let signed_measurement = signature_result.as_ref().ok().map(|f| &f.measurement);
    let measurement_matched = !expected_measurements.is_empty()
        && signed_measurement.is_some_and(|signed| {
            expected_measurements
                .iter()
                .any(|expected| expected == signed)
        });

    // Replay binding: when the caller challenged with a nonce, the nonce MUST
    // equal the report data the hardware signed. Compared in constant time. With
    // no challenge there is nothing to bind, so this is vacuously satisfied —
    // but only counts toward trust once the signature itself verified.
    let report_data_matched = match (expected_report_data, signature_result.as_ref().ok()) {
        (Some(expected), Some(facts)) => bool::from(subtle::ConstantTimeEq::ct_eq(
            &facts.report_data[..],
            &expected[..],
        )),
        (Some(_), None) => false,
        (None, _) => true,
    };

    // Staleness bound (defense-in-depth, only on the un-challenged path). With a
    // nonce, `report_data_matched` already proves freshness. Without one, reject
    // a report whose timestamp is too far in the past so a genuine-but-old quote
    // cannot be replayed. `report.timestamp` is operator-supplied and only
    // tightens (never relaxes) the decision: a future/forged timestamp does not
    // grant trust because the signature/measurement gates still apply.
    let fresh_enough = expected_report_data.is_some()
        || now_secs.saturating_sub(report.timestamp) <= MAX_ATTESTATION_AGE_SECS;

    let verdict = if !structural_ok {
        AttestationVerdict::Unverified {
            reason: "attestation report failed structural validation".to_string(),
        }
    } else if let Err(reason) = signature_result {
        AttestationVerdict::Unverified { reason }
    } else if !report_data_matched {
        AttestationVerdict::Unverified {
            reason: "freshness nonce did not match the report data signed by the hardware (possible replay)".to_string(),
        }
    } else if !fresh_enough {
        AttestationVerdict::Unverified {
            reason: format!(
                "attestation is stale: timestamp {} is more than {}s before now {} and no \
                 freshness nonce was supplied (possible replay)",
                report.timestamp, MAX_ATTESTATION_AGE_SECS, now_secs
            ),
        }
    } else if !measurement_matched {
        AttestationVerdict::MeasurementMismatch
    } else {
        AttestationVerdict::Verified
    };

    AttestationVerification {
        verdict,
        signature_verified,
        measurement_matched,
        report_data_matched,
        structural_ok,
    }
}

/// Poll a sidecar's `/health` endpoint until it responds successfully.
#[allow(dead_code)] // Used by TEE backends
pub(crate) async fn wait_for_sidecar_health(
    sidecar_url: &str,
    token: &str,
    timeout: std::time::Duration,
) -> crate::error::Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(crate::error::SandboxError::CloudProvider(
                "Sidecar health check timed out".into(),
            ));
        }
        if let (Ok(url), Ok(headers)) = (
            crate::http::build_url(sidecar_url, "/health"),
            crate::http::auth_headers(token),
        ) && crate::http::send_json(reqwest::Method::GET, url, None, headers)
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

/// Derive a TEE-bound public key by proxying to the sidecar.
#[allow(dead_code)] // Used by TEE backends
pub(crate) async fn sidecar_derive_public_key(
    deployment_id: &str,
) -> crate::error::Result<sealed_secrets::TeePublicKey> {
    let (sidecar_url, token) = sidecar_info_for_deployment(deployment_id)?;
    let url = crate::http::build_url(&sidecar_url, "/tee/public-key")?;
    let headers = crate::http::auth_headers(&token)?;
    let (_status, body) = crate::http::send_json(reqwest::Method::GET, url, None, headers).await?;
    serde_json::from_str(&body).map_err(|e| {
        crate::error::SandboxError::Http(format!("Invalid TeePublicKey response: {e}"))
    })
}

/// Inject sealed secrets by proxying to the sidecar.
#[allow(dead_code)] // Used by TEE backends
pub(crate) async fn sidecar_inject_sealed_secrets(
    deployment_id: &str,
    sealed: &sealed_secrets::SealedSecret,
) -> crate::error::Result<sealed_secrets::SealedSecretResult> {
    let (sidecar_url, token) = sidecar_info_for_deployment(deployment_id)?;
    let payload = serde_json::to_value(sealed).map_err(|e| {
        crate::error::SandboxError::Validation(format!("Failed to serialize sealed secret: {e}"))
    })?;
    let resp = crate::http::sidecar_post_json(&sidecar_url, "/tee/sealed-secrets", &token, payload)
        .await?;
    serde_json::from_value(resp).map_err(|e| {
        crate::error::SandboxError::Http(format!("Invalid SealedSecretResult response: {e}"))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Mock backend for tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(any(test, feature = "test-utils"))]
pub mod mock {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// A configurable mock TEE backend for tests.
    ///
    /// Tracks call counts via atomics. By default all operations succeed and
    /// sealed secrets are supported. Use `failing()` to create a mock that
    /// returns errors for all operations.
    pub struct MockTeeBackend {
        pub tee_type: TeeType,
        pub deploy_count: AtomicUsize,
        pub stop_count: AtomicUsize,
        pub destroy_count: AtomicUsize,
        pub attestation_count: AtomicUsize,
        pub derive_pk_count: AtomicUsize,
        pub inject_secrets_count: AtomicUsize,
        pub should_fail: AtomicBool,
        pub support_sealed_secrets: AtomicBool,
        pub support_report_data: AtomicBool,
    }

    impl MockTeeBackend {
        pub fn new(tee_type: TeeType) -> Self {
            Self {
                tee_type,
                deploy_count: AtomicUsize::new(0),
                stop_count: AtomicUsize::new(0),
                destroy_count: AtomicUsize::new(0),
                attestation_count: AtomicUsize::new(0),
                derive_pk_count: AtomicUsize::new(0),
                inject_secrets_count: AtomicUsize::new(0),
                should_fail: AtomicBool::new(false),
                support_sealed_secrets: AtomicBool::new(true),
                support_report_data: AtomicBool::new(true),
            }
        }

        pub fn failing(tee_type: TeeType) -> Self {
            let mock = Self::new(tee_type);
            mock.should_fail.store(true, Ordering::Relaxed);
            mock
        }

        fn dummy_attestation(&self) -> AttestationReport {
            AttestationReport {
                tee_type: self.tee_type.clone(),
                evidence: vec![0xDE, 0xAD],
                measurement: vec![0xBE, 0xEF],
                timestamp: 1_700_000_000,
            }
        }
    }

    #[async_trait::async_trait]
    impl TeeBackend for MockTeeBackend {
        async fn deploy(&self, params: &TeeDeployParams) -> crate::error::Result<TeeDeployment> {
            self.deploy_count.fetch_add(1, Ordering::Relaxed);
            if self.should_fail.load(Ordering::Relaxed) {
                return Err(crate::error::SandboxError::CloudProvider(
                    "Mock deploy failure".into(),
                ));
            }
            Ok(TeeDeployment {
                deployment_id: format!("mock-deploy-{}", params.sandbox_id),
                sidecar_url: format!("http://mock-tee:{}", params.http_port),
                ssh_port: params.ssh_port,
                attestation: self.dummy_attestation(),
                metadata_json: r#"{"backend":"mock"}"#.to_string(),
                extra_ports: HashMap::new(),
            })
        }

        async fn attestation(
            &self,
            _deployment_id: &str,
            _report_data: Option<[u8; 64]>,
        ) -> crate::error::Result<AttestationReport> {
            self.attestation_count.fetch_add(1, Ordering::Relaxed);
            if self.should_fail.load(Ordering::Relaxed) {
                return Err(crate::error::SandboxError::CloudProvider(
                    "Mock attestation failure".into(),
                ));
            }
            Ok(self.dummy_attestation())
        }

        async fn stop(&self, _deployment_id: &str) -> crate::error::Result<()> {
            self.stop_count.fetch_add(1, Ordering::Relaxed);
            if self.should_fail.load(Ordering::Relaxed) {
                return Err(crate::error::SandboxError::CloudProvider(
                    "Mock stop failure".into(),
                ));
            }
            Ok(())
        }

        async fn destroy(&self, _deployment_id: &str) -> crate::error::Result<()> {
            self.destroy_count.fetch_add(1, Ordering::Relaxed);
            if self.should_fail.load(Ordering::Relaxed) {
                return Err(crate::error::SandboxError::CloudProvider(
                    "Mock destroy failure".into(),
                ));
            }
            Ok(())
        }

        fn tee_type(&self) -> TeeType {
            self.tee_type.clone()
        }

        fn supports_attestation_report_data(&self) -> bool {
            self.support_report_data.load(Ordering::Relaxed)
        }

        async fn derive_public_key(
            &self,
            _deployment_id: &str,
        ) -> crate::error::Result<sealed_secrets::TeePublicKey> {
            self.derive_pk_count.fetch_add(1, Ordering::Relaxed);
            if !self.support_sealed_secrets.load(Ordering::Relaxed) {
                return Err(crate::error::SandboxError::Validation(
                    "Sealed secrets not supported by mock".into(),
                ));
            }
            Ok(sealed_secrets::TeePublicKey {
                algorithm: "x25519-hkdf-sha256".to_string(),
                public_key_bytes: vec![1, 2, 3, 4, 5, 6, 7, 8],
                attestation: self.dummy_attestation(),
            })
        }

        async fn inject_sealed_secrets(
            &self,
            _deployment_id: &str,
            _sealed: &sealed_secrets::SealedSecret,
        ) -> crate::error::Result<sealed_secrets::SealedSecretResult> {
            self.inject_secrets_count.fetch_add(1, Ordering::Relaxed);
            if !self.support_sealed_secrets.load(Ordering::Relaxed) {
                return Err(crate::error::SandboxError::Validation(
                    "Sealed secrets not supported by mock".into(),
                ));
            }
            Ok(sealed_secrets::SealedSecretResult {
                success: true,
                secrets_count: 3,
                error: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
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
            !verify_attestation(&report, &TeeType::Tdx, &[vec![0xAA, 0xBB]], None)
                .measurement_matched
        );
        assert!(
            !verify_attestation(&report, &TeeType::Tdx, &[vec![0x00]], None).measurement_matched
        );
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
}
