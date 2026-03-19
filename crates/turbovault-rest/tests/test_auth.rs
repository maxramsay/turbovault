mod helpers;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use turbovault_rest::RestConfig;

#[tokio::test]
async fn test_auth_required_when_token_set() {
    let config = RestConfig {
        api_token: Some("test-secret-token".to_string()),
        protected_paths: vec![],
    };
    let (app, _tmp) = helpers::test_app_with_config(config).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/test.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_wrong_token_rejected() {
    let config = RestConfig {
        api_token: Some("correct-token".to_string()),
        protected_paths: vec![],
    };
    let (app, _tmp) = helpers::test_app_with_config(config).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/test.md")
                .header("Authorization", "Bearer wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_correct_token_accepted() {
    let config = RestConfig {
        api_token: Some("correct-token".to_string()),
        protected_paths: vec![],
    };
    let (app, _tmp) = helpers::test_app_with_config(config).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/test.md")
                .header("Authorization", "Bearer correct-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_health_bypasses_auth() {
    let config = RestConfig {
        api_token: Some("test-secret-token".to_string()),
        protected_paths: vec![],
    };
    let (app, _tmp) = helpers::test_app_with_config(config).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Health endpoint is public — must bypass auth
    assert_eq!(response.status(), StatusCode::OK);
}
