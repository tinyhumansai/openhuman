//! Wire payloads for the medulla "harness plane" — the `medulla:task_*`
//! Socket.IO protocol that lets a medulla operator (running in the backend)
//! drive an OpenHuman agent session as a delegated sub-agent.
//!
//! See `docs/specs/session-streaming-api-spec.md` §6 in the medulla repo. All
//! payloads are camelCase on the wire to match the backend's Socket.IO
//! conventions (the harness *envelope* they carry stays snake_case — that is
//! the tinyplace v2 wire format, decoded/encoded by [`super::envelope`]).

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Down: backend / medulla → openhuman agent
// ─────────────────────────────────────────────────────────────────────────────

/// `medulla:task_run` — start a task in an openhuman agent session.
///
/// Creates (or resumes, when `session_id` is supplied) a session and sends
/// `instruction` as the opening prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRun {
    pub task_id: String,
    pub cycle_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub instruction: String,
    /// Which openhuman agent to run the task as (defaults to the orchestrator).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Hard wall-clock budget for the whole task, in milliseconds.
    #[serde(default)]
    pub timeout_ms: u64,
}

/// `medulla:task_send` — mid-task steering (answer a question / approval
/// decision / follow-up); `input` is delivered into the running session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSend {
    pub task_id: String,
    pub input: String,
}

/// `medulla:task_abort` — cancel the session/task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAbort {
    pub task_id: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Up: openhuman agent → backend / medulla
// ─────────────────────────────────────────────────────────────────────────────

/// `medulla:task_envelope` — one live-stream frame for a task, carrying a
/// `tinyplace.harness.session.v2` envelope (see [`super::envelope`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEnvelope {
    pub task_id: String,
    /// A serialized [`tinyplace::types::SessionEnvelopeV2`]. Kept as raw JSON so
    /// this struct stays a thin transport wrapper and never re-derives the
    /// envelope kinds.
    pub envelope: serde_json::Value,
}

/// `medulla:task_result` — explicit completion (preferred over idle-detection).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    pub task_id: String,
    pub ok: bool,
    #[serde(default)]
    pub reply: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A single agent descriptor advertised in the roster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDescriptor {
    pub agent_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// `medulla:register_agents` — roster advertisement sent on connect. The
/// backend clears the roster when this socket disconnects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterAgents {
    pub agents: Vec<AgentDescriptor>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Socket.IO event names
// ─────────────────────────────────────────────────────────────────────────────

/// Down events handled by openhuman.
pub const EVENT_TASK_RUN: &str = "medulla:task_run";
pub const EVENT_TASK_SEND: &str = "medulla:task_send";
pub const EVENT_TASK_ABORT: &str = "medulla:task_abort";

/// Up events emitted by openhuman.
pub const EVENT_TASK_ENVELOPE: &str = "medulla:task_envelope";
pub const EVENT_TASK_RESULT: &str = "medulla:task_result";
pub const EVENT_REGISTER_AGENTS: &str = "medulla:register_agents";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn task_run_round_trips_and_reads_camel_case_wire() {
        let wire = json!({
            "taskId": "t1",
            "cycleId": "c1",
            "sessionId": "s1",
            "instruction": "summarize the doc",
            "agentId": "orchestrator",
            "timeoutMs": 60000,
        });
        let parsed: TaskRun = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(parsed.task_id, "t1");
        assert_eq!(parsed.cycle_id, "c1");
        assert_eq!(parsed.session_id.as_deref(), Some("s1"));
        assert_eq!(parsed.agent_id.as_deref(), Some("orchestrator"));
        assert_eq!(parsed.timeout_ms, 60000);
        // Re-serialize and confirm it decodes back to the same value.
        let again: TaskRun =
            serde_json::from_value(serde_json::to_value(&parsed).unwrap()).unwrap();
        assert_eq!(parsed, again);
    }

    #[test]
    fn task_run_defaults_optional_fields() {
        let wire = json!({
            "taskId": "t2",
            "cycleId": "c2",
            "instruction": "go",
        });
        let parsed: TaskRun = serde_json::from_value(wire).unwrap();
        assert!(parsed.session_id.is_none());
        assert!(parsed.agent_id.is_none());
        assert_eq!(parsed.timeout_ms, 0);
    }

    #[test]
    fn task_send_and_abort_round_trip() {
        let send: TaskSend =
            serde_json::from_value(json!({ "taskId": "t", "input": "yes" })).unwrap();
        assert_eq!(send.input, "yes");
        let abort: TaskAbort = serde_json::from_value(json!({ "taskId": "t" })).unwrap();
        assert_eq!(abort.task_id, "t");
    }

    #[test]
    fn task_result_omits_none_and_round_trips() {
        let res = TaskResult {
            task_id: "t".into(),
            ok: true,
            reply: "done".into(),
            usage: None,
            error: None,
        };
        let v = serde_json::to_value(&res).unwrap();
        assert!(v.get("usage").is_none());
        assert!(v.get("error").is_none());
        assert_eq!(v["taskId"], "t");
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn register_agents_round_trips() {
        let roster = RegisterAgents {
            agents: vec![AgentDescriptor {
                agent_id: "orchestrator".into(),
                name: "Orchestrator".into(),
                description: "default".into(),
            }],
        };
        let wire = serde_json::to_value(&roster).unwrap();
        assert_eq!(wire["agents"][0]["agentId"], "orchestrator");
        let back: RegisterAgents = serde_json::from_value(wire).unwrap();
        assert_eq!(roster, back);
    }
}
