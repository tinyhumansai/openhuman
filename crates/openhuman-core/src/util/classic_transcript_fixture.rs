//! Test fixtures: classic `session_raw/{stem}.jsonl` transcripts, the file
//! layout releases before the storage ports wrote.
//!
//! TinyAgents no longer writes or reads those files; the one-time import and
//! `legacy::rewrite_legacy_transcripts` are the only code that touches them.
//! Tests that need such a file on disk (to exercise the import or the rewrite)
//! build it here. This file depends on `serde_json` and `tinyagents_session`
//! only, so integration tests include it with `#[path]`.

#![allow(dead_code)]

use std::path::Path;

use serde_json::{json, Value};
use tinyagents_session::transcript::{TranscriptMessage, TranscriptMeta};

/// The `{"_meta": ...}` header line of a classic transcript.
pub fn meta_line(meta: &TranscriptMeta) -> Value {
    let mut payload = json!({
        "version": 1,
        "agent": meta.agent_name,
        "dispatcher": meta.dispatcher,
        "created": meta.created,
        "updated": meta.updated,
        "turn_count": meta.turn_count,
        "input_tokens": meta.input_tokens,
        "output_tokens": meta.output_tokens,
        "cached_input_tokens": meta.cached_input_tokens,
        "charged_amount_usd": meta.charged_amount_usd,
    });
    let object = payload.as_object_mut().expect("meta payload is an object");
    for (key, value) in [
        ("agent_id", &meta.agent_id),
        ("agent_type", &meta.agent_type),
        ("provider", &meta.provider),
        ("model", &meta.model),
        ("thread_id", &meta.thread_id),
        ("task_id", &meta.task_id),
        ("session_id", &meta.session_id),
        ("parent_session_id", &meta.parent_session_id),
    ] {
        if let Some(value) = value {
            object.insert(key.to_string(), Value::String(value.clone()));
        }
    }
    if let Some(count) = meta.prefix_message_count {
        object.insert("prefix_message_count".into(), json!(count));
    }
    json!({ "_meta": payload })
}

/// The text of a classic transcript: the meta line, then one line per message.
pub fn classic_jsonl(messages: &[TranscriptMessage], meta: &TranscriptMeta) -> String {
    let mut text = meta_line(meta).to_string();
    text.push('\n');
    for message in messages {
        text.push_str(&serde_json::to_string(message).expect("serialise a transcript row"));
        text.push('\n');
    }
    text
}

/// Writes a classic transcript file, creating its directory.
pub fn write_classic(path: &Path, messages: &[TranscriptMessage], meta: &TranscriptMeta) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create the transcript directory");
    }
    std::fs::write(path, classic_jsonl(messages, meta)).expect("write the classic transcript");
}

/// The message rows of a classic transcript (the `_meta` header skipped), as
/// JSON objects with at least `role` and `content`.
pub fn read_classic_rows(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .expect("read the classic transcript")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("a transcript line is JSON"))
        .filter(|row| row.get("_meta").is_none())
        .collect()
}
