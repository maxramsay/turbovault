mod helpers;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;
use turbovault_rest::RestConfig;

// ---------------------------------------------------------------------------
// Protected paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_write_to_protected_path_returns_403() {
    let config = RestConfig {
        api_token: None,
        protected_paths: vec!["Focus Areas/Writing/".to_string()],
    };
    let (app, _tmp) = helpers::test_app_with_config(config).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/notes/Focus%20Areas/Writing/story.md")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({"content": "# Story\nShould be blocked\n"}))
                        .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json_body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json_body["error"]["code"], "FORBIDDEN");
}

#[tokio::test]
async fn test_read_from_protected_path_allowed() {
    let config = RestConfig {
        api_token: None,
        protected_paths: vec!["Focus Areas/Writing/".to_string()],
    };
    let (app, tmp) = helpers::test_app_with_config(config).await;

    // Create the file directly on the filesystem (bypassing the API)
    let writing_dir = tmp.path().join("Focus Areas/Writing");
    std::fs::create_dir_all(&writing_dir).unwrap();
    std::fs::write(writing_dir.join("story.md"), "# My Story\nProtected content\n").unwrap();

    // GET is a read — must be allowed even though the path is protected for writes
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/Focus%20Areas/Writing/story.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json_body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json_body["success"], true);
    assert!(json_body["data"]["content"]
        .as_str()
        .unwrap()
        .contains("Protected content"));
}

// ---------------------------------------------------------------------------
// Path traversal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_path_traversal_rejected() {
    let (app, _tmp) = helpers::test_app(None).await;

    // Attempt directory traversal: /v1/notes/../../etc/passwd
    // Axum will URL-decode the path — "../../etc/passwd" should be rejected
    // by path validation (INVALID_PATH) or the OS refusing to resolve it.
    // The important thing is it does NOT return 200 with the file contents.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/..%2F..%2Fetc%2Fpasswd")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Acceptable outcomes: 400 (INVALID_PATH) or 404 (file not found outside vault).
    // What is NOT acceptable: 200 with /etc/passwd content.
    assert_ne!(
        response.status(),
        StatusCode::OK,
        "Path traversal must not succeed"
    );
}
