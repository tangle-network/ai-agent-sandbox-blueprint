use serde_json::json;

use crate::CreateSandboxParams;
use crate::JsonResponse;
use crate::ProvisionOutput;
use crate::ProvisionRequest;
use crate::SandboxRecord;
use crate::runtime::{create_sidecar, delete_sidecar};
use crate::tee::TeeBackend;
use crate::{clear_instance_sandbox, require_instance_sandbox};

// ─────────────────────────────────────────────────────────────────────────────
// Core logic (reusable by TEE blueprint)
// ─────────────────────────────────────────────────────────────────────────────

/// Provision a sandbox, optionally inside a TEE.
///
/// `owner` is the hex address of the service requester (e.g. `"0xabcdef..."`).
/// When called from auto-provision, this is read from `serviceOwner(serviceId)` on-chain.
///
/// Returns the `ProvisionOutput` (for on-chain result) and the `SandboxRecord`
/// (for local persistent storage). The caller is responsible for storing the
/// record via `set_instance_sandbox`.
pub async fn provision_core(
    request: &ProvisionRequest,
    tee: Option<&dyn TeeBackend>,
    owner: &str,
) -> Result<(ProvisionOutput, SandboxRecord), String> {
    // Fail if already provisioned — deprovision first.
    if crate::get_instance_sandbox()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("Instance already provisioned — deprovision first".to_string());
    }

    let mut params = CreateSandboxParams::from(request);
    params.owner = owner.to_string();
    if request.tee_required && !request.attestation_nonce.trim().is_empty() {
        if let Some(cfg) = params.tee_config.as_mut() {
            cfg.attestation_nonce = Some(crate::tee::decode_attestation_nonce_hex(
                &request.attestation_nonce,
            )?);
        }
    }
    let (record, attestation) = create_sidecar(&params, tee)
        .await
        .map_err(|e| e.to_string())?;

    // Provision SSH key if requested.
    if request.ssh_enabled && !request.ssh_public_key.trim().is_empty() {
        sandbox_runtime::runtime::provision_ssh_key(&record, None, &request.ssh_public_key).await?;
    }

    let ssh_port = record.ssh_port.unwrap_or(0) as u32;

    let tee_attestation_json = if let Some(att) = attestation {
        serde_json::to_string(&att).unwrap_or_default()
    } else if request.tee_required {
        json!({
            "tee_type": match request.tee_type {
                1 => "tdx",
                2 => "nitro",
                3 => "sev",
                _ => "none",
            },
            "status": "pending",
        })
        .to_string()
    } else {
        String::new()
    };

    // Best-effort: fetch TEE-bound public key for sealed secret encryption.
    let tee_public_key_json =
        if let (Some(dep_id), Some(backend)) = (&record.tee_deployment_id, tee) {
            match backend.derive_public_key(dep_id).await {
                Ok(pk) => serde_json::to_string(&pk).unwrap_or_default(),
                Err(e) => {
                    blueprint_sdk::warn!(
                        sandbox_id = %record.id,
                        deployment_id = %dep_id,
                        error = %e,
                        "TEE public key derivation failed — sealed secrets will not be available"
                    );
                    String::new()
                }
            }
        } else {
            String::new()
        };

    let output = ProvisionOutput {
        sandbox_id: record.id.clone(),
        sidecar_url: record.sidecar_url.clone(),
        ssh_port,
        tee_attestation_json,
        tee_public_key_json,
    };

    Ok((output, record))
}

/// Deprovision the instance sandbox, optionally tearing down a TEE deployment.
///
/// Returns the JSON response body and the sandbox ID that was deprovisioned.
pub async fn deprovision_core(
    tee: Option<&dyn TeeBackend>,
) -> Result<(JsonResponse, String), String> {
    let record = require_instance_sandbox()?;
    delete_sidecar(&record, tee)
        .await
        .map_err(|e| e.to_string())?;

    // Remove from runtime store.
    let _ = crate::runtime::sandboxes()
        .map_err(|e| e.to_string())?
        .remove(&record.id);

    clear_instance_sandbox().map_err(|e| e.to_string())?;

    let sandbox_id = record.id.clone();
    let response = json!({
        "sandboxId": sandbox_id,
        "deprovisioned": true,
    });

    Ok((
        JsonResponse {
            json: response.to_string(),
        },
        sandbox_id,
    ))
}
