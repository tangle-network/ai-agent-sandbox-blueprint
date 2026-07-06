use super::*;

pub async fn bootstrap_workflows_from_chain(
    client: &blueprint_sdk::contexts::tangle::TangleClient,
    service_id: u64,
) -> Result<(), String> {
    let manager = client
        .get_blueprint_manager(service_id)
        .await
        .map_err(|err| format!("Failed to get blueprint manager: {err}"))?;
    let Some(manager) = manager else {
        return Ok(());
    };

    let abi: blueprint_sdk::alloy::json_abi::JsonAbi = serde_json::from_str(WORKFLOW_REGISTRY_ABI)
        .map_err(|err| format!("Invalid workflow ABI: {err}"))?;
    let interface = blueprint_sdk::alloy::contract::Interface::new(abi);
    let contract = blueprint_sdk::alloy::contract::ContractInstance::new(
        manager,
        client.provider().clone(),
        interface,
    );

    let ids = contract
        .function(
            "getWorkflowIds",
            &[blueprint_sdk::alloy::dyn_abi::DynSolValue::Bool(false)],
        )
        .map_err(|err| format!("Failed to build workflow IDs call: {err}"))?
        .call()
        .await
        .map_err(|err| format!("Failed to read workflow IDs: {err}"))?;

    let ids = parse_workflow_ids(ids)?;
    let existing_entries: HashMap<String, WorkflowEntry> = workflows()?
        .values()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|entry| (workflow_key(entry.id), entry))
        .collect();
    let mut entries: HashMap<String, WorkflowEntry> = HashMap::new();
    for workflow_id in ids {
        let output = contract
            .function(
                "getWorkflow",
                &[blueprint_sdk::alloy::dyn_abi::DynSolValue::Uint(
                    blueprint_sdk::alloy::primitives::U256::from_limbs([workflow_id, 0, 0, 0]),
                    64,
                )],
            )
            .map_err(|err| format!("Failed to build workflow {workflow_id} call: {err}"))?
            .call()
            .await
            .map_err(|err| format!("Failed to read workflow {workflow_id}: {err}"))?;
        let mut entry = parse_workflow_config(workflow_id, output)?;
        let key = workflow_key(workflow_id);
        merge_local_workflow_metadata(&mut entry, existing_entries.get(&key))?;
        entries.insert(key, entry);
    }

    workflows()?.replace(entries).map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn parse_workflow_ids(
    values: Vec<blueprint_sdk::alloy::dyn_abi::DynSolValue>,
) -> Result<Vec<u64>, String> {
    let first = values
        .first()
        .ok_or_else(|| "Missing workflow IDs output".to_string())?;
    let blueprint_sdk::alloy::dyn_abi::DynSolValue::Array(ids) = first else {
        return Err("Unexpected workflow IDs output type".to_string());
    };
    let mut parsed = Vec::with_capacity(ids.len());
    for value in ids {
        let blueprint_sdk::alloy::dyn_abi::DynSolValue::Uint(id, _) = value else {
            return Err("Unexpected workflow ID type".to_string());
        };
        let id: u64 = (*id)
            .try_into()
            .map_err(|_| "Workflow ID overflow".to_string())?;
        parsed.push(id);
    }
    Ok(parsed)
}

fn parse_workflow_config(
    workflow_id: u64,
    values: Vec<blueprint_sdk::alloy::dyn_abi::DynSolValue>,
) -> Result<WorkflowEntry, String> {
    let first = values
        .first()
        .ok_or_else(|| "Missing workflow output".to_string())?;
    let blueprint_sdk::alloy::dyn_abi::DynSolValue::Tuple(fields) = first else {
        return Err("Unexpected workflow output type".to_string());
    };
    if fields.len() != 12 {
        return Err("Unexpected workflow tuple size".to_string());
    }

    let name = dyn_string(&fields[0])?;
    let workflow_json = dyn_string(&fields[1])?;
    let trigger_type = dyn_string(&fields[2])?;
    let trigger_config = dyn_string(&fields[3])?;
    let sandbox_config_json = dyn_string(&fields[4])?;
    let target_kind = dyn_u8(&fields[5])?;
    let target_sandbox_id = dyn_string(&fields[6])?;
    let target_service_id = dyn_u64(&fields[7])?;
    let active = dyn_bool(&fields[8])?;
    let last_triggered_at = dyn_u64(&fields[11])?;
    let last_run_at = if last_triggered_at > 0 {
        Some(last_triggered_at)
    } else {
        None
    };
    let next_run_at = resolve_next_run(&trigger_type, &trigger_config, last_run_at)?;

    Ok(WorkflowEntry {
        id: workflow_id,
        name,
        workflow_json,
        trigger_type,
        trigger_config,
        sandbox_config_json,
        target_kind,
        target_sandbox_id,
        target_service_id,
        active,
        next_run_at,
        last_run_at,
        owner: String::new(), // On-chain workflows don't have a caller context
    })
}

pub(crate) fn dyn_string(
    value: &blueprint_sdk::alloy::dyn_abi::DynSolValue,
) -> Result<String, String> {
    match value {
        blueprint_sdk::alloy::dyn_abi::DynSolValue::String(val) => Ok(val.to_string()),
        _ => Err("Unexpected string field type".to_string()),
    }
}

pub(crate) fn dyn_bool(value: &blueprint_sdk::alloy::dyn_abi::DynSolValue) -> Result<bool, String> {
    match value {
        blueprint_sdk::alloy::dyn_abi::DynSolValue::Bool(val) => Ok(*val),
        _ => Err("Unexpected bool field type".to_string()),
    }
}

pub(crate) fn dyn_u64(value: &blueprint_sdk::alloy::dyn_abi::DynSolValue) -> Result<u64, String> {
    match value {
        blueprint_sdk::alloy::dyn_abi::DynSolValue::Uint(val, _) => (*val)
            .try_into()
            .map_err(|_| "Uint field overflow".to_string()),
        _ => Err("Unexpected uint field type".to_string()),
    }
}

fn dyn_u8(value: &blueprint_sdk::alloy::dyn_abi::DynSolValue) -> Result<u8, String> {
    match value {
        blueprint_sdk::alloy::dyn_abi::DynSolValue::Uint(val, _) => (*val)
            .try_into()
            .map_err(|_| "Uint field overflow".to_string()),
        _ => Err("Unexpected uint field type".to_string()),
    }
}

pub(crate) const WORKFLOW_REGISTRY_ABI: &str = r#"[{"type":"function","name":"getWorkflowIds","inputs":[{"name":"activeOnly","type":"bool"}],"outputs":[{"name":"","type":"uint64[]"}],"stateMutability":"view"},{"type":"function","name":"getWorkflow","inputs":[{"name":"workflowId","type":"uint64"}],"outputs":[{"name":"","type":"tuple","components":[{"name":"name","type":"string"},{"name":"workflowJson","type":"string"},{"name":"triggerType","type":"string"},{"name":"triggerConfig","type":"string"},{"name":"sandboxConfigJson","type":"string"},{"name":"targetKind","type":"uint8"},{"name":"targetSandboxId","type":"string"},{"name":"targetServiceId","type":"uint64"},{"name":"active","type":"bool"},{"name":"createdAt","type":"uint64"},{"name":"updatedAt","type":"uint64"},{"name":"lastTriggeredAt","type":"uint64"}]}],"stateMutability":"view"}]"#;
