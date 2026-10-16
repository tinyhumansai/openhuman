use std::sync::Arc;

use serde_json::json;
use tiny_artifacts::{
    ArtifactKind, ArtifactPublisher, ArtifactRecord, ArtifactVersion, Artifacts, Author, Content,
    FileRef, PublishOptions,
};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use openhuman_embed::__host::artifact_publisher::versioned_store;
use openhuman_embed::__host::security::AutonomyLevel;

fn publisher(server_uri: String, autonomy: AutonomyLevel) -> StorageArtifactPublisher {
    StorageArtifactPublisher::new(
        Arc::new(IntegrationClient::new(server_uri, "tok".to_string())),
        Arc::new(SecurityPolicy {
            autonomy,
            ..SecurityPolicy::default()
        }),
    )
}

fn uploaded(public_url: Option<&str>) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "success": true,
        "data": {
            "fileId": "file-1",
            "filename": "x",
            "size": 4,
            "contentType": "text/markdown",
            "visibility": if public_url.is_some() { "public" } else { "private" },
            "expiresAt": "2030-01-01T00:00:00.000Z",
            "publicUrl": public_url,
            "costUsd": 0.0001
        }
    }))
}

async fn mount_link(server: &MockServer, expect: u64) {
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files/file-1/link"))
        .and(body_string_contains("604800"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {
                "url": "https://s3.example/presigned?sig=abc",
                "expiresAt": "2030-01-08T00:00:00.000Z",
                "costUsd": 0.0001
            }
        })))
        .expect(expect)
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_text_version_uploads_its_own_content_behind_a_private_link() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files"))
        .and(body_string_contains("# version one"))
        .and(body_string_contains("filename=\"Release Notes.md\""))
        .and(body_string_contains("private"))
        .respond_with(uploaded(None))
        .expect(1)
        .mount(&server)
        .await;
    mount_link(&server, 1).await;

    let mut record = ArtifactRecord::new(
        "a-1",
        "Release Notes",
        ArtifactKind::Markdown,
        "# version one",
        "agent",
        1,
    );
    record.push_version(ArtifactVersion::new(
        0,
        "# version two",
        tiny_artifacts::ArtifactAuthor::Agent,
        "agent",
        2,
    ));
    // Version 1, not the newest: its own text is what goes out.
    let link = publisher(server.uri(), AutonomyLevel::Full)
        .publish(
            &record,
            record.version(1).unwrap(),
            &PublishOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(link.url, "https://s3.example/presigned?sig=abc");
    assert!(!link.public);
    assert_eq!(link.expires_at_millis, Some(1_894_060_800_000));
}

#[tokio::test]
async fn a_public_file_share_uses_the_public_url_and_makes_no_link() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files"))
        .and(body_string_contains("PNG-BYTES"))
        .and(body_string_contains("filename=\"Chart.png\""))
        .and(body_string_contains("public"))
        .respond_with(uploaded(Some("https://api.example/public/file-1")))
        .expect(1)
        .mount(&server)
        .await;
    mount_link(&server, 0).await;

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("chart.png");
    std::fs::write(&file, b"PNG-BYTES").unwrap();
    let mut version = ArtifactVersion::new(
        1,
        file.display().to_string(),
        tiny_artifacts::ArtifactAuthor::Agent,
        "agent",
        1,
    );
    version.file = Some(FileRef {
        path: file.display().to_string(),
        size_bytes: 9,
        media_type: Some("image/png".into()),
    });
    let mut record = ArtifactRecord::pending("img-1", "Chart", ArtifactKind::Image, 1);
    record.versions.push(version.clone());

    let options = PublishOptions {
        public: true,
        ..PublishOptions::default()
    };
    let link = publisher(server.uri(), AutonomyLevel::Full)
        .publish(&record, &version, &options)
        .await
        .unwrap();
    assert_eq!(link.url, "https://api.example/public/file-1");
    assert!(link.public);
}

#[tokio::test]
async fn read_only_autonomy_blocks_sharing_before_any_network_call() {
    let record = ArtifactRecord::new("a-1", "T", ArtifactKind::Text, "x", "agent", 1);
    // An unreachable backend: a network attempt would fail differently.
    let err = publisher("http://127.0.0.1:9".into(), AutonomyLevel::ReadOnly)
        .publish(
            &record,
            record.latest().unwrap(),
            &PublishOptions::default(),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("[policy-blocked]"), "{err}");
}

#[tokio::test]
async fn sharing_through_the_service_records_and_reuses_the_link() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files"))
        .respond_with(uploaded(None))
        .expect(1)
        .mount(&server)
        .await;
    mount_link(&server, 1).await;

    let workspace = tempfile::tempdir().unwrap();
    let artifacts = Artifacts::new(Arc::new(versioned_store(workspace.path())))
        .with_publisher(Arc::new(publisher(server.uri(), AutonomyLevel::Full)));
    let id = artifacts
        .create(
            ArtifactKind::Markdown,
            "Notes",
            Content::Text("# hi".into()),
            &Author::agent("agent"),
            None,
        )
        .await
        .unwrap()
        .record
        .id;

    let first = artifacts
        .publish(&id, None, &PublishOptions::default())
        .await
        .unwrap();
    assert!(!first.reused);
    let stored = artifacts.get(&id).await.unwrap().unwrap();
    assert_eq!(
        stored.latest().unwrap().published.as_ref(),
        Some(&first.link)
    );

    // The link is live (expires 2030): no second upload.
    let again = artifacts
        .publish(&id, None, &PublishOptions::default())
        .await
        .unwrap();
    assert!(again.reused);
    assert_eq!(again.link.url, first.link.url);
}

#[test]
fn a_link_never_outlives_its_file() {
    let now = 1_000_000_000_000;
    let day = 86_400;
    // The free plan's file lives exactly 7 days from upload: a 7-day link
    // would end a moment after it, so it is cut to the file's life less a
    // minute.
    let file_expires = Some(now + 7 * day * 1000);
    assert_eq!(link_seconds(None, file_expires, now), 7 * day - 60);
    // A shorter request stands.
    assert_eq!(link_seconds(Some(3600), file_expires, now), 3600);
    // Requests are held to the backend's 60 s – 7 day range.
    assert_eq!(link_seconds(Some(10), None, now), 60);
    assert_eq!(link_seconds(Some(30 * day), None, now), 7 * day);
    // A file about to expire still gets the shortest valid link.
    assert_eq!(link_seconds(None, Some(now + 30_000), now), 60);
}

#[tokio::test]
async fn a_refused_lifetime_falls_back_to_the_backend_default() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files"))
        .respond_with(uploaded(None))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files/file-1/link"))
        .and(body_string_contains("expiresInSeconds"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({ "success": false, "error": "Validation failed" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files/file-1/link"))
        .and(wiremock::matchers::body_json(json!({})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": { "url": "https://s3.example/default", "expiresAt": "2030-01-01T01:00:00.000Z", "costUsd": 0.0 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let record = ArtifactRecord::new("a-1", "T", ArtifactKind::Text, "x", "agent", 1);
    let link = publisher(server.uri(), AutonomyLevel::Full)
        .publish(
            &record,
            record.latest().unwrap(),
            &PublishOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(link.url, "https://s3.example/default");
}

#[tokio::test]
async fn an_upload_that_cannot_be_linked_is_deleted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files"))
        .respond_with(uploaded(None))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files/file-1/link"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({ "success": false, "error": "Validation failed" })),
        )
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/agent-integrations/file-storage/files/file-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true, "data": { "deleted": true }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let record = ArtifactRecord::new("a-1", "T", ArtifactKind::Text, "x", "agent", 1);
    let err = publisher(server.uri(), AutonomyLevel::Full)
        .publish(
            &record,
            record.latest().unwrap(),
            &PublishOptions::default(),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("link generation failed"), "{err}");
}

#[tokio::test]
async fn a_server_failure_on_the_link_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files"))
        .respond_with(uploaded(None))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files/file-1/link"))
        .respond_with(
            ResponseTemplate::new(503)
                .set_body_json(json!({ "success": false, "error": "unavailable" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/agent-integrations/file-storage/files/file-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true, "data": { "deleted": true }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let record = ArtifactRecord::new("a-1", "T", ArtifactKind::Text, "x", "agent", 1);
    let err = publisher(server.uri(), AutonomyLevel::Full)
        .publish(
            &record,
            record.latest().unwrap(),
            &PublishOptions::default(),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("link generation failed"), "{err}");
}

#[tokio::test]
async fn a_file_in_a_protected_location_is_never_uploaded() {
    let dir = tempfile::tempdir().unwrap();
    let ssh = dir.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    let key = ssh.join("id_ed25519");
    std::fs::write(&key, b"PRIVATE").unwrap();
    let mut version = ArtifactVersion::new(
        1,
        key.display().to_string(),
        tiny_artifacts::ArtifactAuthor::Agent,
        "agent",
        1,
    );
    version.file = Some(FileRef {
        path: key.display().to_string(),
        size_bytes: 7,
        media_type: None,
    });
    let mut record = ArtifactRecord::pending("k-1", "Key", ArtifactKind::File, 1);
    record.versions.push(version.clone());

    // An unreachable backend: an upload attempt would fail differently.
    let err = publisher("http://127.0.0.1:9".into(), AutonomyLevel::Full)
        .publish(&record, &version, &PublishOptions::default())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("protected location"), "{err}");
}
