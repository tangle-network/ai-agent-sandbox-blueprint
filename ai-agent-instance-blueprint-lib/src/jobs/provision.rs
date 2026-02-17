use serde_json::json;

use crate::CreateSandboxParams;
use crate::JsonResponse;
use crate::ProvisionOutput;
use crate::ProvisionRequest;
use crate::SandboxRecord;
use crate::runtime::{create_sidecar, delete_sidecar};
use crate::tangle::extract::{Caller, TangleArg, TangleResult};
use crate::tee::TeeBackend;
use crate::{clear_instance_sandbox, require_instance_sandbox, set_instance_sandbox};

// ─────────────────────────────────────────────────────────────────────────────
// Core logic (reusable by TEE blueprint)
// ─────────────────────────────────────────────────────────────────────────────

/// Provision a sandbox, optionally inside a TEE.
///
/// Returns the `ProvisionOutput` (for on-chain result) and the `SandboxRecord`
/// (for local persistent storage). The caller is responsible for storing the
/// record via `set_instance_sandbox`.
pub async fn provision_core(
    request: &ProvisionRequest,
    tee: Option<&dyn TeeBackend>,
) -> Result<(ProvisionOutput, SandboxRecord), String> {
    // Fail if already provisioned — deprovision first.
    if crate::get_instance_sandbox()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("Instance already provisioned — deprovision first".to_string());
    }

    let params = CreateSandboxParams::from(request);
    let (record, attestation) = create_sidecar(&params, tee)
        .await
        .map_err(|e| e.to_string())?;

    // Provision SSH key if requested.
    if request.ssh_enabled && !request.ssh_public_key.trim().is_empty() {
        crate::jobs::ssh::provision_key(
            &record.sidecar_url,
            "root",
            &request.ssh_public_key,
            &record.token,
        )
        .await?;
    }

    let ssh_port = record.ssh_port.unwrap_or(0) as u32;

    let tee_attestation_json = if let Some(att) = attestation {
        serde_json::to_string(&att).unwrap_or_default()
    } else if request.tee_required {
        json!({
            "tee_type": match request.tee_type {
                1 => "sgx",
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
    let tee_public_key_json = if let (Some(dep_id), Some(backend)) =
        (&record.tee_deployment_id, tee)
    {
        match backend.derive_public_key(dep_id).await {
            Ok(pk) => serde_json::to_string(&pk).unwrap_or_default(),
            Err(_) => String::new(), // sidecar may not implement this yet
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

// ─────────────────────────────────────────────────────────────────────────────
// Job handlers (thin wrappers — pass None for TEE backend)
// ─────────────────────────────────────────────────────────────────────────────

pub async fn instance_provision(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<ProvisionRequest>,
) -> Result<TangleResult<ProvisionOutput>, String> {
    let (output, record) = provision_core(&request, None).await?;
    set_instance_sandbox(record).map_err(|e| e.to_string())?;
    Ok(TangleResult(output))
}

pub async fn instance_deprovision(
    Caller(_caller): Caller,
    TangleArg(_request): TangleArg<JsonResponse>,
) -> Result<TangleResult<JsonResponse>, String> {
    let (response, _sandbox_id) = deprovision_core(None).await?;
    Ok(TangleResult(response))
}
