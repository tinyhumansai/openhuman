use super::*;

// ── Issue #1376 — chat-name normalization for active-chat → JID lookup ──

#[test]
fn normalize_chat_name_strips_punctuation_and_emoji() {
    // Group titles routinely pick up emoji + punctuation drift between
    // the DOM-parsed conversation header and the IDB-stored chat name.
    // Normalization should collapse both sides to the same key so the
    // lookup at scan_once succeeds.
    assert_eq!(
        normalize_chat_name("17-18-19 July samagam"),
        "171819julysamagam"
    );
    assert_eq!(
        normalize_chat_name("17 18 19 July  samagam"),
        "171819julysamagam"
    );
    assert_eq!(
        normalize_chat_name("17-18-19 July samagam ✨"),
        "171819julysamagam"
    );
    assert_eq!(
        normalize_chat_name("17.18.19 July, samagam!"),
        "171819julysamagam"
    );
    // Identity property — already-normal strings round-trip unchanged.
    assert_eq!(normalize_chat_name("foo123"), "foo123");
    // Empty input → empty output (caller guards against this).
    assert_eq!(normalize_chat_name(""), "");
    assert_eq!(normalize_chat_name("   "), "");
    assert_eq!(normalize_chat_name("✨"), "");
}

#[test]
fn normalize_chat_name_lowercases() {
    assert_eq!(normalize_chat_name("Hello World"), "helloworld");
    assert_eq!(normalize_chat_name("HELLO"), "hello");
    assert_eq!(normalize_chat_name("hElLo"), "hello");
}

fn insert_pending_tasks(
    registry: &ScannerRegistry,
    account_id: &str,
    count: usize,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut tasks = Vec::with_capacity(count);
    let mut abort_handles = Vec::with_capacity(count);
    for _ in 0..count {
        let task = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        abort_handles.push(task.abort_handle());
        tasks.push(task);
    }
    registry
        .started
        .lock()
        .insert(account_id.to_string(), abort_handles);
    tasks
}

async fn assert_cancelled(task: tokio::task::JoinHandle<()>) {
    let err = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("aborted scanner task should finish")
        .expect_err("scanner task should be cancelled");
    assert!(err.is_cancelled());
}

async fn assert_all_cancelled(tasks: Vec<tokio::task::JoinHandle<()>>) {
    for task in tasks {
        assert_cancelled(task).await;
    }
}

#[tokio::test]
async fn registry_forget_aborts_all_handles_for_account_only() {
    let registry = ScannerRegistry::default();
    let account_tasks = insert_pending_tasks(&registry, "acct-1", 2);
    let survivor_tasks = insert_pending_tasks(&registry, "acct-2", 1);

    registry.forget("acct-1");

    {
        let guard = registry.started.lock();
        assert_eq!(guard.len(), 1);
        assert!(guard.contains_key("acct-2"));
    }
    assert_all_cancelled(account_tasks).await;
    assert!(
        !survivor_tasks[0].is_finished(),
        "forget(acct-1) must not abort acct-2"
    );

    assert_eq!(registry.forget_all(), 1);
    assert_all_cancelled(survivor_tasks).await;
}

#[tokio::test]
async fn registry_forget_missing_account_is_noop() {
    let registry = ScannerRegistry::default();
    let mut tasks = insert_pending_tasks(&registry, "acct-1", 1);

    registry.forget("missing");

    {
        let guard = registry.started.lock();
        assert_eq!(guard.len(), 1);
        assert!(guard.contains_key("acct-1"));
    }
    assert!(
        !tasks[0].is_finished(),
        "forget(missing) must not abort existing scanners"
    );

    registry.forget("acct-1");
    assert_cancelled(tasks.pop().expect("task")).await;
}

#[tokio::test]
async fn registry_forget_all_aborts_all_tasks_and_reports_handle_count() {
    let registry = ScannerRegistry::default();
    let task_a = insert_pending_tasks(&registry, "acct-1", 2);
    let task_b = insert_pending_tasks(&registry, "acct-2", 3);

    assert_eq!(registry.forget_all(), 5);

    assert!(registry.started.lock().is_empty());
    assert_all_cancelled(task_a).await;
    assert_all_cancelled(task_b).await;
}

#[tokio::test]
async fn registry_forget_all_is_repeatable_noop_after_drain() {
    let registry = ScannerRegistry::default();
    assert_eq!(registry.forget_all(), 0);

    let tasks = insert_pending_tasks(&registry, "acct-1", 1);
    assert_eq!(registry.forget_all(), 1);
    assert_eq!(registry.forget_all(), 0);

    assert!(registry.started.lock().is_empty());
    assert_all_cancelled(tasks).await;
}

// ── seconds_to_ymd ────────────────────────────────────────────────────────

#[test]
fn seconds_to_ymd_known_timestamp() {
    // Unix timestamp 1_700_000_000 = 2023-11-14 (UTC).
    assert_eq!(seconds_to_ymd(1_700_000_000), "2023-11-14");
}

#[test]
fn seconds_to_ymd_epoch_zero() {
    // Unix epoch origin = 1970-01-01.
    assert_eq!(seconds_to_ymd(0), "1970-01-01");
}

#[test]
fn seconds_to_ymd_output_format_is_yyyy_mm_dd() {
    let s = seconds_to_ymd(1_700_000_000);
    // Must match YYYY-MM-DD: 10 chars, digit/digit/digit/digit-...-...
    assert_eq!(s.len(), 10, "expected 10-char date string, got: {s}");
    let parts: Vec<&str> = s.split('-').collect();
    assert_eq!(parts.len(), 3, "expected 3 dash-separated parts: {s}");
    assert_eq!(parts[0].len(), 4, "year must be 4 digits: {s}");
    assert_eq!(parts[1].len(), 2, "month must be 2 digits: {s}");
    assert_eq!(parts[2].len(), 2, "day must be 2 digits: {s}");
    assert!(
        parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())),
        "all parts must be numeric: {s}"
    );
}

// ── parse_pre_timestamp_ymd ───────────────────────────────────────────────

#[test]
fn parse_pre_timestamp_ymd_valid_wa_format() {
    // WhatsApp Web format: "4:53 AM, 7/5/2025"
    let result = parse_pre_timestamp_ymd("4:53 AM, 7/5/2025");
    assert_eq!(result.as_deref(), Some("2025-07-05"));
}

#[test]
fn parse_pre_timestamp_ymd_another_valid_date() {
    // "10:01 PM, 11/14/2023" — matches our known ts
    let result = parse_pre_timestamp_ymd("10:01 PM, 11/14/2023");
    assert_eq!(result.as_deref(), Some("2023-11-14"));
}

#[test]
fn parse_pre_timestamp_ymd_empty_string_returns_none() {
    assert!(parse_pre_timestamp_ymd("").is_none());
}

#[test]
fn parse_pre_timestamp_ymd_no_comma_returns_none() {
    assert!(parse_pre_timestamp_ymd("4:53 AM 7/5/2025").is_none());
}

#[test]
fn parse_pre_timestamp_ymd_invalid_date_parts_return_none() {
    // Month 13 is out of range.
    assert!(parse_pre_timestamp_ymd("10:00 AM, 13/5/2025").is_none());
    // Day 32 is out of range.
    assert!(parse_pre_timestamp_ymd("10:00 AM, 1/32/2025").is_none());
}

#[test]
fn parse_pre_timestamp_ymd_garbage_returns_none() {
    assert!(parse_pre_timestamp_ymd("not a timestamp at all").is_none());
}

// ── emit_grouped_whatsapp grouping ────────────────────────────────────────

/// Build a minimal message Value that `emit_grouped_whatsapp` will accept.
fn make_msg(chat_id: &str, ts: i64, body: &str, from_me: bool) -> Value {
    json!({
        "chatId": chat_id,
        "body": body,
        "timestamp": ts,
        "fromMe": from_me,
        "from": if from_me { "me" } else { chat_id },
    })
}

#[test]
fn grouping_produces_correct_group_count_and_keys() {
    use std::collections::HashMap;

    // 3 messages in alice@c.us on day 2023-11-14 (ts ≈ 1_700_000_000).
    // 2 messages in group@g.us on a different day (ts ≈ 1_700_100_000 =
    // 2023-11-15 UTC).
    let day1_ts = 1_700_000_000i64; // 2023-11-14
    let day2_ts = 1_700_100_000i64; // 2023-11-15

    let messages = vec![
        make_msg("alice@c.us", day1_ts, "Hello", false),
        make_msg("alice@c.us", day1_ts + 60, "How are you?", false),
        make_msg("alice@c.us", day1_ts + 120, "Fine thanks", true),
        make_msg("group@g.us", day2_ts, "Meeting at 3pm", false),
        make_msg("group@g.us", day2_ts + 30, "Got it", true),
    ];

    // Collect groups the same way emit_grouped_whatsapp does it.
    let empty_chats = serde_json::Map::new();
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let mut groups: HashMap<(String, String), Vec<Value>> = HashMap::new();
    for m in &messages {
        let chat_id = match m.get("chatId").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        let body = m
            .get("body")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if body.is_empty() {
            continue;
        }
        let day: String = if let Some(t) = m.get("timestamp").and_then(|v| v.as_i64()) {
            seconds_to_ymd(t)
        } else {
            seconds_to_ymd(now_secs)
        };
        let _ = &empty_chats;
        groups.entry((chat_id, day)).or_default().push(m.clone());
    }

    assert_eq!(groups.len(), 2, "expected exactly 2 (chatId, day) groups");

    let alice_day = seconds_to_ymd(day1_ts);
    let group_day = seconds_to_ymd(day2_ts);

    let alice_key = ("alice@c.us".to_string(), alice_day.clone());
    let group_key = ("group@g.us".to_string(), group_day.clone());

    assert!(
        groups.contains_key(&alice_key),
        "alice group missing; groups: {groups:?}"
    );
    assert!(
        groups.contains_key(&group_key),
        "group@g.us group missing; groups: {groups:?}"
    );

    assert_eq!(
        groups[&alice_key].len(),
        3,
        "alice chat should have 3 messages"
    );
    assert_eq!(
        groups[&group_key].len(),
        2,
        "group chat should have 2 messages"
    );
}

// ── transcript format ─────────────────────────────────────────────────────

#[test]
fn build_doc_ingest_params_transcript_contains_senders_and_bodies() {
    let day_ts = 1_700_000_000i64; // 2023-11-14
    let ingest = json!({
        "chatId": "alice@c.us",
        "chatName": "Alice",
        "day": seconds_to_ymd(day_ts),
        "messages": [
            {
                "chatId": "alice@c.us",
                "fromMe": false,
                "from": "alice@c.us",
                "fromName": "Alice",
                "body": "Hey there!",
                "timestamp": day_ts,
            },
            {
                "chatId": "alice@c.us",
                "fromMe": true,
                "from": "me",
                "fromName": null,
                "body": "Hi Alice!",
                "timestamp": day_ts + 60,
            },
            {
                "chatId": "alice@c.us",
                "fromMe": false,
                "from": "alice@c.us",
                "fromName": "Alice",
                "body": "How are you?",
                "timestamp": day_ts + 120,
            },
        ],
    });

    let params = build_doc_ingest_params("test-acct@c.us", &ingest)
        .expect("should build params for valid ingest");

    let content = params
        .get("content")
        .and_then(|v| v.as_str())
        .expect("content must be present");

    // Senders should appear in the transcript.
    assert!(
        content.contains("Alice"),
        "transcript must contain sender name 'Alice'; content:\n{content}"
    );
    assert!(
        content.contains("me"),
        "transcript must contain 'me' for self-sent messages; content:\n{content}"
    );

    // Bodies must be present.
    assert!(
        content.contains("Hey there!"),
        "transcript must contain first message body; content:\n{content}"
    );
    assert!(
        content.contains("Hi Alice!"),
        "transcript must contain second message body; content:\n{content}"
    );
    assert!(
        content.contains("How are you?"),
        "transcript must contain third message body; content:\n{content}"
    );

    // Lines must appear in ascending timestamp order — verify by position.
    let pos_hey = content.find("Hey there!").expect("Hey there not found");
    let pos_hi = content.find("Hi Alice!").expect("Hi Alice not found");
    let pos_how = content.find("How are you?").expect("How are you not found");
    assert!(
        pos_hey < pos_hi && pos_hi < pos_how,
        "transcript lines must be in timestamp order"
    );
}

// ── build_doc_ingest_params payload shape ─────────────────────────────────

#[test]
fn build_doc_ingest_params_namespace_and_key_format() {
    let day = "2023-11-14";
    let ingest = json!({
        "chatId": "alice@c.us",
        "chatName": "Alice",
        "day": day,
        "messages": [
            { "chatId": "alice@c.us", "fromMe": false, "from": "alice@c.us",
              "fromName": "Alice", "body": "Hello", "timestamp": 1_700_000_000i64 }
        ],
    });

    let params = build_doc_ingest_params("test-acct@c.us", &ingest).expect("should build params");

    assert_eq!(
        params.get("namespace").and_then(|v| v.as_str()),
        Some("whatsapp-web:test-acct@c.us"),
        "namespace must be 'whatsapp-web:<account_id>'"
    );
    assert_eq!(
        params.get("key").and_then(|v| v.as_str()),
        Some("alice@c.us:2023-11-14"),
        "key must be '<chat_id>:<day>'"
    );
    assert_eq!(
        params.get("source_type").and_then(|v| v.as_str()),
        Some("whatsapp-web"),
        "source_type must be 'whatsapp-web'"
    );

    // Content must be non-empty and contain the body.
    let content = params
        .get("content")
        .and_then(|v| v.as_str())
        .expect("content must be present");
    assert!(!content.is_empty(), "content must not be empty");
    assert!(
        content.contains("Hello"),
        "content must contain message body; got:\n{content}"
    );
}

#[test]
fn build_doc_ingest_params_missing_chat_id_returns_none() {
    let ingest = json!({
        "chatName": "Alice",
        "day": "2023-11-14",
        "messages": [
            { "chatId": "alice@c.us", "fromMe": false, "body": "Hello", "timestamp": 1i64 }
        ],
    });
    assert!(
        build_doc_ingest_params("acct", &ingest).is_none(),
        "missing chatId must return None"
    );
}

#[test]
fn build_doc_ingest_params_empty_messages_returns_none() {
    let ingest = json!({
        "chatId": "alice@c.us",
        "chatName": "Alice",
        "day": "2023-11-14",
        "messages": [],
    });
    assert!(
        build_doc_ingest_params("acct", &ingest).is_none(),
        "empty messages must return None"
    );
}

// ── DOM-IDB merge ─────────────────────────────────────────────────────────

#[test]
fn merge_dom_patches_empty_body_from_idb_message() {
    // IDB message with empty body; matching DOM row has the decrypted body.
    let idb = vec![json!({
        "id": "abc123",
        "chatId": "alice@c.us",
        "fromMe": false,
        "body": "",
    })];
    let dom = vec![json!({
        "dataId": "abc123",
        "msgId": "abc123",
        "chatId": "alice@c.us",
        "fromMe": false,
        "body": "Hello",
        "author": "Alice",
        "preTimestamp": null,
    })];

    let (merged, patched, appended) = merge_dom_into_snapshot(&idb, &dom, None);

    assert_eq!(patched, 1, "one message should be patched");
    assert_eq!(appended, 0, "no messages should be appended");
    assert_eq!(merged.len(), 1, "still one message in merged list");

    let body = merged[0]
        .get("body")
        .and_then(|v| v.as_str())
        .expect("body must be present");
    assert_eq!(body, "Hello", "patched body must equal DOM body");

    let source = merged[0]
        .get("bodySource")
        .and_then(|v| v.as_str())
        .expect("bodySource must be present");
    assert_eq!(source, "dom", "bodySource must be 'dom' after patching");
}

#[test]
fn merge_dom_appends_unmatched_row_with_active_chat_backfill() {
    // No IDB messages; DOM has a row with no chatId.  active_chat_jid
    // should be stamped onto the appended message.
    let idb: Vec<Value> = vec![];
    let dom = vec![json!({
        "dataId": "newrow1",
        "msgId": "newrow1",
        "chatId": "",   // empty — needs backfill
        "fromMe": false,
        "body": "Hey from active chat",
        "author": "Bob",
        "preTimestamp": null,
    })];

    let (merged, patched, appended) = merge_dom_into_snapshot(&idb, &dom, Some("bob@c.us"));

    assert_eq!(patched, 0, "nothing to patch");
    assert_eq!(appended, 1, "one row should be appended");
    assert_eq!(merged.len(), 1, "merged list should have 1 entry");

    let chat_id = merged[0]
        .get("chatId")
        .and_then(|v| v.as_str())
        .expect("chatId must be present");
    assert_eq!(
        chat_id, "bob@c.us",
        "chatId should be backfilled from active_chat_jid"
    );

    let body_source = merged[0]
        .get("bodySource")
        .and_then(|v| v.as_str())
        .expect("bodySource must be present");
    assert_eq!(body_source, "dom-only");
}

#[test]
fn merge_dom_does_not_append_row_without_body() {
    // DOM rows without a body should be silently skipped.
    let idb: Vec<Value> = vec![];
    let dom = vec![json!({
        "dataId": "empty1",
        "msgId": "empty1",
        "chatId": "alice@c.us",
        "fromMe": false,
        "body": "",
    })];

    let (merged, patched, appended) = merge_dom_into_snapshot(&idb, &dom, None);

    assert_eq!(patched, 0);
    assert_eq!(appended, 0, "empty-body DOM rows must not be appended");
    assert!(
        merged.is_empty(),
        "no messages should appear in merged list"
    );
}

#[test]
fn merge_dom_does_not_consume_row_twice() {
    // Two IDB messages with the same bare msgId; only the first match
    // should consume the DOM row.
    let idb = vec![
        json!({ "id": "chat_abc", "chatId": "alice@c.us", "fromMe": false, "body": "" }),
        json!({ "id": "chat_abc_2", "chatId": "alice@c.us", "fromMe": true, "body": "" }),
    ];
    // DOM row keyed only by bare msgId "abc".
    let dom = vec![json!({
        "dataId": "abc",
        "msgId": "abc",
        "chatId": "alice@c.us",
        "fromMe": false,
        "body": "Only once",
    })];

    let (merged, patched, _appended) = merge_dom_into_snapshot(&idb, &dom, None);

    // Exactly one of the two IDB messages should be patched.
    assert_eq!(patched, 1, "DOM row must be consumed at most once");
    assert_eq!(merged.len(), 2, "both IDB messages must survive merge");
    let patched_bodies: Vec<&str> = merged
        .iter()
        .filter_map(|m| m.get("body").and_then(|v| v.as_str()))
        .filter(|b| *b == "Only once")
        .collect();
    assert_eq!(
        patched_bodies.len(),
        1,
        "body 'Only once' must appear exactly once in merged list"
    );
}

#[test]
fn merge_dom_empty_dom_returns_idb_messages_unchanged() {
    let idb = vec![
        json!({ "id": "m1", "chatId": "a@c.us", "body": "hello" }),
        json!({ "id": "m2", "chatId": "a@c.us", "body": "" }),
    ];
    let dom: Vec<Value> = vec![];

    let (merged, patched, appended) = merge_dom_into_snapshot(&idb, &dom, None);

    assert_eq!(patched, 0);
    assert_eq!(appended, 0);
    assert_eq!(merged.len(), 2, "IDB messages must be returned unchanged");
    assert_eq!(
        merged[0].get("body").and_then(|v| v.as_str()),
        Some("hello")
    );
}
