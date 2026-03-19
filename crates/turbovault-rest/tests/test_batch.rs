mod helpers;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn test_batch_read_multiple_notes() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/batch/read")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "paths": ["test.md", "another.md", "Daily/2026-03-19.md"]
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["count"], 3);

    let results = json["data"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);

    for result in results {
        assert!(result["content"].as_str().is_some(), "each result should have content");
        assert!(result["hash"].as_str().is_some(), "each result should have a hash");
        assert!(result["error"].is_null(), "no errors expected for existing files");
    }
}

#[tokio::test]
async fn test_batch_read_partial_failure() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/batch/read")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "paths": ["test.md", "does-not-exist.md"]
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Partial success still returns 200
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], true);
    // count reflects only successful reads
    assert_eq!(json["count"], 1);

    let results = json["data"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);

    let success = results.iter().find(|r| r["path"] == "test.md").unwrap();
    assert!(success["content"].as_str().is_some());
    assert!(success["hash"].as_str().is_some());
    assert!(success["error"].is_null());

    let failure = results
        .iter()
        .find(|r| r["path"] == "does-not-exist.md")
        .unwrap();
    assert_eq!(failure["error"], "NOT_FOUND");
    assert!(failure["content"].is_null());
}

#[tokio::test]
async fn test_batch_read_exceeds_limit() {
    let (app, _tmp) = helpers::test_app(None).await;

    let paths: Vec<String> = (0..51).map(|i| format!("note{}.md", i)).collect();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/batch/read")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({ "paths": paths })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], false);
    assert_eq!(json["error"]["code"], "INVALID_REQUEST");
}

#[tokio::test]
async fn test_batch_read_empty_paths() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/batch/read")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({ "paths": [] })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], false);
    assert_eq!(json["error"]["code"], "INVALID_REQUEST");
}
