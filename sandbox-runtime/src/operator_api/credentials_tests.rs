use super::*;

#[serial_test::serial]
#[tokio::test]
async fn credentials_require_current_owner_and_are_never_cached() {
    init();
    reset_test_state();
    let owner = "0x1234567890abcdef1234567890abcdef12345678";
    let stranger = "0x0000000000000000000000000000000000000001";
    insert_plain_sandbox("credentials-owned", owner);
    insert_plain_sandbox("credentials-ownerless", "");

    for (id, identity, expected) in [
        ("credentials-owned", None, StatusCode::UNAUTHORIZED),
        ("credentials-owned", Some(stranger), StatusCode::FORBIDDEN),
        ("credentials-ownerless", Some(owner), StatusCode::FORBIDDEN),
        ("credentials-missing", Some(owner), StatusCode::NOT_FOUND),
        ("credentials-owned", Some(owner), StatusCode::OK),
    ] {
        let mut request = Request::builder().uri(format!("/api/sandboxes/{id}/credentials"));
        if let Some(address) = identity {
            request = request.header(
                "authorization",
                format!("Bearer {}", session_auth::create_test_token(address)),
            );
        }
        let response = app()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        if expected == StatusCode::OK {
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                body,
                json!({
                    "sandbox_id": id,
                    "sidecar_url": "http://localhost:9999",
                    "token": "plain-token",
                })
            );
        } else {
            let body = String::from_utf8_lossy(&bytes);
            assert!(!body.contains("plain-token"));
            assert!(!body.contains("localhost:9999"));
        }
    }

    // Model the authoritative record replacement performed on recreation.
    insert_plain_sandbox_with_url("credentials-owned", owner, "http://localhost:10000");
    let mut updated = runtime::get_sandbox_by_id("credentials-owned").unwrap();
    updated.token = "replacement-token".into();
    runtime::seal_record(&mut updated).unwrap();
    sandboxes()
        .unwrap()
        .insert(updated.id.clone(), updated)
        .unwrap();
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/credentials-owned/credentials")
                .header("authorization", test_auth_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response.into_body()).await;
    assert_eq!(body["token"], "replacement-token");
    assert_eq!(body["sidecar_url"], "http://localhost:10000");
}
