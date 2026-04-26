//! Shape + validation tests for the pure, pre-IO helpers used by the
//! threads RPC surface. Every test here avoids disk, network, and
//! provider calls — they pin the behaviour of the branches that all of
//! the async `ops::*` entry points rely on.
use super::*;
use crate::openhuman::threads::title::collapse_whitespace;
use serde_json::{json, Value};

// ── request_id ────────────────────────────────────────────────

#[test]
fn request_id_is_a_non_empty_uuid_and_fresh_per_call() {
    let a = request_id();
    let b = request_id();
    assert!(!a.is_empty());
    // v4 UUID canonical form: 36 chars with 4 hyphens.
    assert_eq!(a.len(), 36);
    assert_eq!(a.chars().filter(|c| *c == '-').count(), 4);
    // Two calls must not collide — catches accidental caching.
    assert_ne!(a, b);
}

// ── counts ────────────────────────────────────────────────────

#[test]
fn counts_materialises_entries_as_owned_string_keys() {
    let map = counts([("num_threads", 3), ("num_messages", 7)]);
    assert_eq!(map.get("num_threads"), Some(&3));
    assert_eq!(map.get("num_messages"), Some(&7));
    assert_eq!(map.len(), 2);
}

#[test]
fn counts_empty_iter_yields_empty_map() {
    let map = counts([]);
    assert!(map.is_empty());
}

// ── title_log_fingerprint ─────────────────────────────────────

#[test]
fn title_log_fingerprint_is_16_lowercase_hex_chars() {
    let fp = title_log_fingerprint("Chat Jan 1 1:00 AM");
    assert_eq!(fp.len(), 16);
    assert!(
        fp.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "fingerprint must be lowercase hex, got: {fp}"
    );
}

#[test]
fn title_log_fingerprint_is_deterministic_for_same_title() {
    // The fingerprint is only used for debug logging — the only real
    // contract is stability across calls inside a single process so
    // grep-friendly logs remain correlatable.
    let a = title_log_fingerprint("My cool thread");
    let b = title_log_fingerprint("My cool thread");
    assert_eq!(a, b);
}

#[test]
fn title_log_fingerprint_differs_for_different_titles() {
    let a = title_log_fingerprint("thread one");
    let b = title_log_fingerprint("thread two");
    assert_ne!(a, b, "distinct titles must produce distinct fingerprints");
}

// ── collapse_whitespace ───────────────────────────────────────

#[test]
fn collapse_whitespace_collapses_runs_and_trims_edges() {
    assert_eq!(collapse_whitespace("  a   b\tc\nd  "), "a b c d");
}

#[test]
fn collapse_whitespace_on_empty_or_whitespace_only_is_empty() {
    assert_eq!(collapse_whitespace(""), "");
    assert_eq!(collapse_whitespace("   \t\n "), "");
}

// ── build_title_prompt ────────────────────────────────────────

#[test]
fn build_title_prompt_renders_user_and_assistant_sections_in_order() {
    let prompt = build_title_prompt("hi there", "hello back");
    assert_eq!(
        prompt,
        "First user message:\nhi there\n\nAssistant reply:\nhello back\n\nReturn the best thread title."
    );
}

// ── sanitize_generated_title ──────────────────────────────────

#[test]
fn sanitize_generated_title_trims_surrounding_quotes_and_trailing_punct() {
    assert_eq!(
        sanitize_generated_title("\"Hello, world!\""),
        Some("Hello, world".to_string())
    );
    assert_eq!(
        sanitize_generated_title("`Hello`"),
        Some("Hello".to_string())
    );
    assert_eq!(
        sanitize_generated_title("'Plan trip'"),
        Some("Plan trip".to_string())
    );
}

#[test]
fn sanitize_generated_title_strips_repeated_trailing_punct() {
    assert_eq!(
        sanitize_generated_title("Check this out.!?"),
        Some("Check this out".to_string())
    );
}

#[test]
fn sanitize_generated_title_picks_first_non_empty_line() {
    assert_eq!(
        sanitize_generated_title("\n   \nLine one\nLine two"),
        Some("Line one".to_string())
    );
}

#[test]
fn sanitize_generated_title_returns_none_for_empty_or_whitespace() {
    assert!(sanitize_generated_title("").is_none());
    assert!(sanitize_generated_title("   \n\t").is_none());
    assert!(sanitize_generated_title("\"\"").is_none());
}

#[test]
fn sanitize_generated_title_collapses_internal_whitespace() {
    assert_eq!(
        sanitize_generated_title("Very   spaced\tout"),
        Some("Very spaced out".to_string())
    );
}

#[test]
fn sanitize_generated_title_truncates_to_80_chars_by_char_count() {
    // 100 `a` chars → must truncate to exactly 80. Char-based truncation
    // is load-bearing so multibyte titles never get sliced mid-codepoint.
    let raw = "a".repeat(100);
    let out = sanitize_generated_title(&raw).expect("non-empty");
    assert_eq!(out.chars().count(), 80);
}

#[test]
fn sanitize_generated_title_truncation_is_char_safe_for_multibyte() {
    // 100 emoji (4-byte UTF-8 each) must still truncate on char
    // boundaries, proving the `.chars().take(80)` vs byte slicing
    // guarantee.
    let raw = "🌍".repeat(100);
    let out = sanitize_generated_title(&raw).expect("non-empty");
    assert_eq!(out.chars().count(), 80);
}

// ── is_auto_generated_thread_title ────────────────────────────

#[test]
fn is_auto_generated_thread_title_accepts_canonical_new_chat_format() {
    // Parser locks the format produced by `thread_create_new`:
    // "Chat <Mon> <day> <H:MM> AM|PM".
    assert!(is_auto_generated_thread_title("Chat Jan 1 1:00 AM"));
    assert!(is_auto_generated_thread_title("Chat Dec 31 12:59 PM"));
}

#[test]
fn is_auto_generated_thread_title_tolerates_surrounding_whitespace() {
    // Input is trimmed before parsing — storage layers may round-trip
    // titles with stray whitespace.
    assert!(is_auto_generated_thread_title("  Chat Jan 1 1:00 AM  "));
}

#[test]
fn is_auto_generated_thread_title_rejects_user_edited_titles() {
    // Any freeform user title must fall through to the "not a
    // placeholder" branch so we never overwrite user-authored names.
    assert!(!is_auto_generated_thread_title("My custom title"));
    assert!(!is_auto_generated_thread_title("Trip planning"));
}

#[test]
fn is_auto_generated_thread_title_rejects_short_strings() {
    // Hard `bytes.len() < 16` guard — locks in the minimum shape so
    // we never enter the parser with too-small input.
    assert!(!is_auto_generated_thread_title(""));
    assert!(!is_auto_generated_thread_title("Chat"));
    assert!(!is_auto_generated_thread_title("Chat Jan 1"));
}

#[test]
fn is_auto_generated_thread_title_rejects_non_alpha_month() {
    // Month abbreviation must be 3 ASCII alpha chars.
    assert!(!is_auto_generated_thread_title("Chat 123 1 1:00 AM"));
}

#[test]
fn is_auto_generated_thread_title_rejects_long_month_name() {
    // "January 1 1:00 AM" — after "Chat ", bytes[8] is 'u' not ' '.
    assert!(!is_auto_generated_thread_title("Chat January 1 1:00 AM"));
}

#[test]
fn is_auto_generated_thread_title_rejects_three_digit_day() {
    // day: 1–2 ASCII digits; idx-day_start>2 rejects.
    assert!(!is_auto_generated_thread_title("Chat Jan 100 1:00 AM"));
}

#[test]
fn is_auto_generated_thread_title_rejects_missing_colon() {
    // 3-digit hour consumes through the position the `:` must occupy.
    assert!(!is_auto_generated_thread_title("Chat Jan 1 100 AM"));
}

#[test]
fn is_auto_generated_thread_title_rejects_lowercase_meridiem() {
    // Parser only accepts "AM" | "PM" (not "am"/"pm") so pattern stays
    // tied to the producer in `thread_create_new`.
    assert!(!is_auto_generated_thread_title("Chat Jan 1 1:00 am"));
}

#[test]
fn is_auto_generated_thread_title_rejects_missing_space_before_meridiem() {
    // The `bytes[idx + 2] != b' '` guard must reject "1:00AM" (no space).
    assert!(!is_auto_generated_thread_title("Chat Jan 1 1:00AM"));
}

// ── envelope ──────────────────────────────────────────────────

#[test]
fn envelope_sets_data_and_propagates_counts_and_pagination() {
    let pagination = PaginationMeta {
        limit: 10,
        offset: 0,
        count: 7,
    };
    let counts_map = counts([("num_messages", 7)]);
    let out = envelope(
        json!({"v": 42}),
        Some(counts_map.clone()),
        Some(pagination.clone()),
    );
    let env = &out.value;
    assert_eq!(env.data.as_ref().unwrap()["v"], json!(42));
    assert!(env.error.is_none());
    assert!(!env.meta.request_id.is_empty());
    assert_eq!(env.meta.counts.as_ref().unwrap(), &counts_map);
    let pag = env.meta.pagination.as_ref().unwrap();
    assert_eq!(pag.limit, pagination.limit);
    assert_eq!(pag.count, pagination.count);
    assert_eq!(pag.offset, pagination.offset);
    // No implicit latency/cached info — the envelope helper keeps
    // optional fields unset so callers opt in explicitly.
    assert!(env.meta.latency_seconds.is_none());
    assert!(env.meta.cached.is_none());
    // No logs are attached by default.
    assert!(out.logs.is_empty());
}

#[test]
fn envelope_omits_counts_and_pagination_when_not_provided() {
    let out = envelope(json!(null), None, None);
    assert!(out.value.meta.counts.is_none());
    assert!(out.value.meta.pagination.is_none());
}

#[test]
fn envelope_generates_unique_request_ids_per_call() {
    // request_id uniqueness matters for client-side correlation of
    // overlapping threads-API calls. Lock it in.
    let a = envelope(json!({}), None, None);
    let b = envelope(json!({}), None, None);
    assert_ne!(a.value.meta.request_id, b.value.meta.request_id);
}

// ── thread_to_summary / message_to_record / record_to_message ─

fn sample_thread() -> ConversationThread {
    ConversationThread {
        id: "t-1".into(),
        title: "My thread".into(),
        chat_id: Some(42),
        is_active: true,
        message_count: 5,
        last_message_at: "2026-01-01T00:00:00Z".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
    }
}

fn sample_message() -> ConversationMessage {
    ConversationMessage {
        id: "m-1".into(),
        content: "hi".into(),
        message_type: "text".into(),
        extra_metadata: json!({"k": "v"}),
        sender: "user".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
    }
}

#[test]
fn thread_to_summary_preserves_all_fields() {
    let summary = thread_to_summary(sample_thread());
    assert_eq!(summary.id, "t-1");
    assert_eq!(summary.title, "My thread");
    assert_eq!(summary.chat_id, Some(42));
    assert!(summary.is_active);
    assert_eq!(summary.message_count, 5);
    assert_eq!(summary.last_message_at, "2026-01-01T00:00:00Z");
    assert_eq!(summary.created_at, "2026-01-01T00:00:00Z");
}

#[test]
fn message_to_record_and_back_is_lossless() {
    let msg = sample_message();
    let record = message_to_record(msg.clone());
    assert_eq!(record.id, msg.id);
    assert_eq!(record.content, msg.content);
    assert_eq!(record.message_type, msg.message_type);
    assert_eq!(record.extra_metadata, msg.extra_metadata);
    assert_eq!(record.sender, msg.sender);
    assert_eq!(record.created_at, msg.created_at);

    let round_tripped = record_to_message(record);
    assert_eq!(round_tripped, msg);
}

#[test]
fn record_to_message_preserves_null_extra_metadata() {
    // Default Value::Null must pass through untouched so the downstream
    // storage layer sees the same "no metadata" signal it produced.
    let rec = ConversationMessageRecord {
        id: "m-2".into(),
        content: "x".into(),
        message_type: "text".into(),
        extra_metadata: Value::Null,
        sender: "agent".into(),
        created_at: "2026-01-02T00:00:00Z".into(),
    };
    let msg = record_to_message(rec);
    assert_eq!(msg.extra_metadata, Value::Null);
    assert_eq!(msg.sender, "agent");
}

// ── Title constants ───────────────────────────────────────────

#[test]
fn title_system_prompt_constrains_model_output_shape() {
    // The system prompt is shipped verbatim to the provider. Locking
    // in the trailing "no trailing punctuation" clause catches
    // accidental edits that would let the model emit trailing periods
    // that `sanitize_generated_title` would then silently strip.
    assert!(THREAD_TITLE_SYSTEM_PROMPT.contains("under 8 words"));
    assert!(THREAD_TITLE_SYSTEM_PROMPT.contains("No quotes"));
    assert!(THREAD_TITLE_SYSTEM_PROMPT.contains("No markdown"));
}

#[test]
fn title_log_prefix_is_grep_friendly_and_stable() {
    // The `[threads:title]` prefix is what CLAUDE.md's "debug logging"
    // rule asks contributors to grep for when debugging. It is part
    // of the observable contract — lock it down.
    assert_eq!(THREAD_TITLE_LOG_PREFIX, "[threads:title]");
}
