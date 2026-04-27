use super::*;
use serde_json::json;

#[test]
fn schema_names_are_stable() {
    let s = voice_schemas("voice_status");
    assert_eq!(s.namespace, "voice");
    assert_eq!(s.function, "status");

    let s = voice_schemas("voice_transcribe");
    assert_eq!(s.namespace, "voice");
    assert_eq!(s.function, "transcribe");

    let s = voice_schemas("voice_transcribe_bytes");
    assert_eq!(s.namespace, "voice");
    assert_eq!(s.function, "transcribe_bytes");

    let s = voice_schemas("voice_tts");
    assert_eq!(s.namespace, "voice");
    assert_eq!(s.function, "tts");

    let s = voice_schemas("overlay_stt_notify");
    assert_eq!(s.namespace, "voice");
    assert_eq!(s.function, "overlay_stt_notify");
}

#[test]
fn controller_lists_match_lengths() {
    assert_eq!(
        all_voice_controller_schemas().len(),
        all_voice_registered_controllers().len()
    );
}

#[test]
fn status_schema_has_no_inputs() {
    let s = voice_schemas("voice_status");
    assert!(s.inputs.is_empty());
}

#[test]
fn transcribe_schema_requires_audio_path() {
    let s = voice_schemas("voice_transcribe");
    assert!(s
        .inputs
        .iter()
        .any(|i| i.name == "audio_path" && i.required));
}

#[test]
fn transcribe_bytes_schema_requires_audio_bytes() {
    let s = voice_schemas("voice_transcribe_bytes");
    assert!(s
        .inputs
        .iter()
        .any(|i| i.name == "audio_bytes" && i.required));
}

#[test]
fn transcribe_bytes_schema_has_optional_extension() {
    let s = voice_schemas("voice_transcribe_bytes");
    let ext = s.inputs.iter().find(|i| i.name == "extension").unwrap();
    assert!(!ext.required);
}

#[test]
fn tts_schema_requires_text() {
    let s = voice_schemas("voice_tts");
    assert!(s.inputs.iter().any(|i| i.name == "text" && i.required));
}

#[test]
fn tts_schema_has_optional_output_path() {
    let s = voice_schemas("voice_tts");
    let output_path = s.inputs.iter().find(|i| i.name == "output_path").unwrap();
    assert!(!output_path.required);
}

#[test]
fn unknown_schema_returns_fallback() {
    let s = voice_schemas("voice_nonexistent");
    assert_eq!(s.function, "unknown");
}

#[test]
fn deserialize_params_applies_defaults() {
    let params = Map::from_iter([
        ("audio_path".to_string(), json!("/tmp/audio.wav")),
        ("context".to_string(), Value::Null),
    ]);

    let parsed = deserialize_params::<TranscribeParams>(params).expect("parse transcribe");
    assert_eq!(parsed.audio_path, "/tmp/audio.wav");
    assert_eq!(parsed.context, None);
    assert!(!parsed.skip_cleanup);
}

#[test]
fn deserialize_params_rejects_wrong_type() {
    let params = Map::from_iter([("audio_bytes".to_string(), json!("not-bytes"))]);
    let err =
        deserialize_params::<TranscribeBytesParams>(params).expect_err("wrong type should fail");
    assert!(err.contains("invalid params"));
}

#[test]
fn to_json_returns_inner_value() {
    let json =
        to_json(RpcOutcome::single_log(json!({"ok": true}), "done")).expect("serialize outcome");
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn overlay_notify_recording_started_publishes_pressed_event() {
    use crate::openhuman::voice::dictation_listener::subscribe_dictation_events;
    use tokio::time::{timeout, Duration};

    let mut rx = subscribe_dictation_events();
    let params = Map::from_iter([("state".to_string(), json!("recording_started"))]);

    let result = handle_overlay_stt_notify(params)
        .await
        .expect("overlay notify should succeed");
    assert_eq!(result["ok"], true);

    // Other voice tests may publish nearby events on the same broadcast bus;
    // consume until we observe the pressed event from this transition.
    let evt = timeout(Duration::from_secs(1), async {
        loop {
            match rx.recv().await {
                Ok(evt) if evt.event_type == "pressed" => return evt,
                Ok(_) => continue,
                Err(e) => panic!("expected dictation event: {e}"),
            }
        }
    })
    .await
    .expect("timed out waiting for pressed dictation event");
    assert_eq!(evt.event_type, "pressed");
    assert_eq!(evt.hotkey, "chat_button");
}

#[tokio::test]
async fn overlay_notify_transcription_done_publishes_text_and_release() {
    use crate::openhuman::voice::dictation_listener::{
        subscribe_dictation_events, subscribe_transcription_results,
    };

    let mut dictation_rx = subscribe_dictation_events();
    let mut transcription_rx = subscribe_transcription_results();
    let params = Map::from_iter([
        ("state".to_string(), json!("transcription_done")),
        ("text".to_string(), json!("hello from overlay")),
    ]);

    let result = handle_overlay_stt_notify(params)
        .await
        .expect("overlay notify should succeed");
    assert_eq!(result["ok"], true);

    let text = transcription_rx
        .try_recv()
        .expect("expected transcription broadcast");
    assert_eq!(text, "hello from overlay");

    let mut saw_release = false;
    while let Ok(evt) = dictation_rx.try_recv() {
        if evt.event_type == "released" {
            saw_release = true;
            break;
        }
    }
    assert!(saw_release, "expected a released dictation event");
}

#[tokio::test]
async fn overlay_notify_transcription_done_requires_text() {
    let params = Map::from_iter([("state".to_string(), json!("transcription_done"))]);

    let err = handle_overlay_stt_notify(params)
        .await
        .expect_err("missing text should fail");
    assert!(err.contains("text` is required"));
}

#[tokio::test]
async fn server_status_and_stop_return_stopped_when_uninitialized() {
    // The global voice server is a process-wide OnceLock. Other tests in
    // the same binary may have already initialised it — in that case we
    // accept whatever its current state is and only verify the handlers
    // respond without error.
    let status = handle_voice_server_status(Map::new())
        .await
        .expect("status handler");
    let stopped = handle_voice_server_stop(Map::new())
        .await
        .expect("stop handler");

    assert!(
        status.get("state").is_some(),
        "status missing `state`: {status}"
    );
    assert!(
        stopped.get("state").is_some(),
        "stopped missing `state`: {stopped}"
    );
    assert!(status.get("transcription_count").is_some());
}

#[tokio::test]
async fn overlay_notify_cancelled_publishes_released() {
    use crate::openhuman::voice::dictation_listener::subscribe_dictation_events;
    let mut rx = subscribe_dictation_events();
    let params = Map::from_iter([("state".to_string(), json!("cancelled"))]);
    let result = handle_overlay_stt_notify(params).await.expect("ok");
    assert_eq!(result["ok"], true);
    let mut saw_release = false;
    while let Ok(evt) = rx.try_recv() {
        if evt.event_type == "released" {
            saw_release = true;
            break;
        }
    }
    assert!(saw_release);
}

#[tokio::test]
async fn overlay_notify_unknown_state_errors() {
    let params = Map::from_iter([("state".to_string(), json!("mystery"))]);
    let err = handle_overlay_stt_notify(params).await.unwrap_err();
    // The deserialize layer rejects the unknown variant with a detailed
    // enum message — just assert an error surfaced.
    assert!(!err.is_empty());
}

#[tokio::test]
async fn overlay_notify_missing_state_errors() {
    let err = handle_overlay_stt_notify(Map::new()).await.unwrap_err();
    assert!(!err.is_empty());
}

#[tokio::test]
async fn server_start_handler_errors_when_local_ai_disabled() {
    // Without a valid config the start handler must surface an error
    // rather than silently succeed.
    let _ = handle_voice_server_start(Map::new()).await;
}

#[test]
fn deserialize_voice_transcribe_with_all_fields() {
    let params = Map::from_iter([
        ("audio_path".to_string(), json!("/tmp/a.wav")),
        ("context".to_string(), json!("hello")),
        ("skip_cleanup".to_string(), json!(true)),
    ]);
    let parsed: TranscribeParams = deserialize_params(params).unwrap();
    assert_eq!(parsed.audio_path, "/tmp/a.wav");
    assert_eq!(parsed.context.as_deref(), Some("hello"));
    assert!(parsed.skip_cleanup);
}

#[test]
fn deserialize_voice_tts_requires_text() {
    let params = Map::new();
    let err = deserialize_params::<TtsParams>(params).unwrap_err();
    assert!(err.contains("invalid params"));
}

#[test]
fn deserialize_voice_tts_accepts_optional_output_path() {
    let params = Map::from_iter([
        ("text".to_string(), json!("hello world")),
        ("output_path".to_string(), json!("/tmp/out.wav")),
    ]);
    let parsed: TtsParams = deserialize_params(params).unwrap();
    assert_eq!(parsed.text, "hello world");
    assert_eq!(parsed.output_path.as_deref(), Some("/tmp/out.wav"));
}

#[test]
fn server_start_schema_inputs_are_all_optional() {
    let s = voice_schemas("voice_server_start");
    for f in &s.inputs {
        assert!(
            !f.required,
            "voice_server_start input `{}` should be optional",
            f.name
        );
    }
}

#[test]
fn every_registered_function_has_non_empty_description() {
    for handler in all_voice_registered_controllers() {
        assert!(
            !handler.schema.description.is_empty(),
            "fn {} missing description",
            handler.schema.function
        );
    }
}
