use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode, Url};
use serde_json::Value;
use std::time::Duration;

use crate::util::http_client;

pub fn build_url(base: &str, path: &str) -> Result<Url, String> {
    let base_url = Url::parse(base).map_err(|err| format!("Invalid base URL: {err}"))?;
    base_url
        .join(path)
        .map_err(|err| format!("Invalid path '{path}': {err}"))
}

pub fn auth_headers(token: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let value = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| "Invalid auth token".to_string())?;
    headers.insert(AUTHORIZATION, value);

    Ok(headers)
}

pub async fn send_json(
    method: Method,
    url: Url,
    body: Option<Value>,
    headers: HeaderMap,
    timeout: Duration,
) -> Result<(StatusCode, String), String> {
    let client = http_client(timeout)?;
    let mut request = client.request(method, url).headers(headers);
    if let Some(body) = body {
        request = request.json(&body);
    }

    let response = request
        .send()
        .await
        .map_err(|err| format!("HTTP request failed: {err}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| format!("Failed to read response body: {err}"))?;

    if !status.is_success() {
        return Err(format!("HTTP {status}: {text}"));
    }

    Ok((status, text))
}

pub async fn sidecar_post_json(
    sidecar_url: &str,
    path: &str,
    token: &str,
    payload: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let url = build_url(sidecar_url, path)?;
    let headers = auth_headers(token)?;
    let (_, body) = send_json(Method::POST, url, Some(payload), headers, timeout).await?;
    serde_json::from_str(&body).map_err(|err| format!("Invalid sidecar response JSON: {err}"))
}
