mod helpers;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

/// End-to-end lifecycle test: create → read → append → patch → info → delete → trash → restore → verify.
#[tokio::test]
async fn test_full_lifecycle() {
    let (app, _tmp) = helpers::test_app(None).await;
    let note_uri = "/v1/notes/Staging/lifecycle-test.md";
    let note_info_uri = "/v1/notes-info/Staging/lifecycle-test.md";

    // -----------------------------------------------------------------------
    // Step 1: PUT — create the note
    // -----------------------------------------------------------------------
    let put_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(note_uri)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "content": "# Lifecycle Test\n\n## Section A\n\nOriginal content"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(put_response.status(), StatusCode::OK, "Step 1: PUT should return 200");
    let body = put_response.into_body().collect().await.unwrap().to_bytes();
    let put_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(put_json["success"], true, "Step 1: success should be true");
    assert_eq!(put_json["data"]["status"], "created", "Step 1: status should be 'created'");
    assert_eq!(
        put_json["data"]["path"], "Staging/lifecycle-test.md",
        "Step 1: path should match"
    );

    // -----------------------------------------------------------------------
    // Step 2: GET — read it back, save hash
    // -----------------------------------------------------------------------
    let get_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(note_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK, "Step 2: GET should return 200");
    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        get_json["data"]["content"]
            .as_str()
            .unwrap()
            .contains("Original content"),
        "Step 2: content should contain 'Original content'"
    );
    let _hash_after_create = get_json["data"]["hash"].as_str().unwrap().to_string();

    // -----------------------------------------------------------------------
    // Step 3: POST — append new section
    // -----------------------------------------------------------------------
    let post_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(note_uri)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "content": "\n\n## Section B\n\nAppended"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(post_response.status(), StatusCode::OK, "Step 3: POST should return 200");
    let body = post_response.into_body().collect().await.unwrap().to_bytes();
    let post_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(post_json["success"], true, "Step 3: success should be true");
    assert_eq!(post_json["data"]["status"], "appended", "Step 3: status should be 'appended'");

    // -----------------------------------------------------------------------
    // Step 4: GET — verify append
    // -----------------------------------------------------------------------
    let get_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(note_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK, "Step 4: GET should return 200");
    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let content = get_json["data"]["content"].as_str().unwrap();
    assert!(
        content.contains("Original content"),
        "Step 4: 'Original content' should still be present"
    );
    assert!(
        content.contains("Appended"),
        "Step 4: 'Appended' section should be present"
    );

    // -----------------------------------------------------------------------
    // Step 5: PATCH — insert under heading "Section A"
    // -----------------------------------------------------------------------
    let patch_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(note_uri)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "target_type": "heading",
                        "target": "Section A",
                        "operation": "append",
                        "content": "Patched text"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(patch_response.status(), StatusCode::OK, "Step 5: PATCH should return 200");
    let body = patch_response.into_body().collect().await.unwrap().to_bytes();
    let patch_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(patch_json["success"], true, "Step 5: success should be true");
    assert_eq!(patch_json["data"]["status"], "patched", "Step 5: status should be 'patched'");

    // -----------------------------------------------------------------------
    // Step 6: GET — verify patch
    // -----------------------------------------------------------------------
    let get_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(note_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK, "Step 6: GET should return 200");
    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let content = get_json["data"]["content"].as_str().unwrap();

    let pos_original = content
        .find("Original content")
        .expect("Step 6: 'Original content' should be present");
    let pos_patched = content
        .find("Patched text")
        .expect("Step 6: 'Patched text' should be present");
    let pos_section_b = content
        .find("## Section B")
        .expect("Step 6: '## Section B' should be present");

    assert!(
        pos_original < pos_patched,
        "Step 6: 'Patched text' should appear after 'Original content'"
    );
    assert!(
        pos_patched < pos_section_b,
        "Step 6: 'Patched text' should appear before '## Section B'"
    );

    // -----------------------------------------------------------------------
    // Step 7: GET info — metadata check
    // -----------------------------------------------------------------------
    let info_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(note_info_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(info_response.status(), StatusCode::OK, "Step 7: GET info should return 200");
    let body = info_response.into_body().collect().await.unwrap().to_bytes();
    let info_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(info_json["success"], true, "Step 7: success should be true");
    assert!(
        info_json["data"]["size_bytes"].as_u64().unwrap_or(0) > 0,
        "Step 7: size_bytes should be > 0"
    );
    assert!(
        info_json["data"]["content"].is_null(),
        "Step 7: content field should not be present in notes-info"
    );

    // -----------------------------------------------------------------------
    // Step 8: DELETE — soft delete
    // -----------------------------------------------------------------------
    let del_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(note_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(del_response.status(), StatusCode::OK, "Step 8: DELETE should return 200");
    let body = del_response.into_body().collect().await.unwrap().to_bytes();
    let del_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(del_json["success"], true, "Step 8: success should be true");
    assert_eq!(
        del_json["data"]["original_path"], "Staging/lifecycle-test.md",
        "Step 8: original_path should match"
    );
    assert_eq!(del_json["data"]["restorable"], true, "Step 8: restorable should be true");
    let moved_to = del_json["data"]["moved_to"]
        .as_str()
        .expect("Step 8: moved_to should be present")
        .to_string();
    assert!(
        moved_to.starts_with("Staging/lifecycle-test.md."),
        "Step 8: moved_to should start with 'Staging/lifecycle-test.md.' got: {}",
        moved_to
    );

    // -----------------------------------------------------------------------
    // Step 9: GET — should be 404 now
    // -----------------------------------------------------------------------
    let get_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(note_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::NOT_FOUND, "Step 9: deleted note should 404");
    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(get_json["error"]["code"], "NOT_FOUND", "Step 9: error code should be NOT_FOUND");

    // -----------------------------------------------------------------------
    // Step 10: GET trash — verify note is in trash
    // -----------------------------------------------------------------------
    let trash_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/trash")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(trash_response.status(), StatusCode::OK, "Step 10: GET /v1/trash should return 200");
    let body = trash_response.into_body().collect().await.unwrap().to_bytes();
    let trash_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(trash_json["success"], true, "Step 10: success should be true");

    let entries = trash_json["data"].as_array().expect("Step 10: data should be an array");
    let our_entry = entries
        .iter()
        .find(|e| e["original_path"].as_str() == Some("Staging/lifecycle-test.md"))
        .expect("Step 10: our note should appear in trash");
    assert_eq!(
        our_entry["trash_path"].as_str().unwrap(),
        moved_to,
        "Step 10: trash_path should match moved_to from DELETE"
    );

    // -----------------------------------------------------------------------
    // Step 11: POST restore — bring it back
    // -----------------------------------------------------------------------
    let restore_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/restore/{}", moved_to))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(restore_response.status(), StatusCode::OK, "Step 11: POST restore should return 200");
    let body = restore_response.into_body().collect().await.unwrap().to_bytes();
    let restore_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(restore_json["success"], true, "Step 11: success should be true");
    assert_eq!(
        restore_json["data"]["restored_to"], "Staging/lifecycle-test.md",
        "Step 11: restored_to should match original path"
    );

    // -----------------------------------------------------------------------
    // Step 12: GET — note is back, content includes all modifications
    // -----------------------------------------------------------------------
    let get_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(note_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK, "Step 12: restored note should return 200");
    let body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let final_content = get_json["data"]["content"].as_str().unwrap();

    assert!(
        final_content.contains("Original content"),
        "Step 12: restored content should contain 'Original content'"
    );
    assert!(
        final_content.contains("Appended"),
        "Step 12: restored content should contain appended section"
    );
    assert!(
        final_content.contains("Patched text"),
        "Step 12: restored content should contain patched text"
    );
}
