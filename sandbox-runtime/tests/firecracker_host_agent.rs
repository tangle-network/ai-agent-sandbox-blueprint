use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex as AsyncMutex;

use sandbox_runtime::runtime::{
    CreateSandboxParams, SandboxState, create_sidecar, delete_sidecar, get_sandbox_by_id,
    resume_sidecar, stop_sidecar,
};

const API_KEY: &str = "firecracker-test-key";

#[derive(Clone, Debug)]
struct ContainerMock {
    id: String,
    endpoint: String,
    state: MockStateValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MockStateValue {
    Created,
    Running,
    Stopped,
}

#[derive(Clone, Debug)]
struct MockHostState {
    sidecar_endpoint: String,
    containers: HashMap<String, ContainerMock>,
}

#[derive(Deserialize)]
struct CreateContainerRequest {
    #[serde(rename = "sessionId")]
    session_id: String,
}

fn ensure_api_key(headers: &HeaderMap) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if key == API_KEY {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"missing or invalid api key","code":"UNAUTHORIZED"})),
        ))
    }
}

fn container_json(container: &ContainerMock) -> serde_json::Value {
    let (status, state) = match container.state {
        MockStateValue::Created => ("created", "terminated"),
        MockStateValue::Running => ("running", "running"),
        MockStateValue::Stopped => ("stopped", "terminated"),
    };
    let endpoint = if matches!(container.state, MockStateValue::Running) {
        container.endpoint.clone()
    } else {
        String::new()
    };
    json!({
        "id": container.id,
        "name": container.id,
        "sessionId": container.id,
        "image": "test-image",
        "status": status,
        "state": state,
        "endpoint": endpoint,
        "createdAt": 0,
        "labels": {},
        "resources": { "cpu": 1, "memory": 512, "disk": 1024, "pids": 128 }
    })
}

async fn create_container(
    State(state): State<Arc<AsyncMutex<MockHostState>>>,
    headers: HeaderMap,
    Json(body): Json<CreateContainerRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    ensure_api_key(&headers)?;
    let mut guard = state.lock().await;
    if guard.containers.contains_key(&body.session_id) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"already exists","code":"ALREADY_EXISTS"})),
        ));
    }

    let container = ContainerMock {
        id: body.session_id.clone(),
        endpoint: guard.sidecar_endpoint.clone(),
        state: MockStateValue::Created,
    };
    let response = container_json(&container);
    guard.containers.insert(body.session_id, container);
    Ok((StatusCode::CREATED, Json(response)))
}

async fn start_container(
    State(state): State<Arc<AsyncMutex<MockHostState>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    ensure_api_key(&headers)?;
    let mut guard = state.lock().await;
    let Some(container) = guard.containers.get_mut(&id) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error":"not found","code":"NOT_FOUND"})),
        ));
    };
    container.state = MockStateValue::Running;
    Ok((StatusCode::OK, Json(container_json(container))))
}

async fn stop_container(
    State(state): State<Arc<AsyncMutex<MockHostState>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    ensure_api_key(&headers)?;
    let mut guard = state.lock().await;
    let Some(container) = guard.containers.get_mut(&id) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error":"not found","code":"NOT_FOUND"})),
        ));
    };
    container.state = MockStateValue::Stopped;
    Ok((StatusCode::OK, Json(json!({ "ok": true }))))
}

async fn get_container(
    State(state): State<Arc<AsyncMutex<MockHostState>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    ensure_api_key(&headers)?;
    let guard = state.lock().await;
    let Some(container) = guard.containers.get(&id) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error":"not found","code":"NOT_FOUND"})),
        ));
    };
    Ok((StatusCode::OK, Json(container_json(container))))
}

async fn delete_container(
    State(state): State<Arc<AsyncMutex<MockHostState>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    ensure_api_key(&headers)?;
    let mut guard = state.lock().await;
    if guard.containers.remove(&id).is_some() {
        Ok((StatusCode::OK, Json(json!({ "ok": true }))))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error":"not found","code":"NOT_FOUND"})),
        ))
    }
}

async fn sidecar_health() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::OK, Json(json!({ "ok": true })))
}

#[tokio::test]
async fn firecracker_backend_lifecycle_flows_through_host_agent() {
    let sidecar_app = Router::new().route("/health", get(sidecar_health));
    let sidecar_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind sidecar listener");
    let sidecar_addr = sidecar_listener
        .local_addr()
        .expect("sidecar listener local addr");
    tokio::spawn(async move {
        axum::serve(sidecar_listener, sidecar_app)
            .await
            .expect("sidecar server should run");
    });

    let sidecar_endpoint = format!("http://{}:{}", sidecar_addr.ip(), sidecar_addr.port());
    let state = Arc::new(AsyncMutex::new(MockHostState {
        sidecar_endpoint: sidecar_endpoint.clone(),
        containers: HashMap::new(),
    }));
    let app = Router::new()
        .route("/v1/containers", post(create_container))
        .route(
            "/v1/containers/{id}",
            get(get_container).delete(delete_container),
        )
        .route("/v1/containers/{id}/start", post(start_container))
        .route("/v1/containers/{id}/stop", post(stop_container))
        .with_state(state.clone());

    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind host-agent listener");
    let host_addr = host_listener
        .local_addr()
        .expect("host-agent listener local addr");
    tokio::spawn(async move {
        axum::serve(host_listener, app)
            .await
            .expect("host-agent server should run");
    });

    let state_dir = tempfile::tempdir().expect("temp state dir");
    let host_url = format!("http://{}:{}", host_addr.ip(), host_addr.port());
    unsafe {
        std::env::set_var("BLUEPRINT_STATE_DIR", state_dir.path());
        std::env::set_var(
            "SESSION_AUTH_SECRET",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        );
        std::env::set_var("FIRECRACKER_HOST_AGENT_URL", host_url);
        std::env::set_var("FIRECRACKER_HOST_AGENT_API_KEY", API_KEY);
    }

    let params = CreateSandboxParams {
        name: "firecracker-test".to_string(),
        image: "ghcr.io/tangle-network/sidecar:latest".to_string(),
        metadata_json: r#"{"runtime_backend":"firecracker"}"#.to_string(),
        owner: "0xabc123".to_string(),
        cpu_cores: 1,
        memory_mb: 512,
        disk_gb: 10,
        ..Default::default()
    };

    let (record, attestation) = create_sidecar(&params, None)
        .await
        .expect("create firecracker sandbox");
    assert!(
        attestation.is_none(),
        "firecracker path should not emit TEE attestation"
    );
    assert_eq!(record.state, SandboxState::Running);
    assert_eq!(record.sidecar_url, sidecar_endpoint);
    assert!(
        record
            .metadata_json
            .contains("\"runtime_backend\":\"firecracker\""),
        "record metadata should persist backend marker"
    );

    stop_sidecar(&record)
        .await
        .expect("stop firecracker sandbox");
    let stopped = get_sandbox_by_id(&record.id).expect("load stopped sandbox");
    assert_eq!(stopped.state, SandboxState::Stopped);

    resume_sidecar(&stopped)
        .await
        .expect("resume firecracker sandbox");
    let resumed = get_sandbox_by_id(&record.id).expect("load resumed sandbox");
    assert_eq!(resumed.state, SandboxState::Running);
    assert_eq!(resumed.sidecar_url, sidecar_endpoint);

    delete_sidecar(&resumed, None)
        .await
        .expect("delete firecracker sandbox");
    let guard_state = state.lock().await;
    assert!(
        !guard_state.containers.contains_key(&record.container_id),
        "host-agent should no longer contain the VM after delete"
    );

    drop(guard_state);
}
