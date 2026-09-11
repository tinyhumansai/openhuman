use super::*;

#[test]
fn sanitize_inline_completion_handles_placeholders_and_clamps_length() {
    assert_eq!(sanitize_inline_completion("none", "hello"), "");
    assert_eq!(sanitize_inline_completion("n/a", "hello"), "");
    assert_eq!(
        sanitize_inline_completion("\"- hello world\"", "hello"),
        "hello world"
    );

    let long = "a".repeat(256);
    let out = sanitize_inline_completion(&long, "hello");
    assert_eq!(out.chars().count(), 96);
}

#[test]
fn sanitize_inline_completion_strips_arrow_and_extra_whitespace() {
    assert_eq!(
        sanitize_inline_completion("\t→  keep   it concise\t", "hello"),
        "keep it concise"
    );
}

#[test]
fn sanitize_inline_completion_strips_quoted_generation_label() {
    assert_eq!(
        sanitize_inline_completion("\"suffix: hello\"", "context example"),
        "hello"
    );
}

#[test]
fn sanitize_inline_completion_returns_suffix_only_when_model_repeats_context() {
    let ctx = "Yesterday, I went";
    let raw = "Yesterday, I went to the garden";
    assert_eq!(sanitize_inline_completion(raw, ctx), "to the garden");
}

#[test]
fn sanitize_inline_completion_drops_tabby_unicode_noise() {
    let ctx = "Yester";
    let raw = "Yester\tday, \u{2028}I went\t to garden";
    assert_eq!(
        sanitize_inline_completion(raw, ctx),
        "day, I went to garden"
    );
}

#[test]
fn sanitize_inline_completion_preserves_iso_date_prefix() {
    assert_eq!(
        sanitize_inline_completion("2026-04-07", "context example"),
        "2026-04-07"
    );
}

#[test]
fn sanitize_inline_completion_preserves_time_prefix() {
    assert_eq!(
        sanitize_inline_completion("3pm meeting", "context example"),
        "3pm meeting"
    );
}

#[test]
fn sanitize_inline_completion_preserves_double_dash_help_token() {
    assert_eq!(
        sanitize_inline_completion("--help", "context example"),
        "--help"
    );
}

#[test]
fn sanitize_inline_completion_preserves_task_marker_without_space() {
    assert_eq!(
        sanitize_inline_completion("-[ ] task", "context example"),
        "-[ ] task"
    );
}

#[test]
fn sanitize_inline_completion_strips_numbered_list_prefix_dot() {
    assert_eq!(
        sanitize_inline_completion("1. item", "context example"),
        "item"
    );
}

#[test]
fn sanitize_inline_completion_strips_numbered_list_prefix_paren() {
    assert_eq!(
        sanitize_inline_completion("2) item", "context example"),
        "item"
    );
}
