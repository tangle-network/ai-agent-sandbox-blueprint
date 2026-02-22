use ai_agent_instance_blueprint_lib::tangle::extract::{Caller, TangleArg, TangleResult};
use ai_agent_instance_blueprint_lib::{
    JsonResponse, ProvisionOutput, ProvisionRequest, deprovision_core, provision_core,
    set_instance_sandbox,
};

use crate::tee_backend;

pub async fn tee_provision(
    Caller(caller): Caller,
    TangleArg(request): TangleArg<ProvisionRequest>,
) -> Result<TangleResult<ProvisionOutput>, String> {
    let caller_hex = super::caller_hex(&caller);

    // Idempotent: if auto-provision already created the sandbox, return existing info.
    if let Some(record) =
        ai_agent_instance_blueprint_lib::get_instance_sandbox().map_err(|e| e.to_string())?
    {
        let output = ProvisionOutput {
            sandbox_id: record.id.clone(),
            sidecar_url: record.sidecar_url.clone(),
            ssh_port: record.ssh_port.unwrap_or(0) as u32,
            tee_attestation_json: String::new(),
            tee_public_key_json: String::new(),
        };
        return Ok(TangleResult(output));
    }

    let backend = tee_backend();
    let (output, record) = provision_core(&request, Some(backend.as_ref()), &caller_hex).await?;
    set_instance_sandbox(record).map_err(|e| e.to_string())?;
    Ok(TangleResult(output))
}

pub async fn tee_deprovision(
    Caller(_caller): Caller,
    TangleArg(_request): TangleArg<JsonResponse>,
) -> Result<TangleResult<JsonResponse>, String> {
    let backend = tee_backend();
    let (response, _sandbox_id) = deprovision_core(Some(backend.as_ref())).await?;
    Ok(TangleResult(response))
}
