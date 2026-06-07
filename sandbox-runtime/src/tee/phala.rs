//! Phala dstack TEE backend.
//!
//! Deploys sidecar containers as Phala CVMs via `phala-tee-deploy-rs`.
//! The sidecar image is wrapped in a docker-compose.yml and deployed
//! to Phala Cloud, which runs it inside a TDX-based confidential VM.

use std::collections::HashMap;
use std::time::Duration;

use phala_tee_deploy_rs::{TeeDeployer, TeeDeployerBuilder};

use super::sealed_secrets::{SealedSecret, SealedSecretResult, TeePublicKey};
use super::{AttestationReport, TeeBackend, TeeDeployParams, TeeDeployment, TeeType};
use crate::error::{Result, SandboxError};

/// TEE backend that deploys containers to Phala Cloud CVMs.
pub struct PhalaBackend {
    deployer: TeeDeployer,
}

impl PhalaBackend {
    /// Create a new Phala backend using the given API key.
    ///
    /// Optionally provide a custom API endpoint (defaults to Phala Cloud production).
    pub fn new(api_key: &str, api_endpoint: Option<String>) -> Result<Self> {
        let mut builder = TeeDeployerBuilder::new().with_api_key(api_key);
        if let Some(endpoint) = api_endpoint {
            builder = builder.with_api_endpoint(endpoint);
        }
        let deployer = builder.build().map_err(|e| {
            SandboxError::Validation(format!("Failed to create Phala deployer: {e}"))
        })?;
        Ok(Self { deployer })
    }

    /// Build a verifiable [`AttestationReport`] from a dstack attestation
    /// response.
    ///
    /// dstack's TDX attestation carries the signed DCAP quote inside the
    /// response (commonly `tcb_info.quote` / `tcb_info.intel_quote` /
    /// `tcb_info.tdx_quote`, or a top-level `quote`). We extract that quote and
    /// emit the `{"quote":<hex>,"collateral":{...}}` envelope that
    /// [`super::verify`]'s TDX path expects, so the report can actually reach a
    /// `Verified` verdict. If no DCAP quote is present we fail closed with a
    /// precise reason rather than emitting non-verifiable `tcb_info` JSON as
    /// "evidence" (which `verify_tdx` would reject anyway, but opaquely).
    fn attestation_from_resp(
        att_resp: &phala_tee_deploy_rs::AttestationResponse,
    ) -> Result<AttestationReport> {
        let quote_hex = Self::extract_quote_hex(att_resp).ok_or_else(|| {
            SandboxError::CloudProvider(
                "Phala dstack attestation did not include a DCAP TDX quote (looked for \
                 tcb_info.quote / intel_quote / tdx_quote and a top-level quote); without the \
                 signed quote the report cannot be verified against the Intel SGX Root CA"
                    .to_string(),
            )
        })?;

        // dstack supplies the event log / collateral alongside the quote when
        // available. The verifier accepts Intel PCS collateral in the envelope;
        // when dstack does not bundle it the verifier fetches/needs collateral
        // separately, so we forward whatever collateral the response carries.
        let mut envelope = serde_json::Map::new();
        envelope.insert("quote".into(), serde_json::Value::String(quote_hex));
        if let Some(collateral) = Self::extract_collateral(att_resp) {
            envelope.insert("collateral".into(), collateral);
        }
        let evidence = serde_json::to_vec(&serde_json::Value::Object(envelope)).map_err(|e| {
            SandboxError::CloudProvider(format!(
                "failed to encode Phala TDX evidence envelope: {e}"
            ))
        })?;

        // Advisory MRTD for display/structural validation only. The trust
        // decision rebinds the measurement from inside the verified quote
        // ([`super::verify::verify_tdx`]); this operator-supplied field is never
        // trusted. dstack exposes it as `tcb_info.mrtd`. If absent we fall back
        // to the quote bytes so the report is structurally non-empty without
        // inventing a plausible-but-fake measurement.
        let measurement = Self::extract_advisory_mrtd(att_resp).unwrap_or_else(|| evidence.clone());

        Ok(AttestationReport {
            tee_type: TeeType::Tdx,
            measurement,
            evidence,
            timestamp: crate::util::now_ts(),
        })
    }

    /// Advisory (untrusted) MRTD from the dstack response, used only for display
    /// and structural validation. Never used for the trust decision.
    fn extract_advisory_mrtd(
        att_resp: &phala_tee_deploy_rs::AttestationResponse,
    ) -> Option<Vec<u8>> {
        let hex_str = att_resp
            .tcb_info
            .get("mrtd")
            .or_else(|| att_resp.tcb_info.get("mr_td"))
            .and_then(|v| v.as_str())?;
        hex::decode(hex_str.trim().trim_start_matches("0x")).ok()
    }

    /// Pull a hex-encoded DCAP quote out of a dstack attestation response,
    /// checking the field names dstack uses.
    fn extract_quote_hex(att_resp: &phala_tee_deploy_rs::AttestationResponse) -> Option<String> {
        let as_hex = |v: &serde_json::Value| -> Option<String> {
            v.as_str()
                .map(|s| s.trim().trim_start_matches("0x").to_string())
        };
        for key in ["quote", "intel_quote", "tdx_quote"] {
            if let Some(s) = att_resp.tcb_info.get(key).and_then(as_hex) {
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        if let Some(s) = att_resp.extra.get("quote").and_then(as_hex) {
            if !s.is_empty() {
                return Some(s);
            }
        }
        None
    }

    /// Pull Intel PCS collateral out of a dstack attestation response, if it
    /// bundles it.
    fn extract_collateral(
        att_resp: &phala_tee_deploy_rs::AttestationResponse,
    ) -> Option<serde_json::Value> {
        att_resp
            .tcb_info
            .get("collateral")
            .cloned()
            .or_else(|| att_resp.extra.get("collateral").cloned())
    }

    /// Build a docker-compose YAML wrapping the sidecar image.
    ///
    /// Env vars are deliberately NOT emitted into the compose `environment:`
    /// block. The compose manifest is part of the deployment request and is
    /// visible to the Phala control plane / operator in plaintext; embedding
    /// secrets there would leak them outside the enclave. Instead they are passed
    /// as the separate `env_vars` map to [`deploy_compose`], which encrypts them
    /// to the CVM's KMS public key (`deploy_with_config_do_encrypt`) so only the
    /// enclave can decrypt them. The interpolation references below let the
    /// in-CVM compose see the decrypted values without ever writing them into the
    /// manifest.
    fn compose_yaml(params: &TeeDeployParams) -> String {
        let mut yaml = String::from("services:\n  sidecar:\n");
        yaml.push_str(&format!("    image: {}\n", params.image));

        // Ports
        yaml.push_str("    ports:\n");
        yaml.push_str(&format!(
            "      - \"{}:{}\"\n",
            params.http_port, params.http_port
        ));
        if let Some(ssh) = params.ssh_port {
            yaml.push_str(&format!("      - \"{ssh}:22\"\n"));
        }

        // Reference (not embed) the encrypted env vars: the CVM-side compose
        // resolves `${KEY}` from the decrypted env injected by the KMS, so the
        // names appear in the manifest but the secret values never do.
        if !params.env_vars.is_empty() {
            yaml.push_str("    environment:\n");
            for (k, _v) in &params.env_vars {
                yaml.push_str(&format!("      - {k}=${{{k}}}\n"));
            }
        }

        // Mount dstack socket for attestation inside the CVM
        yaml.push_str("    volumes:\n");
        yaml.push_str("      - /var/run/dstack.sock:/var/run/dstack.sock\n");

        yaml
    }
}

#[async_trait::async_trait]
impl TeeBackend for PhalaBackend {
    async fn deploy(&self, params: &TeeDeployParams) -> Result<TeeDeployment> {
        let compose = Self::compose_yaml(params);

        // The env VALUES travel only in this map, which `deploy_compose` encrypts
        // to the CVM KMS key; the compose YAML carries only `${KEY}` references
        // (see `compose_yaml`). This is the single plaintext copy of the secrets,
        // held just long enough to hand to the encrypting SDK call.
        let env_vars: HashMap<String, String> = params.env_vars.iter().cloned().collect();

        let app_name = format!("sandbox-{}", &params.sandbox_id);

        let deployment = self
            .deployer
            .deploy_compose(
                &compose,
                &app_name,
                env_vars,
                Some(params.cpu_cores.max(1)),
                Some(params.memory_mb.max(1024)),
                Some(params.disk_gb.max(10)),
            )
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala deploy failed: {e}")))?;

        let app_id = deployment.id.to_string();

        // Wait for the CVM to become running.
        self.deployer
            .wait_until_running(&app_id, Duration::from_secs(120))
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala CVM failed to start: {e}")))?;

        // Fetch attestation.
        let att_resp =
            self.deployer.get_attestation(&app_id).await.map_err(|e| {
                SandboxError::Docker(format!("Phala attestation fetch failed: {e}"))
            })?;

        let attestation = Self::attestation_from_resp(&att_resp)?;

        // Get network info for the sidecar URL.
        let network = self
            .deployer
            .get_network_info(&app_id)
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala network info failed: {e}")))?;

        let sidecar_url = if !network.public_urls.app.is_empty() {
            network.public_urls.app.clone()
        } else {
            format!("http://{}:{}", network.internal_ip, params.http_port)
        };

        let metadata = serde_json::json!({
            "phala_app_id": app_id,
            "phala_internal_ip": network.internal_ip,
            "phala_public_url": network.public_urls.app,
        });

        if !params.extra_ports.is_empty() {
            tracing::warn!("Extra ports not yet supported for Phala backend — ignored");
        }

        Ok(TeeDeployment {
            deployment_id: app_id,
            sidecar_url,
            ssh_port: params.ssh_port,
            attestation,
            metadata_json: metadata.to_string(),
            extra_ports: std::collections::HashMap::new(),
        })
    }

    async fn attestation(
        &self,
        deployment_id: &str,
        _report_data: Option<[u8; 64]>,
    ) -> Result<AttestationReport> {
        let att_resp = self
            .deployer
            .get_attestation(deployment_id)
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala attestation fetch failed: {e}")))?;

        Self::attestation_from_resp(&att_resp)
    }

    async fn stop(&self, deployment_id: &str) -> Result<()> {
        self.deployer
            .shutdown(deployment_id)
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala shutdown failed: {e}")))?;
        Ok(())
    }

    async fn destroy(&self, deployment_id: &str) -> Result<()> {
        // Graceful shutdown first, then delete.
        let _ = self.deployer.shutdown(deployment_id).await;
        self.deployer
            .delete(deployment_id)
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala delete failed: {e}")))?;
        Ok(())
    }

    fn tee_type(&self) -> TeeType {
        TeeType::Tdx
    }

    // ── Sealed secrets ──────────────────────────────────────────────────────

    async fn derive_public_key(&self, deployment_id: &str) -> Result<TeePublicKey> {
        super::sidecar_derive_public_key(deployment_id).await
    }

    async fn inject_sealed_secrets(
        &self,
        deployment_id: &str,
        sealed: &SealedSecret,
    ) -> Result<SealedSecretResult> {
        super::sidecar_inject_sealed_secrets(deployment_id, sealed).await
    }
}
