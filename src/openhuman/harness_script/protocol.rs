//! Wire protocol for the harness-script runner.
//!
//! Parent (Rust) and child (Python) exchange newline-delimited JSON over the
//! child's stdio, matching the stdio framing already used by
//! [`crate::openhuman::runtime_python::process`]. Every message is a JSON object
//! carrying a `type` tag; the child's stderr is a free-form log channel and is
//! not parsed as protocol.
//!
//! Phase 1 defines only the handshake (`init`/`ready`) and the terminal
//! (`result`/`error`) frames. The gap between `ready` and the terminal message
//! is where Phase 2 will insert `call`/`return` request frames, so the framing
//! is versioned: a peer that sees an unknown `protocol_version` fails fast
//! rather than silently misinterpreting a newer dialect.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Protocol dialect understood by this build. Bumped whenever the message set
/// or field semantics change in a non-backward-compatible way.
pub const PROTOCOL_VERSION: u32 = 1;

/// Resource limits handed to the child alongside the script. The child is free
/// to self-enforce where cheap, but the Rust runner treats these as the
/// authoritative caps and does not rely on child cooperation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScriptLimits {
    /// Wall-clock budget, in milliseconds, for the script to reach a terminal
    /// message after the handshake completes.
    pub wall_time_ms: u64,
    /// Maximum bytes of protocol stdout the runner will buffer before aborting
    /// with [`super::runner::HarnessScriptError::OutputCapExceeded`].
    pub max_stdout_bytes: usize,
    /// Maximum bytes of stderr log the runner retains; excess is dropped and
    /// flagged rather than treated as an error.
    pub max_stderr_bytes: usize,
}

/// Parent → child. Sent exactly once, immediately after a valid `ready`.
///
/// The `type` tag is an explicit field (rather than a serde container
/// attribute) so the on-wire shape is unambiguous and independent of serde's
/// struct-tagging behavior.
#[derive(Debug, Clone, Serialize)]
pub struct InitMessage {
    #[serde(rename = "type")]
    msg_type: &'static str,
    pub protocol_version: u32,
    /// The Python source the child should execute.
    pub script: String,
    /// Arbitrary JSON made available to the script as `inputs`.
    pub inputs: Value,
    pub limits: ScriptLimits,
}

impl InitMessage {
    /// Build an `init` frame for the current protocol version.
    pub fn new(script: String, inputs: Value, limits: ScriptLimits) -> Self {
        Self {
            msg_type: "init",
            protocol_version: PROTOCOL_VERSION,
            script,
            inputs,
            limits,
        }
    }
}

/// Child → parent messages. Phase 1 recognises the handshake ack and the two
/// terminal frames; unknown tags decode into [`ChildMessage::Unknown`] so the
/// runner can report a [`super::runner::HarnessScriptError::MalformedMessage`]
/// instead of failing to deserialize.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChildMessage {
    /// Handshake ack emitted on child startup.
    Ready { protocol_version: u32 },
    /// Terminal success; `output` is whatever the script passed to
    /// `set_result(...)` (JSON `null` if it never called it).
    Result { output: Value },
    /// Terminal failure; the script raised before producing a result.
    Error {
        error_class: String,
        message: String,
        #[serde(default)]
        traceback: Option<String>,
    },
    /// Any frame whose `type` is not recognised by this build.
    #[serde(other)]
    Unknown,
}

/// Serialize a parent→child message as a single protocol line (JSON + `\n`).
pub fn encode_line<T: Serialize>(message: &T) -> serde_json::Result<String> {
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    Ok(line)
}

/// Decode one child→parent protocol line.
pub fn decode_line(line: &str) -> serde_json::Result<ChildMessage> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn init_encodes_with_type_tag_and_trailing_newline() {
        let init = InitMessage::new(
            "set_result(1)".to_string(),
            json!({"k": "v"}),
            ScriptLimits {
                wall_time_ms: 1000,
                max_stdout_bytes: 4096,
                max_stderr_bytes: 4096,
            },
        );
        let line = encode_line(&init).expect("encode");
        assert!(line.ends_with('\n'));
        let parsed: Value = serde_json::from_str(line.trim_end()).expect("valid json");
        assert_eq!(parsed["type"], "init");
        assert_eq!(parsed["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(parsed["inputs"]["k"], "v");
    }

    #[test]
    fn decodes_ready_result_and_error_frames() {
        match decode_line(r#"{"type":"ready","protocol_version":1}"#).unwrap() {
            ChildMessage::Ready { protocol_version } => assert_eq!(protocol_version, 1),
            other => panic!("expected ready, got {other:?}"),
        }
        match decode_line(r#"{"type":"result","output":[1,2,3]}"#).unwrap() {
            ChildMessage::Result { output } => assert_eq!(output, json!([1, 2, 3])),
            other => panic!("expected result, got {other:?}"),
        }
        match decode_line(
            r#"{"type":"error","error_class":"ValueError","message":"boom","traceback":"tb"}"#,
        )
        .unwrap()
        {
            ChildMessage::Error {
                error_class,
                message,
                traceback,
            } => {
                assert_eq!(error_class, "ValueError");
                assert_eq!(message, "boom");
                assert_eq!(traceback.as_deref(), Some("tb"));
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_tag_decodes_to_unknown_variant() {
        let msg = decode_line(r#"{"type":"call","method":"agent.run"}"#).unwrap();
        assert!(matches!(msg, ChildMessage::Unknown));
    }
}
