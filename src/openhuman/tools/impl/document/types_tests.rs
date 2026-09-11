use super::*;

/// One valid section carrying a heading + a paragraph + a bullet.
fn section() -> DocumentSection {
    DocumentSection {
        heading: Some("Overview".to_string()),
        paragraphs: vec!["A body paragraph.".to_string()],
        bullets: vec!["A bullet".to_string()],
    }
}

/// A minimal valid input; individual tests mutate one field to drive a
/// single rejection branch.
fn base() -> GenerateDocumentInput {
    GenerateDocumentInput {
        title: "Charter".to_string(),
        author: Some("Alice".to_string()),
        sections: vec![section()],
    }
}

/// Assert `validate_input` rejects `input` naming `field` in the error.
fn assert_rejects(input: &GenerateDocumentInput, field: &str) {
    match validate_input(input) {
        Err(DocumentError::InvalidInput { field: f, .. }) => {
            assert!(
                f.contains(field),
                "expected error field to contain {field:?}, got {f:?}"
            );
        }
        other => panic!("expected InvalidInput({field}), got {other:?}"),
    }
}

#[test]
fn accepts_a_well_formed_input() {
    assert!(validate_input(&base()).is_ok());
}

#[test]
fn rejects_an_empty_title() {
    let mut input = base();
    input.title = "   ".to_string();
    assert_rejects(&input, "title");
}

#[test]
fn rejects_an_empty_section_list() {
    let mut input = base();
    input.sections.clear();
    assert_rejects(&input, "sections");
}

#[test]
fn rejects_a_blank_section_naming_its_index() {
    let mut input = base();
    input.sections.push(DocumentSection {
        heading: Some("  ".to_string()),
        paragraphs: vec![],
        bullets: vec![],
    });
    assert_rejects(&input, "sections[1]");
}

#[test]
fn rejects_over_long_text_fields() {
    let mut input = base();
    input.title = "t".repeat(MAX_TEXT_CHARS + 1);
    assert_rejects(&input, "title");
}

#[test]
fn tinydocs_invalid_input_keeps_its_field_and_reason() {
    // The structured pair is what the agent self-corrects on, so the
    // crate-boundary mapping must not flatten it into a message string.
    let mapped = DocumentError::from(
        crate::openhuman::tools::implementations::document::format::Error::InvalidInput {
            field: "sections[3].bullets[1]".to_string(),
            reason: "must be ≤ 20000 chars".to_string(),
        },
    );
    match mapped {
        DocumentError::InvalidInput { field, reason } => {
            assert_eq!(field, "sections[3].bullets[1]");
            assert_eq!(reason, "must be ≤ 20000 chars");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn tinydocs_generation_failure_maps_without_re_truncating() {
    // `tinydocs` already truncated this detail; re-truncating would eat
    // the suffix and misreport how much was dropped.
    let detail = crate::openhuman::tools::implementations::document::format::Error::truncate_detail(
        &"x".repeat(10_000),
    );
    let mapped = DocumentError::from(
        crate::openhuman::tools::implementations::document::format::Error::GenerationFailed {
            detail: detail.clone(),
        },
    );
    match mapped {
        DocumentError::GenerationFailed { stderr_truncated } => {
            assert_eq!(stderr_truncated, detail);
        }
        other => panic!("expected GenerationFailed, got {other:?}"),
    }
}

#[test]
fn truncate_stderr_bounds_the_payload() {
    let out = DocumentError::truncate_stderr(&"x".repeat(10_000));
    assert_eq!(
        out.chars().count(),
        crate::openhuman::tools::implementations::document::format::Error::MAX_DETAIL_CHARS
    );
    assert!(out.ends_with("[…truncated]"));
}

#[test]
fn the_json_wire_shape_is_unchanged_by_the_extraction() {
    // The agent-facing schema is `{title, author?, sections:[{heading?,
    // paragraphs?, bullets?}]}`. Pinning it here catches a `tinydocs`
    // bump that renames a field out from under the tool schema.
    let input: GenerateDocumentInput = serde_json::from_value(serde_json::json!({
        "title": "T",
        "author": "A",
        "sections": [{ "heading": "H", "paragraphs": ["p"], "bullets": ["b"] }],
    }))
    .expect("the historical wire shape must still deserialise");
    assert_eq!(input.title, "T");
    assert_eq!(input.author.as_deref(), Some("A"));
    assert_eq!(input.sections.len(), 1);
    assert_eq!(input.sections[0].heading.as_deref(), Some("H"));
    assert_eq!(input.sections[0].paragraphs, vec!["p".to_string()]);
    assert_eq!(input.sections[0].bullets, vec!["b".to_string()]);
}

#[test]
fn optional_fields_still_default() {
    let input: GenerateDocumentInput =
        serde_json::from_value(serde_json::json!({ "title": "T", "sections": [] }))
            .expect("author and per-section fields are optional");
    assert!(input.author.is_none());
    assert!(input.sections.is_empty());
}
