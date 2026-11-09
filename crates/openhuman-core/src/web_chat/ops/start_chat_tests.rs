use super::*;
use crate::web_chat::GuardrailReason;

/// `From<StartChatError> for String` on the `Other` variant passes the
/// message through unchanged — every pre-existing `.to_string()`/`{err}`
/// consumer of the old `Result<String, String>` `start_chat` must see
/// identical text after the `StartChatError` migration.
#[test]
fn other_variant_converts_to_string_unchanged() {
    let error = StartChatError::Other("client_id is required".to_string());
    let message: String = error.into();
    assert_eq!(message, "client_id is required");
}

/// `From<StartChatError> for String` on `Guardrail` produces the
/// `GUARDRAIL:` sentinel followed by a JSON `GuardrailPayload` that decodes
/// back to the original verdict/score/reasons — this is the RPC-surface
/// encoding `channel_web_chat`'s `?` conversion (and any other
/// string-error caller) sees.
#[test]
fn guardrail_variant_converts_to_sentinel_plus_json_payload() {
    let error = StartChatError::Guardrail {
        verdict: "block".to_string(),
        score: 0.87,
        reasons: vec![GuardrailReason {
            code: "prompt_injection".to_string(),
            message: "detected embedded instruction override".to_string(),
        }],
    };
    let message: String = error.into();
    assert!(
        message.starts_with(GUARDRAIL_ERROR_PREFIX),
        "must start with the sentinel prefix, got: {message}"
    );

    let json_part = message.strip_prefix(GUARDRAIL_ERROR_PREFIX).unwrap();
    let payload: crate::web_chat::GuardrailPayload =
        serde_json::from_str(json_part).expect("payload must be valid JSON");
    assert_eq!(payload.verdict, "block");
    assert_eq!(payload.score, 0.87);
    assert_eq!(payload.reasons.len(), 1);
    assert_eq!(payload.reasons[0].code, "prompt_injection");
}

/// `Display` on `Guardrail` reuses the same human-readable copy a fresh
/// (non-error) rejection gets, keyed off the verdict string, so an existing
/// `.to_string()`/`{err}` consumer sees an actionable message rather than a
/// bare verdict/score dump.
#[test]
fn guardrail_display_uses_verdict_specific_user_message() {
    let blocked = StartChatError::Guardrail {
        verdict: "block".to_string(),
        score: 1.0,
        reasons: vec![],
    };
    let review_blocked = StartChatError::Guardrail {
        verdict: "review_blocked".to_string(),
        score: 0.5,
        reasons: vec![],
    };
    // Different verdicts must not collapse onto the same copy.
    assert_ne!(blocked.to_string(), review_blocked.to_string());
    assert!(!blocked.to_string().is_empty());
    assert!(!review_blocked.to_string().is_empty());
}

/// `From<&str>`/`From<String>` still build the plain `Other` variant, so
/// every internal `Err("...".to_string())` site that migrated to
/// `StartChatError` keeps compiling and behaving identically via `?`.
#[test]
fn plain_string_conversions_build_other_variant() {
    let from_owned: StartChatError = "thread_id is required".to_string().into();
    assert!(matches!(from_owned, StartChatError::Other(ref m) if m == "thread_id is required"));

    let from_borrowed: StartChatError = "message is required".into();
    assert!(matches!(from_borrowed, StartChatError::Other(ref m) if m == "message is required"));
}

#[test]
fn typed_media_is_valid_chat_input_without_caption_text() {
    let image = tinychannels_bus::ContentPart::Image {
        media: tinychannels_bus::MediaReference::default(),
        caption: None,
        alt_text: None,
    };
    let stored = crate::agent::queued_turn::StoredAttachmentInput {
        persisted_message_id: "user:row-1".into(),
        parts: vec![image.clone()],
    };
    assert!(submission_has_content("", Some(&stored), None));
    assert!(submission_has_content(
        "  ",
        None,
        Some(std::slice::from_ref(&image))
    ));
    assert!(!submission_has_content("  ", None, None));
    assert!(!submission_has_content(
        "",
        Some(&crate::agent::queued_turn::StoredAttachmentInput::default()),
        Some(&[]),
    ));
}

#[test]
fn collect_queue_rejects_typed_media_before_it_can_be_accepted_and_dropped() {
    assert!(validate_queue_mode_media(QueueMode::Collect, true, false, false).is_err());
    assert!(validate_queue_mode_media(QueueMode::Collect, false, true, false).is_err());
    assert!(validate_queue_mode_media(QueueMode::Collect, false, false, true).is_err());
    assert!(validate_queue_mode_media(QueueMode::Followup, false, false, true).is_ok());
}

#[test]
fn steer_queue_rejects_typed_media_that_cannot_survive_recovery() {
    assert!(validate_queue_mode_media(QueueMode::Steer, true, false, false).is_err());
    assert!(validate_queue_mode_media(QueueMode::Steer, false, true, false).is_err());
    assert!(validate_queue_mode_media(QueueMode::Steer, false, false, true).is_err());
    assert!(validate_queue_mode_media(QueueMode::Steer, false, false, false).is_ok());
    assert!(validate_queue_mode_media(QueueMode::Followup, false, false, true).is_ok());
}

#[tokio::test]
async fn voice_note_turn_id_is_bound_to_its_exact_user_row_before_admission() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::threads::store::{blocking, ConversationMessage, CreateConversationThread};
    use crate::tools::toolpacks::ToolGroups;
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let config = crate::config::Config {
        workspace_dir: workspace.clone(),
        ..crate::config::Config::default()
    };
    let ctx = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(config, DomainSet::kernel(), ToolGroups::none()),
    ));
    blocking::ensure_thread(
        ctx.clone(),
        workspace.clone(),
        CreateConversationThread {
            id: "voice-thread".into(),
            title: "Voice note".into(),
            created_at: "2026-10-20T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .unwrap();
    for (id, note_id) in [("voice-row", "note-1"), ("plain-row", "note-2")] {
        blocking::append_message(
            ctx.clone(),
            workspace.clone(),
            "voice-thread".into(),
            ConversationMessage {
                id: id.into(),
                content: "same transcription".into(),
                message_type: "text".into(),
                extra_metadata: json!({"voiceNote": {"noteId": note_id}}),
                sender: "user".into(),
                created_at: "2026-10-20T00:00:01Z".into(),
            },
        )
        .await
        .unwrap();
    }

    bind_voice_note_request_id(
        &ctx,
        "voice-thread",
        "same transcription",
        "note-1",
        "request-1",
    )
    .await
    .unwrap();
    let conflicting = bind_voice_note_request_id(
        &ctx,
        "voice-thread",
        "same transcription",
        "note-1",
        "request-2",
    )
    .await;
    assert!(
        conflicting.is_err(),
        "a replay cannot overwrite the row's turn identity"
    );
    let rows = blocking::get_messages(ctx, workspace, "voice-thread".into())
        .await
        .unwrap();
    let voice_row = rows.iter().find(|row| row.id == "voice-row").unwrap();
    let plain_row = rows.iter().find(|row| row.id == "plain-row").unwrap();
    assert_eq!(voice_row.extra_metadata["requestId"], "request-1");
    assert!(plain_row.extra_metadata.get("requestId").is_none());
}
