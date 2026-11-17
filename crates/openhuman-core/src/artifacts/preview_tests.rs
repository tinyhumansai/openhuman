use std::path::Path;

use tiny_artifacts::{ArtifactAuthor, ArtifactKind, ArtifactRecord, ArtifactVersion, FileRef};

use super::*;

fn file_record(path: &Path) -> ArtifactRecord {
    let mut version = ArtifactVersion::new(
        1,
        path.display().to_string(),
        ArtifactAuthor::Agent,
        "agent",
        1,
    );
    version.file = Some(FileRef {
        key: path.display().to_string(),
        size_bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        media_type: None,
    });
    let mut record = ArtifactRecord::pending("f-1", "Report", ArtifactKind::File, 1);
    record.versions.push(version);
    record
}

fn roots(dir: &Path) -> tiny_artifacts::FileRoots {
    tiny_artifacts::FileRoots::new([dir.to_path_buf()])
}

#[tokio::test]
async fn a_text_kind_shows_its_newest_version() {
    let mut record =
        ArtifactRecord::new("m-1", "Notes", ArtifactKind::Markdown, "# v1", "agent", 1);
    record.push_version(ArtifactVersion::new(
        0,
        "# v2",
        ArtifactAuthor::Operator,
        "operator",
        2,
    ));
    let dir = tempfile::tempdir().unwrap();
    let body = body_of(&record, &roots(dir.path()), &DocumentConverter::none()).await;
    assert_eq!(body.kind, "markdown");
    assert_eq!(body.version, 2);
    assert_eq!(body.body, "# v2");
    assert!(body.is_text && !body.converted);
}

#[tokio::test]
async fn a_text_file_inside_the_roots_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "plain notes").unwrap();
    let body = body_of(
        &file_record(&file),
        &roots(dir.path()),
        &DocumentConverter::none(),
    )
    .await;
    assert_eq!(body.kind, "file");
    assert_eq!(body.body, "plain notes");
    assert!(body.is_text);
}

#[tokio::test]
async fn a_file_outside_the_roots_is_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let secret = elsewhere.path().join("secret.txt");
    std::fs::write(&secret, "do not read").unwrap();
    let body = body_of(
        &file_record(&secret),
        &roots(dir.path()),
        &DocumentConverter::none(),
    )
    .await;
    // Shown as a file: its path, never its content.
    assert!(!body.is_text);
    assert_eq!(body.body, secret.display().to_string());
}

#[tokio::test]
async fn a_file_over_the_preview_limit_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("big.txt");
    std::fs::File::create(&file)
        .unwrap()
        .set_len(PREVIEW_MAX_BYTES + 1)
        .unwrap();
    let body = body_of(
        &file_record(&file),
        &roots(dir.path()),
        &DocumentConverter::none(),
    )
    .await;
    assert!(!body.is_text);
    assert_eq!(body.body, file.display().to_string());
}

#[tokio::test]
async fn a_binary_file_with_no_converter_shows_as_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("report.docx");
    std::fs::write(&file, b"PK\x03\x04\x00\x00binary").unwrap();
    let body = body_of(
        &file_record(&file),
        &roots(dir.path()),
        &DocumentConverter::none(),
    )
    .await;
    assert!(!body.is_text && !body.converted);
    assert_eq!(body.body, file.display().to_string());
}

fn config_in(workspace: &Path) -> crate::config::Config {
    crate::config::Config {
        workspace_dir: workspace.to_path_buf(),
        action_dir: workspace.join("action"),
        files_dir_override: Some(workspace.join("Files")),
        ..crate::config::Config::default()
    }
}

#[tokio::test]
async fn ids_that_name_another_file_are_refused() {
    let workspace = tempfile::tempdir().unwrap();
    let config = config_in(workspace.path());
    for id in ["../x", "a/b", "..", ".hidden"] {
        assert!(read_body(&config, id).await.is_err(), "{id:?}");
    }
}

#[tokio::test]
async fn an_unknown_artifact_is_not_found() {
    let workspace = tempfile::tempdir().unwrap();
    let err = read_body(&config_in(workspace.path()), "missing")
        .await
        .unwrap_err();
    assert!(err.contains("not found"), "{err}");
}

#[tokio::test]
async fn an_artifact_made_before_versioning_opens_from_its_legacy_record() {
    use crate::artifacts::store::{create_artifact, finalize_artifact};
    use crate::artifacts::types::ArtifactKind as LegacyKind;

    let workspace = tempfile::tempdir().unwrap();
    let config = config_in(workspace.path());
    let files = config.files_dir();
    std::fs::create_dir_all(&files).unwrap();
    // Only the legacy record: no versioned one.
    let (meta, path) = create_artifact(
        workspace.path(),
        files.clone(),
        LegacyKind::Document,
        "Old notes",
        "md",
        None,
    )
    .await
    .unwrap();
    std::fs::write(&path, "# from before").unwrap();
    finalize_artifact(workspace.path(), &meta.id, 13, None)
        .await
        .unwrap();

    let body = read_body(&config, &meta.id).await.unwrap();
    assert_eq!(body.title, "Old notes");
    assert_eq!(body.kind, "file");
    assert_eq!(body.body, "# from before");
    assert!(body.is_text);
}
