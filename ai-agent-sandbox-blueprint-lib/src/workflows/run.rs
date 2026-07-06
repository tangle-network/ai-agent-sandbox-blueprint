use super::*;

pub async fn run_workflow(entry: &WorkflowEntry) -> Result<WorkflowExecution, String> {
    let spec = parse_workflow_task_spec(entry.workflow_json.as_str())?;
    let record = resolve_workflow_sandbox(entry)?;

    // Fast-fail: if the sandbox has no agent configured, the sidecar will
    // reject the request with "No factory registered for agent identifier".
    // Fail immediately with a clear message instead of burning a timeout.
    if record.agent_identifier.trim().is_empty() {
        return Err(format!(
            "Sandbox '{}' has no agent configured. \
             Configure an agent identifier on the sandbox before running workflows.",
            record.id
        ));
    }

    // Look up token from sandbox record. Falls back to spec.sidecar_token for
    // backward compat with workflows created before 2-phase provisioning.
    let token = record.token.clone();
    if token.is_empty() {
        // Legacy path: use token from workflow spec
        let token_fallback = require_sidecar_token(spec.sidecar_token.as_deref().unwrap_or(""))?;
        let _record = require_sidecar_auth(&record.sidecar_url, &token_fallback)?;
    }

    // Session-per-tick: each execution gets a unique session so messages don't
    // accumulate in a single session forever. The stored session_id acts as a
    // prefix (e.g. "trading-bot123") and we append a timestamp suffix.
    let session_id = match spec.session_id {
        Some(ref base) if !base.is_empty() => {
            format!("{}-{}", base, chrono::Utc::now().timestamp())
        }
        _ => format!("wf-{}-{}", entry.id, chrono::Utc::now().timestamp()),
    };

    let sidecar_url = record.sidecar_url.clone();
    let request = SandboxTaskRequest {
        sidecar_url: sidecar_url.clone(),
        prompt: spec.prompt,
        session_id,
        max_turns: spec.max_turns.unwrap_or(0),
        model: spec.model.unwrap_or_default(),
        context_json: spec.context_json.unwrap_or_default(),
        timeout_ms: spec.timeout_ms.unwrap_or(0),
    };

    // Resolve backend profile: prefer backend_profile_json, fall back to
    // legacy system_prompt wrapped as a profile.
    let backend_profile: Option<Value> = spec
        .backend_profile_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .or_else(|| {
            spec.system_prompt
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|sp| json!({ "systemPrompt": sp }))
        });

    let response =
        run_task_request_with_profile(&request, &token, backend_profile.as_ref()).await?;
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

        // Advance next_run_at BEFORE starting the run to prevent duplicate
        // executions when the cron fires faster than the workflow completes.
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
