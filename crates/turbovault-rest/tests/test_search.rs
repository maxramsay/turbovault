mod helpers;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn test_search_finds_note_by_content() {
    let (app, _tmp) = helpers::test_app(None).await;

    // "content" appears in Daily/2026-03-19.md and another.md
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/search?q=content")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], true);
    let results = json["data"].as_array().unwrap();
    assert!(!results.is_empty(), "expected at least one search result");

    // Each result must have the required fields
    for r in results {
        assert!(r["path"].as_str().is_some(), "result missing 'path'");
        assert!(r["title"].as_str().is_some(), "result missing 'title'");
        assert!(r["score"].as_f64().is_some(), "result missing 'score'");
        assert!(r["snippet"].as_str().is_some(), "result missing 'snippet'");
    }

    // At least one result should be one of the files that contain "content"
    let paths: Vec<&str> = results
        .iter()
        .filter_map(|r| r["path"].as_str())
        .collect();
    let found = paths
        .iter()
        .any(|p| p.contains("another") || p.contains("Daily"));
    assert!(found, "expected result with 'another.md' or Daily note, got: {:?}", paths);
}

#[tokio::test]
async fn test_search_returns_empty_for_no_match() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/search?q=xyznonexistent123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], true);
    let results = json["data"].as_array().unwrap();
    assert!(results.is_empty(), "expected no results, got: {:?}", results);
    assert_eq!(json["count"], 0);
}

#[tokio::test]
async fn test_search_requires_query() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/search")
                .body(Body::empty())
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
