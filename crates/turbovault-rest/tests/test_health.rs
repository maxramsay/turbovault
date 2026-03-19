mod helpers;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn test_health_returns_200() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["status"], "ok");
    assert!(json["data"]["uptime_seconds"].is_number());
    // We created 6 .md files in the temp vault (3 original + 3 notes/ for link tests)
    assert_eq!(json["data"]["note_count"], 6);
    assert_eq!(json["data"]["vault_name"], "default");
}

#[tokio::test]
async fn test_health_works_without_auth_token() {
    // Build app WITH an api_token configured
    let (app, _tmp) = helpers::test_app(Some("super-secret-token".to_string())).await;

    // Send request WITHOUT Authorization header
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Health endpoint is public — should still return 200
    assert_eq!(response.status(), StatusCode::OK);
}
