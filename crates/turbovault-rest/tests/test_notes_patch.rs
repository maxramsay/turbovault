mod helpers;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn test_patch_append_under_heading() {
    let (app, _tmp) = helpers::test_app(None).await;

    // PUT a note with two headings
    let note_body = "# Title\n\n## Section A\n\nOriginal content\n\n## Section B\n\nMore content\n";
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/notes/patch-test.md")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({"content": note_body})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // PATCH: append under "Section A"
    let patch_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/notes/patch-test.md")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "target_type": "heading",
                        "target": "Section A",
                        "operation": "append",
                        "content": "New text"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(patch_response.status(), StatusCode::OK);

    let body = patch_response.into_body().collect().await.unwrap().to_bytes();
    let patch_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(patch_json["success"], true);
    assert_eq!(patch_json["data"]["status"], "patched");
    assert_eq!(patch_json["data"]["target_type"], "heading");
    assert_eq!(patch_json["data"]["target"], "Section A");
    assert_eq!(patch_json["data"]["operation"], "append");

    // GET and verify structure
    let get_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/patch-test.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let content = get_json["data"]["content"].as_str().unwrap();

    // "New text" should appear after "Original content" but before "## Section B"
    let pos_original = content.find("Original content").expect("Original content should be present");
    let pos_new = content.find("New text").expect("New text should be present");
    let pos_section_b = content.find("## Section B").expect("Section B should be present");

    assert!(
        pos_original < pos_new,
        "New text should appear after Original content"
    );
    assert!(
        pos_new < pos_section_b,
        "New text should appear before ## Section B"
    );
}

#[tokio::test]
async fn test_patch_with_text_markdown_content_type() {
    let (app, _tmp) = helpers::test_app(None).await;

    // PUT a note
    let note_body = "# Title\n\n## Section A\n\nOriginal content\n\n## Section B\n\nMore content\n";
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/notes/patch-markdown-test.md")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({"content": note_body})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // PATCH using text/markdown with query params
    let patch_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/notes/patch-markdown-test.md?target_type=heading&target=Section+A&operation=append")
                .header("Content-Type", "text/markdown")
                .body(Body::from("Markdown appended text"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(patch_response.status(), StatusCode::OK);

    let body = patch_response.into_body().collect().await.unwrap().to_bytes();
    let patch_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(patch_json["success"], true);
    assert_eq!(patch_json["data"]["status"], "patched");

    // GET and verify
    let get_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/patch-markdown-test.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let content = get_json["data"]["content"].as_str().unwrap();

    assert!(
        content.contains("Markdown appended text"),
        "Appended text should appear in the note"
    );
    assert!(
        content.contains("Original content"),
        "Original content should still be present"
    );
}

#[tokio::test]
async fn test_patch_nonexistent_note_returns_404() {
    let (app, _tmp) = helpers::test_app(None).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/notes/nope.md")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "target_type": "heading",
                        "target": "Some Heading",
                        "operation": "append",
                        "content": "Some content"
                    }))
                    .unwrap(),
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
