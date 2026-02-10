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
        // create_sidecar() records metrics internally.
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

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}

// ---------------------------------------------------------------------------
// Batch task
// ---------------------------------------------------------------------------

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

    let validated = validate_tokens(&request.sidecar_urls, &request.sidecar_tokens)?;

    let results = if request.parallel {
        let mut results = vec![Value::Null; validated.len()];
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_BATCH_CONCURRENCY));
        let mut set = JoinSet::new();

        for (idx, (url, tok)) in validated.iter().enumerate() {
            let sem = sem.clone();
            let req = make_task_request(url, tok, &request);
            let url = url.clone();
            set.spawn(async move {
                let _permit = sem.acquire().await;
                (idx, format_task_result(&url, run_task_request(&req).await))
            });
        }

        while let Some(Ok((idx, result))) = set.join_next().await {
            results[idx] = result;
        }
        results
    } else {
        let mut results = Vec::with_capacity(validated.len());
        for (url, tok) in &validated {
            let req = make_task_request(url, tok, &request);
            results.push(format_task_result(url, run_task_request(&req).await));
        }
        results
    };

    store_batch("task", results).await
}

fn make_task_request(
    sidecar_url: &str,
    token: &str,
    request: &BatchTaskRequest,
) -> crate::SandboxTaskRequest {
    crate::SandboxTaskRequest {
        sidecar_url: sidecar_url.to_string(),
        prompt: request.prompt.to_string(),
        session_id: request.session_id.to_string(),
        max_turns: request.max_turns,
        model: request.model.to_string(),
        context_json: request.context_json.to_string(),
        timeout_ms: request.timeout_ms,
        sidecar_token: token.to_string(),
    }
}

fn format_task_result(
    sidecar_url: &str,
    result: Result<crate::SandboxTaskResponse, String>,
) -> Value {
    match result {
        Ok(resp) => json!({
            "sidecarUrl": sidecar_url,
            "success": resp.success,
            "result": resp.result,
            "error": resp.error,
            "traceId": resp.trace_id,
            "durationMs": resp.duration_ms,
            "inputTokens": resp.input_tokens,
            "outputTokens": resp.output_tokens,
            "sessionId": resp.session_id,
        }),
        Err(err) => json!({
            "sidecarUrl": sidecar_url,
            "success": false,
            "error": err,
        }),
    }
}

// ---------------------------------------------------------------------------
// Batch exec
// ---------------------------------------------------------------------------

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

    let validated = validate_tokens(&request.sidecar_urls, &request.sidecar_tokens)?;

    let results = if request.parallel {
        let mut results = vec![Value::Null; validated.len()];
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_BATCH_CONCURRENCY));
        let mut set = JoinSet::new();

        for (idx, (url, tok)) in validated.iter().enumerate() {
            let sem = sem.clone();
            let url = url.clone();
            let tok = tok.clone();
            let payload = crate::jobs::exec::build_exec_payload(
                &request.command,
                &request.cwd,
                &request.env_json,
                request.timeout_ms,
            );
            set.spawn(async move {
                let _permit = sem.acquire().await;
                (idx, exec_and_format(&url, &tok, payload).await)
            });
        }

        while let Some(Ok((idx, result))) = set.join_next().await {
            results[idx] = result;
        }
        results
    } else {
        let mut results = Vec::with_capacity(validated.len());
        for (url, tok) in &validated {
            let payload = crate::jobs::exec::build_exec_payload(
                &request.command,
                &request.cwd,
                &request.env_json,
                request.timeout_ms,
            );
            results.push(exec_and_format(url, tok, payload).await);
        }
        results
    };

    store_batch("exec", results).await
}

async fn exec_and_format(
    sidecar_url: &str,
    token: &str,
    payload: serde_json::Map<String, Value>,
) -> Value {
    crate::http::sidecar_post_json(
        sidecar_url,
        "/terminals/commands",
        token,
        Value::Object(payload),
    )
    .await
    .map(|parsed| {
        let (exit_code, stdout, stderr) = crate::jobs::exec::extract_exec_fields(&parsed);
        json!({
            "sidecarUrl": sidecar_url,
            "exitCode": exit_code,
            "stdout": stdout,
            "stderr": stderr,
        })
    })
    .unwrap_or_else(|err| {
        json!({
            "sidecarUrl": sidecar_url,
            "error": err.to_string(),
        })
    })
}

// ---------------------------------------------------------------------------
// Batch collect
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn validate_tokens(urls: &[String], tokens: &[String]) -> Result<Vec<(String, String)>, String> {
    urls.iter()
        .zip(tokens.iter())
        .map(|(url, tok)| {
            let token = require_sidecar_token(tok)?;
            require_sidecar_auth(url, &token)?;
            Ok((url.to_string(), token))
        })
        .collect()
}

async fn store_batch(kind: &str, results: Vec<Value>) -> Result<TangleResult<JsonResponse>, String> {
    let batch_id = crate::next_batch_id();
    let record = crate::BatchRecord {
        id: batch_id.clone(),
        kind: kind.to_string(),
        results: Value::Array(results.clone()),
        created_at: crate::workflows::now_ts(),
    };

    crate::batches()
        .map_err(|e| e.to_string())?
        .insert(batch_id.clone(), record)
        .map_err(|e| e.to_string())?;

    let results_key = format!("{kind}Results");
    let response = json!({
        "batchId": batch_id,
        results_key: results,
    });

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}
