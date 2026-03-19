mod helpers;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// If-Match tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_put_with_matching_if_match_succeeds() {
    let (app, _tmp) = helpers::test_app(None).await;

    // GET the existing note to obtain its current hash
    let get_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/notes/test.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK);

    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let hash = get_json["data"]["hash"].as_str().unwrap().to_string();

    // PUT with the correct If-Match hash — should succeed
    let put_response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/notes/test.md")
                .header("Content-Type", "application/json")
                .header("If-Match", &hash)
                .body(Body::from(
                    serde_json::to_string(&json!({"content": "# Updated\nNew content\n"})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(put_response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_put_with_stale_if_match_fails() {
    let (app, _tmp) = helpers::test_app(None).await;

    // GET note to obtain its current hash
    let get_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/notes/test.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let old_hash = get_json["data"]["hash"].as_str().unwrap().to_string();

    // PUT without If-Match to change the content (and therefore the hash)
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/notes/test.md")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({"content": "# Changed\nDifferent content\n"}))
                        .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // PUT again with the OLD hash — should return 409 HASH_MISMATCH
    let stale_response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/notes/test.md")
                .header("Content-Type", "application/json")
                .header("If-Match", &old_hash)
                .body(Body::from(
                    serde_json::to_string(&json!({"content": "# Conflict\nShould fail\n"}))
                        .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(stale_response.status(), StatusCode::CONFLICT);

    let body = stale_response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "HASH_MISMATCH");
}

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

#[tokio::test]
async fn test_put_creates_new_note() {
    let (app, _tmp) = helpers::test_app(None).await;

    let put_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/notes/Staging/new-note.md")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({"content": "# New Note\nCreated via PUT\n"})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(put_response.status(), StatusCode::OK);

    let body = put_response.into_body().collect().await.unwrap().to_bytes();
    let put_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(put_json["success"], true);
    assert_eq!(put_json["data"]["path"], "Staging/new-note.md");
    assert_eq!(put_json["data"]["status"], "created");
    assert!(put_json["data"]["hash"].as_str().is_some());

    // GET the note back and verify content
    let get_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/Staging/new-note.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK);

    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(get_json["data"]["content"]
        .as_str()
        .unwrap()
        .contains("# New Note"));
}

#[tokio::test]
async fn test_put_overwrites_existing_note() {
    let (app, _tmp) = helpers::test_app(None).await;

    let put_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/notes/test.md")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({"content": "# Overwritten\nNew content only\n"})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(put_response.status(), StatusCode::OK);

    let body = put_response.into_body().collect().await.unwrap().to_bytes();
    let put_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(put_json["success"], true);
    assert_eq!(put_json["data"]["status"], "overwritten");

    // GET back and verify old content is gone, new content present
    let get_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/test.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let content = get_json["data"]["content"].as_str().unwrap();

    assert!(content.contains("Overwritten"), "new content should be present");
    assert!(!content.contains("Hello world"), "old content should be gone");
}

#[tokio::test]
async fn test_post_appends_to_existing_note() {
    let (app, _tmp) = helpers::test_app(None).await;

    let post_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/notes/test.md")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({"content": "\n\nAppended text"})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(post_response.status(), StatusCode::OK);

    let body = post_response.into_body().collect().await.unwrap().to_bytes();
    let post_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(post_json["success"], true);
    assert_eq!(post_json["data"]["status"], "appended");

    // GET and confirm both original and appended content are present
    let get_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/test.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let content = get_json["data"]["content"].as_str().unwrap();

    assert!(content.contains("Hello world"), "original content should still be present");
    assert!(content.contains("Appended text"), "appended content should be present");
}

#[tokio::test]
async fn test_post_to_nonexistent_returns_404() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/notes/does-not-exist.md")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({"content": "some text"})).unwrap(),
                ))
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
async fn test_put_with_text_markdown_content_type() {
    let (app, _tmp) = helpers::test_app(None).await;

    let put_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/notes/markdown-test.md")
                .header("Content-Type", "text/markdown")
                .body(Body::from("# Markdown PUT\nRaw markdown body\n"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(put_response.status(), StatusCode::OK);

    let body = put_response.into_body().collect().await.unwrap().to_bytes();
    let put_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(put_json["success"], true);
    assert_eq!(put_json["data"]["status"], "created");

    // GET back and verify
    let get_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/markdown-test.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let content = get_json["data"]["content"].as_str().unwrap();

    assert!(content.contains("Raw markdown body"));
}
