use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest::{Client, Method, StatusCode, Url};
use serde_json::Value;

use crate::error::{Result, SandboxError};
use crate::util::{http_client, http_client_no_timeout};

pub fn build_url(base: &str, path: &str) -> Result<Url> {
    let base_url =
        Url::parse(base).map_err(|err| SandboxError::Http(format!("Invalid base URL: {err}")))?;
    base_url
        .join(path)
        .map_err(|err| SandboxError::Http(format!("Invalid path '{path}': {err}")))
}

pub fn auth_headers(token: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let value = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| SandboxError::Auth("Invalid auth token".into()))?;
    headers.insert(AUTHORIZATION, value);

    Ok(headers)
}

async fn send_json_with_client(
    client: &Client,
    method: Method,
    url: Url,
    body: Option<Value>,
    headers: HeaderMap,
) -> Result<(StatusCode, String)> {
    let mut request = client.request(method, url).headers(headers);
    if let Some(body) = body {
        request = request.json(&body);
    }

    let response = request.send().await.map_err(|err| {
        tracing::error!("reqwest send failed: {err:?}");
        SandboxError::Http(format!("HTTP request failed: {err}"))
    })?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| SandboxError::Http(format!("Failed to read response body: {err}")))?;

    if !status.is_success() {
        return Err(SandboxError::Http(format!("HTTP {status}: {text}")));
    }

    Ok((status, text))
}

pub async fn send_json(
    method: Method,
    url: Url,
    body: Option<Value>,
    headers: HeaderMap,
) -> Result<(StatusCode, String)> {
    let client = http_client()?;
    send_json_with_client(client, method, url, body, headers).await
}

pub async fn sidecar_post_json(
    sidecar_url: &str,
    path: &str,
    token: &str,
    payload: Value,
) -> Result<Value> {
    let url = build_url(sidecar_url, path)?;
    let mut headers = auth_headers(token)?;

    // Propagate the operator request ID to the sidecar so that sidecar logs
    // can be correlated with the originating operator API request.
    if let Ok(rid) = crate::operator_api::CURRENT_REQUEST_ID.try_with(|id| id.clone()) {
        if let Ok(val) = HeaderValue::from_str(&rid) {
            headers.insert("x-request-id", val);
        }
    }

    let (_, body) = send_json(Method::POST, url, Some(payload), headers).await?;
    serde_json::from_str(&body)
        .map_err(|err| SandboxError::Http(format!("Invalid sidecar response JSON: {err}")))
}

pub async fn sidecar_post_json_without_timeout(
    sidecar_url: &str,
    path: &str,
    token: &str,
    payload: Value,
) -> Result<Value> {
    let url = build_url(sidecar_url, path)?;
    let mut headers = auth_headers(token)?;

    if let Ok(rid) = crate::operator_api::CURRENT_REQUEST_ID.try_with(|id| id.clone()) {
        if let Ok(val) = HeaderValue::from_str(&rid) {
            headers.insert("x-request-id", val);
        }
    }

    let client = http_client_no_timeout()?;
    let (_, body) =
        send_json_with_client(client, Method::POST, url, Some(payload), headers).await?;
    serde_json::from_str(&body)
        .map_err(|err| SandboxError::Http(format!("Invalid sidecar response JSON: {err}")))
}

pub async fn sidecar_get_json(sidecar_url: &str, path: &str, token: &str) -> Result<Value> {
    let url = build_url(sidecar_url, path)?;
    let mut headers = auth_headers(token)?;

    if let Ok(rid) = crate::operator_api::CURRENT_REQUEST_ID.try_with(|id| id.clone()) {
        if let Ok(val) = HeaderValue::from_str(&rid) {
            headers.insert("x-request-id", val);
        }
    }

    let (_, body) = send_json(Method::GET, url, None, headers).await?;
    serde_json::from_str(&body)
        .map_err(|err| SandboxError::Http(format!("Invalid sidecar response JSON: {err}")))
}

/// Headers that MUST NOT be forwarded from the client to the proxied backend.
/// These are either hop-by-hop, security-sensitive (the operator's own auth),
/// or set by the proxy itself.
const STRIP_REQUEST_HEADERS: &[&str] = &[
    "host",
    "authorization", // operator PASETO — not for the backend
    "connection",
    "keep-alive",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
    "proxy-authorization",
    "proxy-connection",
    // Prevent leaking internal proxy topology to the container backend.
    "x-forwarded-for",
    "x-forwarded-proto",
    "x-forwarded-host",
    "x-real-ip",
];

/// Headers that MUST NOT be forwarded from the proxied backend to the client.
const STRIP_RESPONSE_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
];

/// Generic HTTP proxy: forward a request to a target URL and return the raw
/// response (status, headers, body). Unlike [`sidecar_post_json`], this does
/// not assume JSON and supports any HTTP method. Forwards safe request and
/// response headers.
pub async fn proxy_http(
    target_url: Url,
    method: Method,
    request_headers: &HeaderMap,
    body: Vec<u8>,
) -> Result<(StatusCode, HeaderMap, Vec<u8>)> {
    let client = http_client()?;
    let mut request = client.request(method, target_url);

    // Forward safe request headers
    for (name, value) in request_headers.iter() {
        if !STRIP_REQUEST_HEADERS
            .iter()
            .any(|&h| name.as_str().eq_ignore_ascii_case(h))
        {
            request = request.header(name, value);
        }
    }

    // Propagate request ID for tracing
    if let Ok(rid) = crate::operator_api::CURRENT_REQUEST_ID.try_with(|id| id.clone()) {
        if let Ok(val) = HeaderValue::from_str(&rid) {
            request = request.header("x-request-id", val);
        }
    }

    if !body.is_empty() {
        request = request.body(body);
    }

    let response = request.send().await.map_err(|err| {
        tracing::error!("proxy request failed: {err:?}");
        SandboxError::Http(format!("Proxy request failed: {err}"))
    })?;

    let status = response.status();
    let raw_headers = response.headers().clone();
    let resp_body = response
        .bytes()
        .await
        .map_err(|err| SandboxError::Http(format!("Failed to read proxy response: {err}")))?;

    // Filter response headers
    let mut resp_headers = HeaderMap::new();
    for (name, value) in raw_headers.iter() {
        if !STRIP_RESPONSE_HEADERS
            .iter()
            .any(|&h| name.as_str().eq_ignore_ascii_case(h))
        {
            resp_headers.append(name, value.clone());
        }
    }

    Ok((status, resp_headers, resp_body.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_url ───────────────────────────────────────────────────────

    #[test]
    fn build_url_normal() {
        let url = build_url("http://localhost:8080", "/api/test").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/api/test");
    }

    #[test]
    fn build_url_trailing_slash_on_base() {
        let url = build_url("http://localhost:8080/", "/api/test").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/api/test");
    }

    #[test]
    fn build_url_no_leading_slash_on_path() {
        let url = build_url("http://localhost:8080", "api/test").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/api/test");
    }

    #[test]
    fn build_url_empty_path() {
        let url = build_url("http://localhost:8080", "").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/");
    }

    #[test]
    fn build_url_with_port_and_nested_path() {
        let url = build_url("https://example.com:9443", "/v1/sandboxes/create").unwrap();
        assert_eq!(url.as_str(), "https://example.com:9443/v1/sandboxes/create");
    }

    #[test]
    fn build_url_invalid_base() {
        let result = build_url("not-a-url", "/api/test");
        assert!(result.is_err());
    }

    #[test]
    fn build_url_base_with_path_prefix() {
        // When the base already has a path segment, join should resolve relative to it
        let url = build_url("http://localhost:8080/prefix/", "api/test").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/prefix/api/test");
    }

    // ── auth_headers ────────────────────────────────────────────────────

    #[test]
    fn auth_headers_contains_bearer_token() {
        let headers = auth_headers("my-secret-token").unwrap();
        let auth = headers.get(AUTHORIZATION).unwrap();
        assert_eq!(auth.to_str().unwrap(), "Bearer my-secret-token");
    }

    #[test]
    fn auth_headers_contains_content_type() {
        let headers = auth_headers("token").unwrap();
        let ct = headers.get(CONTENT_TYPE).unwrap();
        assert_eq!(ct.to_str().unwrap(), "application/json");
    }

    #[test]
    fn auth_headers_with_complex_token() {
        let token = "v4.local.abcdef1234567890-complex.token";
        let headers = auth_headers(token).unwrap();
        let auth = headers.get(AUTHORIZATION).unwrap();
        assert_eq!(
            auth.to_str().unwrap(),
            "Bearer v4.local.abcdef1234567890-complex.token"
        );
    }

    #[test]
    fn auth_headers_rejects_invalid_token_chars() {
        // Header values cannot contain certain control characters
        let result = auth_headers("token\x00with\x01nulls");
        assert!(result.is_err());
    }
}
