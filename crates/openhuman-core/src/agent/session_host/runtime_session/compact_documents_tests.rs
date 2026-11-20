use super::*;
use std::path::Path;
use std::sync::Arc;
use tinyinference_llm::message::UserMessage;

use crate::agent::tinyagents::context_wiring::{compact_history, CompactionSummarizer, IdleFold};

/// The flat weight the estimator charges a document block, in characters.
const FLAT: usize = 4 * 1024;

fn config_in(root: &Path) -> Config {
    let mut config = Config::default();
    config.action_dir = root.into();
    config.workspace_dir = root.join("internal");
    config.config_path = config.workspace_dir.join("config.toml");
    config.modules.enabled = false;
    config.modules.allow_download = false;
    config
}

/// Writes an upload at `relative` under `root`, with `readout` cached beside
/// it as a provider fallback writes it (`<key>\n<text>`).
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

fn message(block: ContentBlock) -> Message {
    Message::User(UserMessage {
        content: vec![ContentBlock::Text("[the user sent a file]".into()), block],
    })
}

fn document(reference: MediaRef) -> ContentBlock {
    ContentBlock::Document(reference)
}

fn sent(path: &str) -> Message {
    message(document(MediaRef::Path {
        path: path.into(),
        media_type: Some("application/pdf".into()),
    }))
}

fn photo(n: usize) -> Message {
    message(ContentBlock::Image(tinyinference_llm::message::ImageRef {
        url: format!("uploads/t/p{n}/photo.jpg"),
        mime_type: None,
    }))
}

async fn extra(root: &Path, config: &Config, history: &[Message]) -> u64 {
    let scope = AttachmentAccessScope {
        workspace: Some(root.into()),
        ..AttachmentAccessScope::default()
    };
    extra_tokens(&DispatchCtx::ambient(), config, &scope, history).await
}

async fn sized(root: &Path, history: &[Message]) -> u64 {
    extra(root, &config_in(root), history).await
}

#[test]
fn the_flat_weight_matches_what_the_estimator_charges_a_document() {
    let block = document(MediaRef::Path {
        path: "uploads/t/a/book.pdf".into(),
        media_type: None,
    });
    assert_eq!(block.estimated_char_weight(), FLAT);
}

#[tokio::test]
async fn a_document_counts_its_cached_readout_beyond_the_flat_weight() {
    let temp = tempfile::tempdir().unwrap();
    upload(
        temp.path(),
        "uploads/t/a/book.pdf",
        Some(&"x".repeat(FLAT + 40_000)),
    )
    .await;

    assert_eq!(
        sized(temp.path(), &[sent("uploads/t/a/book.pdf")]).await,
        10_000
    );
}

#[tokio::test]
async fn every_replayed_document_is_counted() {
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

    assert_eq!(sized(temp.path(), &history).await, 1_000 + 2_000 + 1_000);
}

#[tokio::test]
async fn only_the_documents_the_history_budget_replays_are_counted() {
    let temp = tempfile::tempdir().unwrap();
    let mut history = Vec::new();
    for n in 0..MAX_HISTORICAL_MEDIA_BLOCKS + 2 {
        let path = format!("uploads/t/d{n}/book.pdf");
        upload(temp.path(), &path, Some(&"x".repeat(FLAT + 4_000))).await;
        history.push(sent(&path));
    }

    let counted = sized(temp.path(), &history).await;

    assert_eq!(counted, MAX_HISTORICAL_MEDIA_BLOCKS as u64 * 1_000);
}

#[tokio::test]
async fn newer_photos_use_up_the_history_budget_before_an_older_document() {
    let temp = tempfile::tempdir().unwrap();
    upload(
        temp.path(),
        "uploads/t/a/book.pdf",
        Some(&"x".repeat(FLAT + 40_000)),
    )
    .await;
    let mut history = vec![sent("uploads/t/a/book.pdf")];
    history.extend((0..MAX_HISTORICAL_MEDIA_BLOCKS).map(photo));

    assert_eq!(sized(temp.path(), &history).await, 0);
}

#[test]
fn file_and_image_markers_in_text_take_history_slots() {
    let book = MediaRef::Path {
        path: "uploads/t/a/book.pdf".into(),
        media_type: None,
    };
    let markers = Message::user("[IMAGE:uploads/t/x.jpg] and [FILE:uploads/t/y.txt]");
    let history = [message(document(book.clone())), markers];

    assert_eq!(replayed_documents(&history, 3).len(), 1);
    assert!(replayed_documents(&history, 2).is_empty());
}

#[tokio::test]
async fn nothing_is_counted_when_attachments_are_off() {
    let temp = tempfile::tempdir().unwrap();
    upload(
        temp.path(),
        "uploads/t/a/book.pdf",
        Some(&"x".repeat(FLAT + 40_000)),
    )
    .await;
    let mut config = config_in(temp.path());
    config.multimodal_files.max_files = 0;

    assert_eq!(
        extra(temp.path(), &config, &[sent("uploads/t/a/book.pdf")]).await,
        0
    );
}

#[tokio::test]
async fn a_document_with_no_readout_keeps_its_flat_weight() {
    let temp = tempfile::tempdir().unwrap();
    upload(temp.path(), "uploads/t/a/book.pdf", None).await;

    assert_eq!(sized(temp.path(), &[sent("uploads/t/a/book.pdf")]).await, 0);
}

#[tokio::test]
async fn a_readout_shorter_than_the_flat_weight_adds_nothing() {
    let temp = tempfile::tempdir().unwrap();
    upload(temp.path(), "uploads/t/a/ticket.pdf", Some("PNR Q8C7KV")).await;

    assert_eq!(
        sized(temp.path(), &[sent("uploads/t/a/ticket.pdf")]).await,
        0
    );
}

#[tokio::test]
async fn a_document_by_url_keeps_its_flat_weight() {
    let temp = tempfile::tempdir().unwrap();
    let history = [message(document(MediaRef::Url {
        url: "https://example.com/book.pdf".into(),
        media_type: None,
    }))];

    assert_eq!(sized(temp.path(), &history).await, 0);
}

#[tokio::test]
async fn a_staged_document_that_does_not_resolve_keeps_its_flat_weight() {
    let temp = tempfile::tempdir().unwrap();
    let staged = message(document(MediaRef::Url {
        url: "openhuman-staged://not-registered".into(),
        media_type: Some("application/pdf".into()),
    }));
    let scope = AttachmentAccessScope {
        workspace: Some(temp.path().into()),
        thread_id: Some("thread".into()),
        ..AttachmentAccessScope::default()
    };

    let counted = extra_tokens(
        &DispatchCtx::ambient(),
        &config_in(temp.path()),
        &scope,
        &[staged.clone()],
    )
    .await;

    assert_eq!(counted, 0);
    let unscoped = sized(temp.path(), &[staged]).await;
    assert_eq!(unscoped, 0, "without a thread a staged id is not resolved");
}

#[test]
fn a_history_with_no_document_has_none() {
    assert!(!has_documents(&[
        Message::user("hi"),
        Message::assistant("hello")
    ]));
    assert!(has_documents(&[sent("uploads/t/a/book.pdf")]));
}

#[test]
fn the_fold_threshold_is_unchanged_without_documents() {
    assert_eq!(fold_threshold(200_000, 0, 250_000), 200_000);
}

#[test]
fn the_fold_threshold_moves_down_by_the_document_tokens() {
    assert_eq!(fold_threshold(200_000, 150_000, 120_000), 50_000);
}

#[test]
fn the_fold_threshold_keeps_a_floor_for_the_verbatim_tail() {
    assert_eq!(fold_threshold(200_000, 190_000, 120_000), FOLD_FLOOR_TOKENS);
    assert_eq!(fold_threshold(32_000, 30_000, 120_000), 32_000);
}

#[test]
fn the_fold_threshold_never_exceeds_what_the_fold_measures() {
    assert_eq!(fold_threshold(200_000, 190_000, 10_000), 10_000);
    assert_eq!(fold_threshold(200_000, 400_000, 0), 1);
}

/// A thread with little text but long documents passes the gate on its
/// readouts. The fold, which measures documents at their flat weight, must
/// still fold its old messages, documents included.
#[tokio::test]
async fn a_document_heavy_thread_with_little_text_is_folded() {
    use tinyagents_harness::testkit::ScriptedModel;

    let mut history = vec![Message::system("system prompt")];
    for round in 0..6 {
        history.push(sent(&format!("uploads/t/d{round}/book.pdf")));
        history.push(Message::assistant(format!(
            "answer {round}: {}",
            "result ".repeat(60)
        )));
    }
    let measured = tinyagents_harness::token_estimation::estimate_slice_tokens(&history);
    assert!(
        measured < FOLD_FLOOR_TOKENS,
        "the fold's own measure is under the floor"
    );
    let summarizer = Arc::new(ScriptedModel::replies(vec!["## Goal\nDOCS-SUMMARY"; 4]));

    let folded = compact_history(
        &history,
        IdleFold {
            min_tokens: fold_threshold(200_000, 190_000, measured),
            keep_recent_tokens: None,
            strategy: crate::config::CompactionStrategy::Summary,
            context_window: None,
            model: "turn-model",
        },
        CompactionSummarizer {
            model: summarizer.clone(),
            model_id: "m".into(),
            settings: Default::default(),
        },
    )
    .await
    .expect("the fold runs")
    .expect("the old documents are folded");

    assert!(folded.len() < history.len());
    assert!(folded[1].text().contains("DOCS-SUMMARY"));
    assert!(
        !folded.contains(&history[1]),
        "the oldest document is folded into the summary"
    );
}
