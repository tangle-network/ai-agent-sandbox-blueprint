use super::*;

pub async fn run_workflow(entry: &WorkflowEntry) -> Result<WorkflowExecution, String> {
    if entry.workflow_json.trim().is_empty() {
        return Err("workflow_json is required".to_string());
    }
    if entry.target_kind != WORKFLOW_TARGET_INSTANCE {
        return Err("workflow target is not an instance".to_string());
    }
    if entry.target_service_id == 0 {
        return Err("workflow target_service_id is required".to_string());
    }

    let spec: WorkflowTaskSpec = serde_json::from_str(entry.workflow_json.as_str())
        .map_err(|err| format!("workflow_json must be valid task JSON: {err}"))?;

    let sandbox = crate::require_instance_sandbox()?;

    // Fast-fail: if the instance has no agent configured, the sidecar will
    // reject the request with "No factory registered for agent identifier".
    if sandbox.agent_identifier.trim().is_empty() {
        return Err("Instance has no agent configured. \
             Configure an agent identifier on the instance before running workflows."
            .to_string());
    }

    match sandbox.service_id {
        Some(service_id) if service_id == entry.target_service_id => {}
        Some(service_id) => {
            return Err(format!(
                "Instance workflow targets service {} but local instance is bound to service {}",
                entry.target_service_id, service_id
            ));
        }
        None => return Err("Local instance sandbox is missing service binding".to_string()),
    }

    // Session-per-tick: each execution gets a unique session so messages don't
    // accumulate in a single session forever. The stored session_id acts as a
    // prefix and we append a timestamp suffix.
    let session_id = match spec.session_id {
        Some(ref base) if !base.is_empty() => {
            format!("{}-{}", base, chrono::Utc::now().timestamp())
        }
        _ => format!("wf-{}-{}", entry.id, chrono::Utc::now().timestamp()),
    };

    let request = InstanceTaskRequest {
        prompt: spec.prompt,
        session_id,
        max_turns: spec.max_turns.unwrap_or(0),
        model: spec.model.unwrap_or_default(),
        context_json: spec.context_json.unwrap_or_default(),
        timeout_ms: spec.timeout_ms.unwrap_or(0),
    };

    let response =
        run_instance_task(&sandbox.sidecar_url, &sandbox.token, &sandbox.id, &request).await?;
    let now = now_ts();
    let next_run_at = resolve_next_run(&entry.trigger_type, &entry.trigger_config, Some(now))?;
    let latest_execution = WorkflowLatestExecution {
        executed_at: now,
        success: response.success,
        result: response.result.clone(),
        error: response.error.clone(),
        trace_id: response.trace_id.clone(),
        duration_ms: response.duration_ms,
        input_tokens: response.input_tokens,
        output_tokens: response.output_tokens,
        session_id: response.session_id.clone(),
    };

    Ok(WorkflowExecution {
        response: json!({
            "workflowId": entry.id,
            "name": entry.name,
            "status": if entry.active { "active" } else { "inactive" },
            "executedAt": now,
            "sandboxConfigJson": entry.sandbox_config_json,
            "task": {
                "success": response.success,
                "result": response.result,
                "error": response.error,
                "traceId": response.trace_id,
                "durationMs": response.duration_ms,
                "inputTokens": response.input_tokens,
                "outputTokens": response.output_tokens,
                "sessionId": response.session_id,
            }
        }),
        last_run_at: now,
        next_run_at,
        latest_execution,
    })
}

pub fn apply_workflow_execution(
    entry: &mut WorkflowEntry,
    last_run_at: u64,
    next_run_at: Option<u64>,
) {
    entry.last_run_at = Some(last_run_at);
    entry.next_run_at = next_run_at;
}

pub async fn workflow_tick() -> Result<Value, String> {
    let now = now_ts();
    let all = workflows()?.values().map_err(|e| e.to_string())?;

    let due: Vec<u64> = all
        .iter()
        .filter(|e| e.active && e.trigger_type == "cron")
        .filter(|entry| {
            !matches!(
                resolve_workflow_target_status(entry),
                Ok(WorkflowTargetStatus::Missing)
            )
        })
        .filter_map(|e| e.next_run_at.filter(|&t| t <= now).map(|_| e.id))
        .collect();

    let mut executed = Vec::new();
    for workflow_id in due {
        let _run_guard = match acquire_workflow_run(workflow_id) {
            Ok(guard) => guard,
            Err(_) => {
                tracing::debug!("Workflow {workflow_id} already running, skipping");
                continue;
            }
        };

        let key = workflow_key(workflow_id);
        let entry = match workflows()?.get(&key).map_err(|e| e.to_string())? {
            Some(e) if e.active => e,
            _ => continue,
        };

        // Advance next_run_at before running to avoid duplicate executions when
        // cron fires faster than task completion.
        let tentative_next =
            resolve_next_run(&entry.trigger_type, &entry.trigger_config, Some(now))
                .ok()
                .flatten();
        workflows()?
            .update(&key, |e| {
                e.next_run_at = tentative_next;
            })
            .map_err(|e| e.to_string())?;

        match run_workflow(&entry).await {
            Ok(execution) => {
                let last_run_at = execution.last_run_at;
                let next_run_at = execution.next_run_at;
                store_latest_execution(workflow_id, execution.latest_execution.clone())?;
                workflows()?
                    .update(&key, |e| {
                        e.last_run_at = Some(last_run_at);
                        e.next_run_at = next_run_at;
                    })
                    .map_err(|e| e.to_string())?;
                executed.push(execution.response);
            }
            Err(err) => {
                store_failed_execution(workflow_id, err.clone())?;
                executed.push(json!({
                    "workflowId": workflow_id,
                    "status": "error",
                    "error": err,
                }));
            }
        }
    }

    Ok(json!({
        "executed": executed,
        "count": executed.len(),
    }))
}
