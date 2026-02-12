use ai_agent_instance_blueprint_lib::tangle::extract::{Caller, TangleArg, TangleResult};
use ai_agent_instance_blueprint_lib::{
    JsonResponse, ProvisionOutput, ProvisionRequest, deprovision_core, provision_core,
    set_instance_sandbox,
};

use crate::tee_backend;

pub async fn tee_provision(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<ProvisionRequest>,
) -> Result<TangleResult<ProvisionOutput>, String> {
    let backend = tee_backend();
    let (output, record) = provision_core(&request, Some(backend.as_ref())).await?;
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
