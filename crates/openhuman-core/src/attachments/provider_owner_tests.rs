use super::*;

#[test]
fn omitted_unsupported_url_is_redacted_without_changing_local_path_detection() {
    let source = "ftp://docs.example.test/old.pdf?signature=private-query";
    let block = history_source_block(InputModality::Document, source);
    let hint = super::super::historical_source_hint(&block, &AttachmentAccessScope::default());
    assert_eq!(hint, "remote or inline media omitted");
    assert!(!hint.contains("private-query"));

    let local = history_source_block(InputModality::Document, r"C:\docs\old.pdf");
    assert!(matches!(
        local,
        ContentBlock::Document(MediaRef::Path { .. })
    ));
}
