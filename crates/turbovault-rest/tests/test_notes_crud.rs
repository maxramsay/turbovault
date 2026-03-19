mod helpers;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn test_read_existing_note() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/test.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["path"], "test.md");
    assert!(json["data"]["content"].as_str().unwrap().contains("Hello world"));
    assert!(json["data"]["hash"].as_str().is_some());
    assert!(!json["data"]["hash"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn test_read_nonexistent_note() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/does-not-exist.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], false);
    assert_eq!(json["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn test_read_note_returns_etag_header() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/test.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let etag = response.headers().get("ETag").cloned();
    assert!(etag.is_some(), "ETag header should be present");

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let hash_in_body = json["data"]["hash"].as_str().unwrap();
    let etag_value = etag.unwrap();
    let etag_str = etag_value.to_str().unwrap();

    assert_eq!(etag_str, hash_in_body, "ETag header should match hash in body");
}

#[tokio::test]
async fn test_notes_info() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes-info/test.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["path"], "test.md");
    assert!(json["data"]["size_bytes"].as_u64().is_some());
    assert!(json["data"]["size_bytes"].as_u64().unwrap() > 0);
    assert!(json["data"]["modified_at"].as_str().is_some());
    assert!(json["data"]["has_frontmatter"].is_boolean());
    // Should NOT have content field
    assert!(json["data"]["content"].is_null());
}

#[tokio::test]
async fn test_notes_info_nonexistent() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes-info/nope.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], false);
    assert_eq!(json["error"]["code"], "NOT_FOUND");
}
