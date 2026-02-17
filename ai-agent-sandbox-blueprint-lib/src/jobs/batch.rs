use serde_json::{Value, json};
use tokio::task::JoinSet;

use crate::BatchCollectRequest;
use crate::BatchCreateRequest;
use crate::BatchExecRequest;
use crate::BatchTaskRequest;
use crate::CreateSandboxParams;
use crate::JsonResponse;
use crate::runtime::{create_sidecar, require_sandbox_owner_by_url};
use crate::tangle::extract::{Caller, TangleArg, TangleResult};
use crate::jobs::exec::run_task_request;

/// Maximum number of concurrent operations in parallel batch execution.
const MAX_BATCH_CONCURRENCY: usize = 10;

pub async fn batch_create(
    Caller(caller): Caller,
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

    let mut params = CreateSandboxParams::from(&request.template_request);
    params.owner = super::caller_hex(&caller);
    let mut sandboxes_out = Vec::with_capacity(request.count as usize);
    for _ in 0..request.count {
        let (record, _) = create_sidecar(&params, None).await?;
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
    Caller(caller): Caller,
    TangleArg(request): TangleArg<BatchTaskRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    if request.sidecar_urls.is_empty() {
        return Err("Batch task requires at least one sidecar_url".to_string());
    }

    let caller_hex = super::caller_hex(&caller);
    let validated = validate_urls_with_owner(&request.sidecar_urls, &caller_hex)?;

    let results = if request.parallel {
        let mut results = vec![Value::Null; validated.len()];
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_BATCH_CONCURRENCY));
        let mut set = JoinSet::new();

        for (idx, (url, tok)) in validated.iter().enumerate() {
            let sem = sem.clone();
            let req = make_task_request(url, &request);
            let url = url.clone();
            let tok = tok.clone();
            set.spawn(async move {
                let _permit = sem.acquire().await;
                (idx, format_task_result(&url, run_task_request(&req, &tok).await))
            });
        }

        while let Some(Ok((idx, result))) = set.join_next().await {
            results[idx] = result;
        }
        results
    } else {
        let mut results = Vec::with_capacity(validated.len());
        for (url, tok) in &validated {
            let req = make_task_request(url, &request);
            results.push(format_task_result(url, run_task_request(&req, tok).await));
        }
        results
    };

    store_batch("task", results).await
}

fn make_task_request(
    sidecar_url: &str,
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
    Caller(caller): Caller,
    TangleArg(request): TangleArg<BatchExecRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    if request.sidecar_urls.is_empty() {
        return Err("Batch exec requires at least one sidecar_url".to_string());
    }

    let caller_hex = super::caller_hex(&caller);
    let validated = validate_urls_with_owner(&request.sidecar_urls, &caller_hex)?;

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
        if let Some(record) = crate::runtime::get_sandbox_by_url_opt(sidecar_url) {
            crate::runtime::touch_sandbox(&record.id);
        }
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

/// Validate caller owns all sandboxes at the given URLs. Returns (url, token) pairs.
fn validate_urls_with_owner(urls: &[String], caller: &str) -> Result<Vec<(String, String)>, String> {
    urls.iter()
        .map(|url| {
            let record = require_sandbox_owner_by_url(url, caller)?;
            Ok((url.to_string(), record.token))
        })
        .collect()
}

async fn store_batch(
    kind: &str,
    results: Vec<Value>,
) -> Result<TangleResult<JsonResponse>, String> {
    let batch_id = crate::next_batch_id();
    let record = crate::BatchRecord {
        id: batch_id.clone(),
        kind: kind.to_string(),
        results: Value::Array(results.clone()),
        created_at: crate::util::now_ts(),
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
