use super::*;
use std::path::Path;
use tinyinference_llm::message::UserMessage;

/// The flat weight the estimator charges a document block, in characters.
const FLAT: usize = 4 * 1024;

fn config_in(root: &Path) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.action_dir = root.into();
    config.workspace_dir = root.join("internal");
    config.config_path = config.workspace_dir.join("config.toml");
    config.modules.enabled = false;
    config.modules.allow_download = false;
    config
}

/// Writes an upload at `relative` under `root`, with `readout` cached beside
/// it as a provider fallback would write it (`<key>\n<text>`).
async fn upload(root: &Path, relative: &str, readout: Option<&str>) {
    let path = root.join(relative);
    let dir = path.parent().unwrap();
    tokio::fs::create_dir_all(dir).await.unwrap();
    tokio::fs::write(&path, b"%PDF-1.4 original bytes")
        .await
        .unwrap();
    if let Some(text) = readout {
        let derived = dir.join(".openhuman-derived");
        tokio::fs::create_dir_all(&derived).await.unwrap();
        tokio::fs::write(derived.join("readout.txt"), format!("key\n{text}"))
            .await
            .unwrap();
    }
}

fn document(reference: MediaRef) -> Message {
    Message::User(UserMessage {
        content: vec![
            ContentBlock::Text("[the user sent a document]".into()),
            ContentBlock::Document(reference),
        ],
    })
}

fn sent(path: &str) -> Message {
    document(MediaRef::Path {
        path: path.into(),
        media_type: Some("application/pdf".into()),
    })
}

async fn extra_tokens(root: &Path, history: &[Message]) -> u64 {
    let scope = AttachmentAccessScope {
        workspace: Some(root.into()),
        ..AttachmentAccessScope::default()
    };
    documents_extra_tokens(&config_in(root), &scope, history).await
}

#[test]
fn the_flat_weight_matches_what_the_estimator_charges_a_document() {
    let block = ContentBlock::Document(MediaRef::Path {
        path: "uploads/t/a/book.pdf".into(),
        media_type: None,
    });
    assert_eq!(block.estimated_char_weight(), FLAT);
}

#[tokio::test]
async fn a_document_counts_its_cached_readout_beyond_the_flat_weight() {
    let temp = tempfile::tempdir().unwrap();
    let readout = "x".repeat(FLAT + 40_000);
    upload(temp.path(), "uploads/t/a/book.pdf", Some(&readout)).await;

    let extra = extra_tokens(temp.path(), &[sent("uploads/t/a/book.pdf")]).await;

    assert_eq!(extra, 10_000);
}

#[tokio::test]
async fn every_document_the_history_replays_is_counted() {
    let temp = tempfile::tempdir().unwrap();
    upload(
        temp.path(),
        "uploads/t/a/one.pdf",
        Some(&"x".repeat(FLAT + 4_000)),
    )
    .await;
    upload(
        temp.path(),
        "uploads/t/b/two.pdf",
        Some(&"y".repeat(FLAT + 8_000)),
    )
    .await;
    let history = [
        sent("uploads/t/a/one.pdf"),
        Message::assistant("read it"),
        sent("uploads/t/b/two.pdf"),
        sent("uploads/t/a/one.pdf"),
    ];

    let extra = extra_tokens(temp.path(), &history).await;

    assert_eq!(extra, 1_000 + 2_000 + 1_000);
}

#[tokio::test]
async fn a_document_with_no_readout_keeps_its_flat_weight() {
    let temp = tempfile::tempdir().unwrap();
    upload(temp.path(), "uploads/t/a/book.pdf", None).await;

    let extra = extra_tokens(temp.path(), &[sent("uploads/t/a/book.pdf")]).await;

    assert_eq!(extra, 0);
}

#[tokio::test]
async fn a_readout_shorter_than_the_flat_weight_adds_nothing() {
    let temp = tempfile::tempdir().unwrap();
    upload(temp.path(), "uploads/t/a/ticket.pdf", Some("PNR Q8C7KV")).await;

    let extra = extra_tokens(temp.path(), &[sent("uploads/t/a/ticket.pdf")]).await;

    assert_eq!(extra, 0);
}

#[tokio::test]
async fn a_document_by_url_is_left_at_its_flat_weight() {
    let temp = tempfile::tempdir().unwrap();
    let history = [document(MediaRef::Url {
        url: "https://example.com/book.pdf".into(),
        media_type: None,
    })];

    assert_eq!(extra_tokens(temp.path(), &history).await, 0);
    assert_eq!(documents_of(&history).count(), 0);
}

#[test]
fn the_fold_threshold_is_unchanged_without_documents() {
    assert_eq!(fold_threshold(200_000, 0), 200_000);
}

#[test]
fn the_fold_threshold_moves_down_by_the_document_tokens() {
    assert_eq!(fold_threshold(200_000, 150_000), 50_000);
}

#[test]
fn the_fold_threshold_keeps_a_floor_for_the_verbatim_tail() {
    assert_eq!(fold_threshold(200_000, 190_000), FOLD_FLOOR_TOKENS);
    assert_eq!(fold_threshold(200_000, 400_000), FOLD_FLOOR_TOKENS);
}

#[test]
fn a_threshold_under_the_floor_is_kept() {
    assert_eq!(fold_threshold(32_000, 30_000), 32_000);
}
