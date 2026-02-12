use serde_json::json;

use crate::CreateSandboxParams;
use crate::JsonResponse;
use crate::ProvisionOutput;
use crate::ProvisionRequest;
use crate::runtime::{create_sidecar, delete_sidecar};
use crate::tangle::extract::{Caller, TangleArg, TangleResult};
use crate::{clear_instance_sandbox, require_instance_sandbox, set_instance_sandbox};

pub async fn instance_provision(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<ProvisionRequest>,
) -> Result<TangleResult<ProvisionOutput>, String> {
    // Fail if already provisioned — deprovision first.
    if crate::get_instance_sandbox()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("Instance already provisioned — deprovision first".to_string());
    }

    let params = CreateSandboxParams::from(&request);
    let (record, attestation) = create_sidecar(&params, None).await.map_err(|e| e.to_string())?;

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

    let output = ProvisionOutput {
        sandbox_id: record.id.clone(),
        sidecar_url: record.sidecar_url.clone(),
        ssh_port,
        tee_attestation_json,
    };

    // Store as the instance's sandbox.
    set_instance_sandbox(record).map_err(|e| e.to_string())?;

    Ok(TangleResult(output))
}

pub async fn instance_deprovision(
    Caller(_caller): Caller,
    TangleArg(_request): TangleArg<JsonResponse>,
) -> Result<TangleResult<JsonResponse>, String> {
    let record = require_instance_sandbox()?;
    delete_sidecar(&record, None).await.map_err(|e| e.to_string())?;

    // Remove from runtime store.
    let _ = crate::runtime::sandboxes()
        .map_err(|e| e.to_string())?
        .remove(&record.id);

    clear_instance_sandbox().map_err(|e| e.to_string())?;

    let response = json!({
        "sandboxId": record.id,
        "deprovisioned": true,
    });

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}
