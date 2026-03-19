mod helpers;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

/// Helper: PUT a note, then DELETE it. Returns (app, delete_json, _tmp).
async fn put_and_delete(
    note_path: &str,
    content: &str,
) -> (axum::Router, serde_json::Value, tempfile::TempDir) {
    let (app, tmp) = helpers::test_app(None).await;

    // PUT a note
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/notes/{}", note_path))
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({"content": content})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // DELETE the note
    let del_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/v1/notes/{}", note_path))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(del_response.status(), StatusCode::OK);

    let body = del_response.into_body().collect().await.unwrap().to_bytes();
    let del_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    (app, del_json, tmp)
}

#[tokio::test]
async fn test_delete_moves_to_trash() {
    let (app, del_json, _tmp) = put_and_delete("trash-test.md", "# Trash Test\nContent\n").await;

    assert_eq!(del_json["success"], true);
    assert_eq!(del_json["data"]["original_path"], "trash-test.md");
    assert_eq!(del_json["data"]["restorable"], true);
    assert!(del_json["data"]["moved_to"].as_str().unwrap().starts_with("trash-test.md."));

    // GET the original path should 404
    let get_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/trash-test.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_trash_list() {
    let (app, _tmp) = helpers::test_app(None).await;

    // Delete two existing notes
    for note_path in &["test.md", "another.md"] {
        let del_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/notes/{}", note_path))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_response.status(), StatusCode::OK);
    }

    // GET /v1/trash
    let list_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/trash")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(list_response.status(), StatusCode::OK);

    let body = list_response.into_body().collect().await.unwrap().to_bytes();
    let list_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(list_json["success"], true);
    assert_eq!(list_json["count"], 2);

    let entries = list_json["data"].as_array().unwrap();
    assert_eq!(entries.len(), 2);

    let paths: Vec<&str> = entries
        .iter()
        .map(|e| e["original_path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"test.md"));
    assert!(paths.contains(&"another.md"));
}

#[tokio::test]
async fn test_restore_from_trash() {
    let (app, del_json, _tmp) =
        put_and_delete("restore-test.md", "# Restore Me\nOriginal content\n").await;

    let trash_path = del_json["data"]["moved_to"].as_str().unwrap();

    // POST /v1/restore/{trash-path}
    let restore_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/restore/{}", trash_path))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(restore_response.status(), StatusCode::OK);

    let body = restore_response.into_body().collect().await.unwrap().to_bytes();
    let restore_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(restore_json["success"], true);
    assert_eq!(restore_json["data"]["restored_to"], "restore-test.md");

    // GET the original path should return 200 with original content
    let get_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes/restore-test.md")
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
        .contains("Original content"));
}

#[tokio::test]
async fn test_request_purge_returns_202() {
    let (app, del_json, tmp) =
        put_and_delete("purge-test.md", "# Purge Me\nContent\n").await;

    let trash_path = del_json["data"]["moved_to"].as_str().unwrap();

    // POST /v1/request-purge/{trash-path}
    let purge_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/request-purge/{}", trash_path))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(purge_response.status(), StatusCode::ACCEPTED);

    let body = purge_response.into_body().collect().await.unwrap().to_bytes();
    let purge_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(purge_json["success"], true);
    assert_eq!(purge_json["data"]["status"], "pending");
    assert!(purge_json["data"]["message"]
        .as_str()
        .unwrap()
        .contains("curator"));
    assert!(purge_json["data"]["requested_at"].as_str().is_some());

    // File should still exist in .trash/ (not actually deleted)
    let trash_file = tmp.path().join(".trash").join(trash_path);
    assert!(trash_file.exists(), "File should still exist in .trash/ after purge request");
}

#[tokio::test]
async fn test_restore_nonexistent_returns_404() {
    let (app, _tmp) = helpers::test_app(None).await;

    let restore_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/restore/nonexistent.md.9999999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(restore_response.status(), StatusCode::NOT_FOUND);

    let body = restore_response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["success"], false);
    assert_eq!(json["error"]["code"], "NOT_FOUND");
}
