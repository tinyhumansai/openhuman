use super::*;
use crate::agent::attachments::parse;
use tinyinference_llm::message::{ContentBlock, Message};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDRpixels";
const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n";

fn config(root: &Path) -> Config {
    Config {
        action_dir: root.join("acting"),
        workspace_dir: root.join("internal"),
        ..Config::default()
    }
}

async fn staged(attachments: &[HostAttachment], config: &Config) -> anyhow::Result<String> {
    stage_host(
        attachments,
        "thread-1",
        config,
        &AttachmentAccessScope::default(),
    )
    .await
}

#[tokio::test]
async fn host_bytes_become_durable_references_in_order() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let references = staged(
        &[
            HostAttachment::new(PNG, "image/png"),
            HostAttachment::new(PDF, "application/pdf").named("../../Q3 report.pdf"),
        ],
        &config,
    )
    .await
    .unwrap();

    let (text, files) = parse(&references);
    assert_eq!(text.trim(), "");
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].mime, "image/png");
    assert_eq!(files[1].mime, "application/pdf");
    assert_eq!(files[1].name, "../../Q3 report.pdf");
    for (file, bytes) in files.iter().zip([PNG, PDF]) {
        assert!(file.path.starts_with("uploads/thread-1/"), "{}", file.path);
        assert!(!file.path.contains(".."), "{}", file.path);
        let stored = tokio::fs::read(config.action_dir.join(&file.path))
            .await
            .unwrap();
        assert_eq!(stored, bytes);
    }
    // No payload rides the reference itself.
    assert!(!references.contains("PNG"));
}

#[tokio::test]
async fn references_convert_to_image_and_document_blocks() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let references = staged(
        &[
            HostAttachment::new(PNG, "image/png"),
            HostAttachment::new(PDF, "application/pdf"),
        ],
        &config,
    )
    .await
    .unwrap();

    let message = crate::agent::message_convert::user_message_from_text(&format!(
        "{references}what is this?"
    ));
    let Message::User(user) = message else {
        panic!("not a user message");
    };
    assert!(user
        .content
        .iter()
        .any(|block| matches!(block, ContentBlock::Image(_))));
    assert!(user
        .content
        .iter()
        .any(|block| matches!(block, ContentBlock::Document(_))));
    assert!(user
        .content
        .iter()
        .any(|block| matches!(block, ContentBlock::Text(text) if text.contains("what is this?"))));
}

#[tokio::test]
async fn a_generic_type_is_sniffed_from_the_bytes_and_name() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let references = staged(
        &[
            HostAttachment::new(PNG, "application/octet-stream"),
            HostAttachment::new(PDF, "").named("notes.pdf"),
        ],
        &config,
    )
    .await
    .unwrap();
    let mimes: Vec<String> = parse(&references).1.into_iter().map(|f| f.mime).collect();
    assert_eq!(mimes, ["image/png", "application/pdf"]);
}

#[tokio::test]
async fn too_many_images_are_refused_before_anything_is_written() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = config(temp.path());
    config.multimodal.max_images = 1;
    let error = staged(
        &[
            HostAttachment::new(PNG, "image/png"),
            HostAttachment::new(PNG, "image/png"),
        ],
        &config,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("count exceeds"), "{error}");
    assert!(!config.action_dir.join("uploads").exists());
}

#[tokio::test]
async fn too_many_files_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = config(temp.path());
    config.multimodal_files.max_files = 1;
    let error = staged(
        &[
            HostAttachment::new(PDF, "application/pdf"),
            HostAttachment::new(PDF, "application/pdf"),
        ],
        &config,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("count exceeds"), "{error}");
}

#[tokio::test]
async fn an_oversized_attachment_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = config(temp.path());
    config.multimodal.max_image_size_mb = 1;
    let mut big = PNG.to_vec();
    big.resize(1024 * 1024 + 1, 0);
    let error = staged(&[HostAttachment::new(big, "image/png")], &config)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("size limit"), "{error}");
}

#[tokio::test]
async fn an_empty_attachment_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let error = staged(
        &[HostAttachment::new(Vec::new(), "application/pdf")],
        &config,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("empty"), "{error}");
}

#[test]
fn debug_never_prints_the_payload() {
    let attachment = HostAttachment::new(b"secret-bytes".to_vec(), "text/plain").named("a.txt");
    let printed = format!("{attachment:?}");
    assert!(!printed.contains("secret"), "{printed}");
    assert!(printed.contains("size_bytes: 12"), "{printed}");
}
