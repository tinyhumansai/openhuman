use super::*;

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

// NOTE: the title_log_fingerprint / collapse_whitespace copies were removed
// here (plan.md §2.1) — threads/title.rs (the owning module) already covers
// these functions with equivalent cases; the lowercase-hex assertion was
// folded into title.rs so no coverage was lost.

// ── build_title_prompt ────────────────────────────────────────

// ── build_title_request: the outgoing wire model (#5637) ──────

#[test]
fn build_title_request_sends_no_per_request_model_override() {
    // The regression guard for #5637. `ModelRequest::model` is a per-request
    // override that the managed backend resolves verbatim, so anything set
    // here replaces the model the `summarization` provider already resolved.
    // It used to be `"hint:summarize"` — a string no hint-alias table in the
    // tree defines — which reached the backend as a literal model id and was
    // answered `400 Model 'hint:summarize' is not available` on every call,
    // for four months.
    //
    // Asserted as `None` rather than as some expected id on purpose: leaving
    // it unset is the only form that is correct for every branch of
    // `create_chat_model` (managed backend, Claude Agent SDK, Claude Code,
    // local runtime, BYOK cloud slug). Pinning a concrete managed tier here
    // would fix the managed route and break the other four.
    let request = crate::openhuman::threads::title::build_title_request("hi there", "hello back");

    assert!(
        request.model.is_none(),
        "the title request must carry no model override so the resolved \
         `summarization` provider decides; got {:?}",
        request.model,
    );
}

#[test]
fn build_title_request_carries_the_system_prompt_and_rendered_user_prompt() {
    // Pins the rest of the request so the `model` assertion above cannot be
    // satisfied by an empty or malformed request.
    use tinyinference::message::Message;

    let request = crate::openhuman::threads::title::build_title_request("hi there", "hello back");

    assert_eq!(request.messages.len(), 2, "system + user");
    assert!(
        matches!(request.messages[0], Message::System(_)),
        "first message must be the system prompt, got {:?}",
        request.messages[0],
    );
    assert_eq!(request.messages[0].text(), THREAD_TITLE_SYSTEM_PROMPT,);
    assert!(
        matches!(request.messages[1], Message::User(_)),
        "second message must be the user prompt, got {:?}",
        request.messages[1],
    );
    assert_eq!(
        request.messages[1].text(),
        build_title_prompt("hi there", "hello back"),
    );
}

#[test]
fn build_title_prompt_renders_user_and_assistant_sections_in_order() {
    let prompt = build_title_prompt("hi there", "hello back");
    assert_eq!(
        prompt,
        "First user message:\nhi there\n\nAssistant reply:\nhello back\n\nReturn the best thread name."
    );
}

// NOTE: the sanitize_generated_title / title_from_user_message copies were
// removed here (plan.md §2.1) — threads/title.rs (the owning module) already
// covers these functions with equivalent cases (title shaping, filler removal,
// first non-empty line, empty→None, and the char-safe length ceiling incl.
// multibyte input).

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
    assert_eq!(summary.labels, vec!["general".to_string()]);
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
    // The system prompt is shipped verbatim to the provider. Locking in the
    // length clause catches accidental edits that would let the model emit a
    // sentence-shaped title that `sanitize_generated_title` would then have to
    // rewrite silently.
    assert!(THREAD_TITLE_SYSTEM_PROMPT.contains("at most 3 words"));
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

#[tokio::test]
async fn message_append_returns_typed_not_found_for_stale_thread() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", workspace.path());
    let thread_id = "thread-missing";

    let err = message_append(AppendConversationMessageRequest {
        thread_id: thread_id.to_string(),
        message: ConversationMessageRecord {
            id: "msg-1".to_string(),
            content: "hello".to_string(),
            message_type: "text".to_string(),
            extra_metadata: Value::Null,
            sender: "user".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        },
    })
    .await
    .expect_err("missing thread must return a typed not-found error");

    assert_eq!(
        err,
        ThreadsError::NotFound {
            thread_id: thread_id.to_string()
        }
    );
    assert_eq!(err.to_string(), "thread thread-missing not found");
}

#[tokio::test]
async fn generate_title_returns_typed_not_found_for_stale_thread() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", workspace.path());
    let thread_id = "thread-missing";

    let err = thread_generate_title(GenerateConversationThreadTitleRequest {
        thread_id: thread_id.to_string(),
        assistant_message: None,
    })
    .await
    .expect_err("missing thread must return a typed not-found error");

    assert_eq!(
        err,
        ThreadsError::NotFound {
            thread_id: thread_id.to_string()
        }
    );
    assert_eq!(err.to_string(), "thread thread-missing not found");
}

#[tokio::test]
async fn generate_title_leaves_custom_title_unchanged() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", workspace.path());
    let thread_id = "thread-custom";
    create_thread_with_title(&workspace, thread_id, "Already named").await;
    let dir = crate::openhuman::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir;
    conversations_store::append_message(
        dir,
        thread_id,
        ConversationMessage {
            id: "msg-1".into(),
            content: "Please summarize my notes".into(),
            message_type: "text".into(),
            extra_metadata: Value::Null,
            sender: "user".into(),
            created_at: "2026-01-01T00:01:00Z".into(),
        },
    )
    .unwrap();

    let outcome = thread_generate_title(GenerateConversationThreadTitleRequest {
        thread_id: thread_id.to_string(),
        assistant_message: None,
    })
    .await
    .expect("generate title");

    assert_eq!(
        outcome.value.data.as_ref().unwrap().title,
        "Already named",
        "non-placeholder titles must not be replaced"
    );
}

#[tokio::test]
async fn generate_title_returns_existing_title_when_no_user_message_exists() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", workspace.path());
    let thread_id = "thread-no-user";
    create_thread_with_title(&workspace, thread_id, "Chat Jan 1 1:00 AM").await;

    let outcome = thread_generate_title(GenerateConversationThreadTitleRequest {
        thread_id: thread_id.to_string(),
        assistant_message: Some("assistant reply".into()),
    })
    .await
    .expect("generate title");

    assert_eq!(
        outcome.value.data.as_ref().unwrap().title,
        "Chat Jan 1 1:00 AM"
    );
}

#[tokio::test]
async fn generate_title_falls_back_to_first_user_message_when_assistant_missing() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", workspace.path());
    let thread_id = "thread-fallback";
    create_thread_with_title(&workspace, thread_id, "Chat Jan 1 1:00 AM").await;
    let dir = crate::openhuman::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir;
    let user_message = "Please summarize the latest five email threads for me.";
    conversations_store::append_message(
        dir,
        thread_id,
        ConversationMessage {
            id: "msg-1".into(),
            content: user_message.into(),
            message_type: "text".into(),
            extra_metadata: Value::Null,
            sender: "user".into(),
            created_at: "2026-01-01T00:01:00Z".into(),
        },
    )
    .unwrap();

    let outcome = thread_generate_title(GenerateConversationThreadTitleRequest {
        thread_id: thread_id.to_string(),
        assistant_message: None,
    })
    .await
    .expect("generate title");

    assert_eq!(
        outcome.value.data.as_ref().unwrap().title,
        title_from_user_message(user_message).unwrap()
    );
}

#[tokio::test]
async fn thread_delete_removes_persisted_turn_state_snapshot() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", workspace.path());
    let thread_id = "thread-delete";
    create_thread_with_title(&workspace, thread_id, "Chat Jan 1 1:00 AM").await;
    let dir = crate::openhuman::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir;

    let snapshot = TurnState::started(thread_id, "req-1", 4, "2026-01-01T00:00:00Z");
    turn_state::store::put(dir.clone(), &snapshot).expect("put snapshot");
    assert!(turn_state::store::get(dir, thread_id).unwrap().is_some());

    // Queue a finished background sub-agent result for this thread; deleting the
    // thread must discard it so it's never delivered into a dead thread.
    use crate::openhuman::agent::orchestration::background_completions as bg;
    bg::record_completion(
        "sess-del",
        "sub-del-1",
        "researcher",
        "result",
        Some(thread_id.to_string()),
    );
    assert_eq!(bg::pending_count("sess-del"), 1);

    thread_delete(DeleteConversationThreadRequest {
        thread_id: thread_id.to_string(),
        deleted_at: "2026-01-01T00:02:00Z".into(),
    })
    .await
    .expect("delete thread");

    assert_eq!(
        bg::pending_count("sess-del"),
        0,
        "queued completion for the deleted thread should be discarded"
    );

    let turn_state = turn_state_get(GetTurnStateRequest {
        thread_id: thread_id.to_string(),
    })
    .await
    .expect("turn_state_get");
    assert!(turn_state.value.data.unwrap().turn_state.is_none());
}

#[tokio::test]
async fn threads_purge_removes_valid_and_corrupted_turn_state_files() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", workspace.path());
    create_thread_with_title(&workspace, "thread-a", "Chat Jan 1 1:00 AM").await;
    create_thread_with_title(&workspace, "thread-b", "Chat Jan 1 1:01 AM").await;
    let dir = crate::openhuman::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir;

    let snapshot = TurnState::started("thread-a", "req-1", 4, "2026-01-01T00:00:00Z");
    turn_state::store::put(dir.clone(), &snapshot).expect("put snapshot");

    let turn_state_dir = dir.join("memory").join("conversations").join("turn_states");
    std::fs::create_dir_all(&turn_state_dir).unwrap();
    std::fs::write(
        turn_state_dir.join("corrupted.json"),
        "{ definitely not json",
    )
    .unwrap();

    // Queue background sub-agent results across sessions; a full purge must wipe
    // them all since no parent thread survives.
    use crate::openhuman::agent::orchestration::background_completions as bg;
    bg::record_completion(
        "sess-p1",
        "sub-p1",
        "researcher",
        "x",
        Some("thread-a".into()),
    );
    bg::record_completion(
        "sess-p2",
        "sub-p2",
        "researcher",
        "y",
        Some("thread-b".into()),
    );

    threads_purge(EmptyRequest {})
        .await
        .expect("purge threads should also clear snapshots");

    assert!(!bg::has_pending("sess-p1"));
    assert!(!bg::has_pending("sess-p2"));

    if turn_state_dir.exists() {
        let remaining_json: Vec<_> = std::fs::read_dir(&turn_state_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("json"))
            .collect();
        assert!(remaining_json.is_empty(), "expected no snapshot json files");
    }
}

#[tokio::test]
async fn turn_state_clear_reports_false_when_snapshot_is_absent() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", workspace.path());

    let outcome = turn_state_clear(ClearTurnStateRequest {
        thread_id: "missing-thread".into(),
    })
    .await
    .expect("turn_state_clear");

    assert!(!outcome.value.data.unwrap().cleared);
}

// ── thread_update_title ───────────────────────────────────────

#[tokio::test]
async fn thread_update_title_rejects_empty_title() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", workspace.path());

    let err = thread_update_title(
        crate::openhuman::memory::UpdateConversationThreadTitleRequest {
            thread_id: "t-1".to_string(),
            title: "".to_string(),
        },
    )
    .await
    .expect_err("empty title must be rejected");

    assert!(
        err.contains("must not be empty"),
        "expected empty-title error, got: {err}"
    );
}

#[tokio::test]
async fn thread_update_title_rejects_whitespace_only_title() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", workspace.path());

    let err = thread_update_title(
        crate::openhuman::memory::UpdateConversationThreadTitleRequest {
            thread_id: "t-1".to_string(),
            title: "   ".to_string(),
        },
    )
    .await
    .expect_err("whitespace-only title must be rejected");

    assert!(
        err.contains("must not be empty"),
        "expected empty-title error, got: {err}"
    );
}
