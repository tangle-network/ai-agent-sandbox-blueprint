use serde_json::{Value, json};
use tokio::task::JoinSet;

use crate::BatchCollectRequest;
use crate::BatchCreateRequest;
use crate::BatchExecRequest;
use crate::BatchTaskRequest;
use crate::JsonResponse;
use crate::auth::require_sidecar_token;
use crate::runtime::{create_sidecar, require_sidecar_auth};
use crate::tangle::extract::{Caller, TangleArg, TangleResult};
use crate::workflows::run_task_request;

/// Maximum number of concurrent operations in parallel batch execution.
const MAX_BATCH_CONCURRENCY: usize = 10;

pub async fn batch_create(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<BatchCreateRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    if request.count == 0 {
        return Err("Batch create requires count > 0".to_string());
    }
    if request.count > crate::MAX_BATCH_COUNT {
        return Err(format!(
            "Batch count exceeds max {}",
            crate::MAX_BATCH_COUNT
        ));
    }

    if !request.template_request.sidecar_token.trim().is_empty() {
        return Err(
            "Batch create must not reuse sidecar_token; leave blank to auto-generate".to_string(),
        );
    }

    let mut sandboxes_out = Vec::with_capacity(request.count as usize);
    for _ in 0..request.count {
        let record = create_sidecar(&request.template_request).await?;
        crate::metrics::metrics().record_sandbox_created(
            request.template_request.cpu_cores,
            request.template_request.memory_mb,
        );
        sandboxes_out.push(json!({
            "sandboxId": record.id,
            "sidecarUrl": record.sidecar_url,
            "token": record.token,
            "sshPort": record.ssh_port,
        }));
    }

    let response = json!({
        "batchId": crate::next_batch_id(),
        "sandboxes": sandboxes_out,
    });

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn batch_task(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<BatchTaskRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    if request.sidecar_urls.is_empty() {
        return Err("Batch task requires at least one sidecar_url".to_string());
    }
    if request.sidecar_tokens.len() != request.sidecar_urls.len() {
        return Err("Batch task requires one sidecar_token per sidecar_url".to_string());
    }

    // Validate all tokens upfront before starting any work
    let validated: Vec<(String, String)> = request
        .sidecar_urls
        .iter()
        .zip(request.sidecar_tokens.iter())
        .map(|(url, tok)| {
            let token = require_sidecar_token(tok)?;
            require_sidecar_auth(url, &token)?;
            Ok((url.to_string(), token))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let results = if request.parallel {
        run_batch_tasks_parallel(&validated, &request).await
    } else {
        run_batch_tasks_sequential(&validated, &request).await
    };

    let batch_id = crate::next_batch_id();
    let record = crate::BatchRecord {
        id: batch_id.clone(),
        kind: "task".to_string(),
        results: Value::Array(results.clone()),
        created_at: crate::workflows::now_ts(),
    };

    crate::batches()
        .map_err(|e| e.to_string())?
        .insert(batch_id.clone(), record)
        .map_err(|e| e.to_string())?;

    let response = json!({
        "batchId": batch_id,
        "taskResults": results,
    });

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}

async fn run_batch_tasks_sequential(
    validated: &[(String, String)],
    request: &BatchTaskRequest,
) -> Vec<Value> {
    let mut results = Vec::with_capacity(validated.len());
    for (sidecar_url, token) in validated {
        let result = run_single_task(sidecar_url, token, request).await;
        results.push(result);
    }
    results
}

async fn run_batch_tasks_parallel(
    validated: &[(String, String)],
    request: &BatchTaskRequest,
) -> Vec<Value> {
    let mut results = vec![Value::Null; validated.len()];
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_BATCH_CONCURRENCY));

    let mut set = JoinSet::new();
    for (idx, (sidecar_url, token)) in validated.iter().enumerate() {
        let sem = semaphore.clone();
        let url = sidecar_url.clone();
        let tok = token.clone();
        let task_req = crate::SandboxTaskRequest {
            sidecar_url: url.clone(),
            prompt: request.prompt.to_string(),
            session_id: request.session_id.to_string(),
            max_turns: request.max_turns,
            model: request.model.to_string(),
            context_json: request.context_json.to_string(),
            timeout_ms: request.timeout_ms,
            sidecar_token: tok.clone(),
        };

        set.spawn(async move {
            let _permit = sem.acquire().await;
            let result = run_task_request(&task_req)
                .await
                .map(|resp| {
                    json!({
                        "sidecarUrl": url,
                        "success": resp.success,
                        "result": resp.result,
                        "error": resp.error,
                        "traceId": resp.trace_id,
                        "durationMs": resp.duration_ms,
                        "inputTokens": resp.input_tokens,
                        "outputTokens": resp.output_tokens,
                        "sessionId": resp.session_id,
                    })
                })
                .unwrap_or_else(|err| {
                    json!({
                        "sidecarUrl": url,
                        "success": false,
                        "error": err,
                    })
                });
            (idx, result)
        });
    }

    while let Some(Ok((idx, result))) = set.join_next().await {
        results[idx] = result;
    }

    results
}

async fn run_single_task(sidecar_url: &str, token: &str, request: &BatchTaskRequest) -> Value {
    let task_request = crate::SandboxTaskRequest {
        sidecar_url: sidecar_url.to_string(),
        prompt: request.prompt.to_string(),
        session_id: request.session_id.to_string(),
        max_turns: request.max_turns,
        model: request.model.to_string(),
        context_json: request.context_json.to_string(),
        timeout_ms: request.timeout_ms,
        sidecar_token: token.to_string(),
    };

    run_task_request(&task_request)
        .await
        .map(|response| {
            json!({
                "sidecarUrl": sidecar_url,
                "success": response.success,
                "result": response.result,
                "error": response.error,
                "traceId": response.trace_id,
                "durationMs": response.duration_ms,
                "inputTokens": response.input_tokens,
                "outputTokens": response.output_tokens,
                "sessionId": response.session_id,
            })
        })
        .unwrap_or_else(|err| {
            json!({
                "sidecarUrl": sidecar_url,
                "success": false,
                "error": err,
            })
        })
}

pub async fn batch_exec(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<BatchExecRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    if request.sidecar_urls.is_empty() {
        return Err("Batch exec requires at least one sidecar_url".to_string());
    }
    if request.sidecar_tokens.len() != request.sidecar_urls.len() {
        return Err("Batch exec requires one sidecar_token per sidecar_url".to_string());
    }

    // Validate all tokens upfront
    let validated: Vec<(String, String)> = request
        .sidecar_urls
        .iter()
        .zip(request.sidecar_tokens.iter())
        .map(|(url, tok)| {
            let token = require_sidecar_token(tok)?;
            require_sidecar_auth(url, &token)?;
            Ok((url.to_string(), token))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let results = if request.parallel {
        run_batch_exec_parallel(&validated, &request).await
    } else {
        run_batch_exec_sequential(&validated, &request).await
    };

    let batch_id = crate::next_batch_id();
    let record = crate::BatchRecord {
        id: batch_id.clone(),
        kind: "exec".to_string(),
        results: Value::Array(results.clone()),
        created_at: crate::workflows::now_ts(),
    };

    crate::batches()
        .map_err(|e| e.to_string())?
        .insert(batch_id.clone(), record)
        .map_err(|e| e.to_string())?;

    let response = json!({
        "batchId": batch_id,
        "execResults": results,
    });

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}

async fn run_batch_exec_sequential(
    validated: &[(String, String)],
    request: &BatchExecRequest,
) -> Vec<Value> {
    let mut results = Vec::with_capacity(validated.len());
    for (sidecar_url, token) in validated {
        let result = run_single_exec(sidecar_url, token, request).await;
        results.push(result);
    }
    results
}

async fn run_batch_exec_parallel(
    validated: &[(String, String)],
    request: &BatchExecRequest,
) -> Vec<Value> {
    let mut results = vec![Value::Null; validated.len()];
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_BATCH_CONCURRENCY));

    let mut set = JoinSet::new();
    for (idx, (sidecar_url, token)) in validated.iter().enumerate() {
        let sem = semaphore.clone();
        let url = sidecar_url.clone();
        let tok = token.clone();
        let cmd = request.command.to_string();
        let cwd = request.cwd.to_string();
        let env_json = request.env_json.to_string();
        let timeout_ms = request.timeout_ms;

        set.spawn(async move {
            let _permit = sem.acquire().await;
            run_single_exec_owned(&url, &tok, &cmd, &cwd, &env_json, timeout_ms)
                .await
                .map(|v| (idx, v))
                .unwrap_or_else(|_| (idx, json!({"sidecarUrl": url, "error": "exec failed"})))
        });
    }

    while let Some(Ok((idx, result))) = set.join_next().await {
        results[idx] = result;
    }

    results
}

async fn run_single_exec(sidecar_url: &str, token: &str, request: &BatchExecRequest) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "command".to_string(),
        Value::String(request.command.to_string()),
    );
    if !request.cwd.is_empty() {
        payload.insert("cwd".to_string(), Value::String(request.cwd.to_string()));
    }
    if request.timeout_ms > 0 {
        payload.insert("timeout".to_string(), json!(request.timeout_ms));
    }
    if !request.env_json.trim().is_empty() {
        if let Ok(Some(env_map)) = crate::util::parse_json_object(&request.env_json, "env_json") {
            payload.insert("env".to_string(), env_map);
        }
    }

    crate::http::sidecar_post_json(sidecar_url, "/exec", token, Value::Object(payload))
        .await
        .map(|parsed| {
            json!({
                "sidecarUrl": sidecar_url,
                "exitCode": parsed.get("exitCode").and_then(Value::as_u64).unwrap_or(0),
                "stdout": parsed.get("stdout").and_then(Value::as_str).unwrap_or_default(),
                "stderr": parsed.get("stderr").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .unwrap_or_else(|err| {
            json!({
                "sidecarUrl": sidecar_url,
                "error": err.to_string(),
            })
        })
}

async fn run_single_exec_owned(
    sidecar_url: &str,
    token: &str,
    command: &str,
    cwd: &str,
    env_json: &str,
    timeout_ms: u64,
) -> Result<Value, String> {
    let mut payload = serde_json::Map::new();
    payload.insert("command".to_string(), Value::String(command.to_string()));
    if !cwd.is_empty() {
        payload.insert("cwd".to_string(), Value::String(cwd.to_string()));
    }
    if timeout_ms > 0 {
        payload.insert("timeout".to_string(), json!(timeout_ms));
    }
    if !env_json.trim().is_empty() {
        if let Ok(Some(env_map)) = crate::util::parse_json_object(env_json, "env_json") {
            payload.insert("env".to_string(), env_map);
        }
    }

    let parsed =
        crate::http::sidecar_post_json(sidecar_url, "/exec", token, Value::Object(payload))
            .await
            .map_err(|e| e.to_string())?;

    Ok(json!({
        "sidecarUrl": sidecar_url,
        "exitCode": parsed.get("exitCode").and_then(Value::as_u64).unwrap_or(0),
        "stdout": parsed.get("stdout").and_then(Value::as_str).unwrap_or_default(),
        "stderr": parsed.get("stderr").and_then(Value::as_str).unwrap_or_default(),
    }))
}

pub async fn batch_collect(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<BatchCollectRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let batch_id = request.batch_id.to_string();
    let record = crate::batches()
        .map_err(|e| e.to_string())?
        .remove(&batch_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Batch not found".to_string())?;

    let response = json!({
        "batchId": record.id,
        "kind": record.kind,
        "results": record.results,
    });

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}
