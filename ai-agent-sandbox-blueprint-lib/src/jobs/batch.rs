use serde_json::{Value, json};

use crate::BatchCollectRequest;
use crate::BatchCreateRequest;
use crate::BatchExecRequest;
use crate::BatchTaskRequest;
use crate::JsonResponse;
use crate::auth::require_sidecar_token;
use crate::runtime::{create_sidecar, require_sidecar_auth};
use crate::tangle_evm::extract::{Caller, TangleEvmArg, TangleEvmResult};
use crate::workflows::run_task_request;

pub async fn batch_create(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<BatchCreateRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
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

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn batch_task(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<BatchTaskRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    if request.sidecar_urls.is_empty() {
        return Err("Batch task requires at least one sidecar_url".to_string());
    }
    if request.sidecar_tokens.len() != request.sidecar_urls.len() {
        return Err("Batch task requires one sidecar_token per sidecar_url".to_string());
    }

    let mut results = Vec::with_capacity(request.sidecar_urls.len());

    for (sidecar_url, token_raw) in request
        .sidecar_urls
        .iter()
        .zip(request.sidecar_tokens.iter())
    {
        let token = require_sidecar_token(token_raw)?;
        require_sidecar_auth(sidecar_url, &token)?;

        let task_request = crate::SandboxTaskRequest {
            sidecar_url: sidecar_url.to_string(),
            prompt: request.prompt.to_string(),
            session_id: request.session_id.to_string(),
            max_turns: request.max_turns,
            model: request.model.to_string(),
            context_json: request.context_json.to_string(),
            timeout_ms: request.timeout_ms,
            sidecar_token: token.clone(),
        };

        let result = run_task_request(
            &task_request,
            crate::runtime::SidecarRuntimeConfig::load().timeout,
        )
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
        });

        results.push(result);
    }

    let batch_id = crate::next_batch_id();
    let record = crate::BatchRecord {
        id: batch_id.clone(),
        kind: "task".to_string(),
        results: Value::Array(results.clone()),
    };

    crate::batches()?
        .lock()
        .map_err(|_| "Batch store poisoned".to_string())?
        .insert(batch_id.clone(), record);

    let response = json!({
        "batchId": batch_id,
        "taskResults": results,
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn batch_exec(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<BatchExecRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    if request.sidecar_urls.is_empty() {
        return Err("Batch exec requires at least one sidecar_url".to_string());
    }
    if request.sidecar_tokens.len() != request.sidecar_urls.len() {
        return Err("Batch exec requires one sidecar_token per sidecar_url".to_string());
    }

    let mut results = Vec::with_capacity(request.sidecar_urls.len());

    for (sidecar_url, token_raw) in request
        .sidecar_urls
        .iter()
        .zip(request.sidecar_tokens.iter())
    {
        let token = require_sidecar_token(token_raw)?;
        require_sidecar_auth(sidecar_url, &token)?;

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
            let env_map = crate::util::parse_json_object(&request.env_json, "env_json")?;
            if let Some(env_map) = env_map {
                payload.insert("env".to_string(), env_map);
            }
        }

        let parsed = crate::http::sidecar_post_json(
            sidecar_url,
            "/exec",
            &token,
            Value::Object(payload),
            crate::runtime::SidecarRuntimeConfig::load().timeout,
        )
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
                "error": err,
            })
        });

        results.push(parsed);
    }

    let batch_id = crate::next_batch_id();
    let record = crate::BatchRecord {
        id: batch_id.clone(),
        kind: "exec".to_string(),
        results: Value::Array(results.clone()),
    };

    crate::batches()?
        .lock()
        .map_err(|_| "Batch store poisoned".to_string())?
        .insert(batch_id.clone(), record);

    let response = json!({
        "batchId": batch_id,
        "execResults": results,
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn batch_collect(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<BatchCollectRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let store = crate::batches()?;
    let mut store = store
        .lock()
        .map_err(|_| "Batch store poisoned".to_string())?;
    let record = store
        .remove(request.batch_id.as_str())
        .ok_or_else(|| "Batch not found".to_string())?;

    let response = json!({
        "batchId": record.id,
        "kind": record.kind,
        "results": record.results,
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}
