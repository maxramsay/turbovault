mod helpers;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn test_list_files_root() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/files")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["operation"], "list_files");

    let data = json["data"].as_array().unwrap();
    assert!(!data.is_empty(), "root listing should not be empty");

    // Should contain test.md and another.md files, and Daily directory
    let names: Vec<&str> = data
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Daily"), "should contain Daily directory");
    assert!(names.contains(&"test.md"), "should contain test.md");
    assert!(names.contains(&"another.md"), "should contain another.md");

    // Directories should appear before files (Daily before test.md)
    let daily_pos = names.iter().position(|n| *n == "Daily").unwrap();
    let test_pos = names.iter().position(|n| *n == "test.md").unwrap();
    assert!(daily_pos < test_pos, "directories should appear before files");

    // Check entry has expected fields
    let daily_entry = data.iter().find(|e| e["name"] == "Daily").unwrap();
    assert_eq!(daily_entry["type"], "directory");
    assert!(daily_entry["size_bytes"].is_null(), "directories should not have size_bytes");

    let test_entry = data.iter().find(|e| e["name"] == "test.md").unwrap();
    assert_eq!(test_entry["type"], "file");
    assert!(test_entry["size_bytes"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_list_files_subdir() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/files/Daily")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], true);

    let data = json["data"].as_array().unwrap();
    assert!(!data.is_empty(), "Daily/ listing should contain notes");

    let names: Vec<&str> = data.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(
        names.contains(&"2026-03-19.md"),
        "should contain the daily note created by test helper"
    );
}

#[tokio::test]
async fn test_list_files_nonexistent_dir() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/files/nonexistent-dir")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["success"], false);
}

#[tokio::test]
async fn test_list_files_excludes_hidden() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/files")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let data = json["data"].as_array().unwrap();

    let names: Vec<&str> = data.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(
        !names.iter().any(|n| n.starts_with('.')),
        "hidden files/dirs should be excluded from listing"
    );
}

#[tokio::test]
async fn test_periodic_daily_exists() {
    let (app, _tmp) = helpers::test_app(None).await;

    // The test helper creates Daily/2026-03-19.md
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/periodic/daily?date=2026-03-19")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["operation"], "get_periodic_note");
    assert_eq!(json["data"]["exists"], true);
    assert_eq!(json["data"]["period"], "daily");
    assert_eq!(json["data"]["date"], "2026-03-19");
    assert_eq!(json["data"]["path"], "Daily/2026-03-19.md");
    assert!(
        json["data"]["content"].as_str().unwrap().contains("Daily Note"),
        "should return note content"
    );
}

#[tokio::test]
async fn test_periodic_daily_not_found() {
    let (app, _tmp) = helpers::test_app(None).await;

    // A date with no note
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/periodic/daily?date=2000-01-01")
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
    // Error message should include the expected path
    let msg = json["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("Daily/2000-01-01.md"),
        "error message should include expected path, got: {}",
        msg
    );
}

#[tokio::test]
async fn test_periodic_invalid_period() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/periodic/hourly")
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

#[tokio::test]
async fn test_recent_changes() {
    let (app, _tmp) = helpers::test_app(None).await;

    // Use a large window so recently-created test files are included
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/recent?days=30")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["operation"], "recent_changes");

    let data = json["data"].as_array().unwrap();
    assert!(!data.is_empty(), "recent changes should include test files");

    // Verify each entry has required fields
    for entry in data {
        assert!(entry["path"].as_str().is_some(), "entry should have path");
        assert!(
            entry["modified_at"].as_u64().is_some(),
            "entry should have modified_at"
        );
        assert!(
            entry["size_bytes"].as_u64().is_some(),
            "entry should have size_bytes"
        );
        // Only .md files
        assert!(
            entry["path"].as_str().unwrap().ends_with(".md"),
            "only .md files should appear in recent changes"
        );
    }

    // Should be sorted newest first
    if data.len() > 1 {
        let times: Vec<u64> = data.iter().map(|e| e["modified_at"].as_u64().unwrap()).collect();
        let is_sorted = times.windows(2).all(|w| w[0] >= w[1]);
        assert!(is_sorted, "results should be sorted newest first");
    }
}

#[tokio::test]
async fn test_recent_changes_zero_days() {
    let (app, _tmp) = helpers::test_app(None).await;

    // days=0 means only files modified in the last 0 days (essentially now)
    // This should return few or zero results
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/recent?days=0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["success"], true);
    // Just check the structure is correct
    assert!(json["data"].is_array());
}
