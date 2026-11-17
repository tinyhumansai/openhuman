use super::common::{opt, out};
use crate::core::{ControllerSchema, TypeSchema};

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    match function {
        "policy_get" => Some(ControllerSchema {
            namespace: "memory",
            function: "policy_get",
            description: "The memory lifecycle's policy: turn logging, the per-turn pack and its budgets, and the root and agent id work outside an agent resolves to.",
            inputs: vec![],
            outputs: out(
                "{log_conversations, recall: {enabled, budget_tokens, learnings_limit, brain_limit, history_limit, team_limit, build_beliefs_every, pre_turn_timeout_ms, date_hint, compaction_timeout_ms, build_delay_secs}, root, agent_id, host_bound}",
            ),
        }),
        "policy_set" => Some(ControllerSchema {
            namespace: "memory",
            function: "policy_set",
            description: "Change the memory lifecycle's policy.",
            inputs: vec![
                opt(
                    "log_conversations",
                    TypeSchema::Bool,
                    "Log every turn to the acting agent's conversations.",
                ),
                opt(
                    "recall_enabled",
                    TypeSchema::Bool,
                    "Give every turn a memory pack.",
                ),
                opt(
                    "budget_tokens",
                    TypeSchema::BoundedU64 {
                        min: 100,
                        max: 16_000,
                    },
                    "The pack's size in tokens.",
                ),
                opt(
                    "learnings_limit",
                    TypeSchema::BoundedU64 { min: 0, max: 50 },
                    "Learnings and built beliefs per pack.",
                ),
                opt(
                    "brain_limit",
                    TypeSchema::BoundedU64 { min: 0, max: 50 },
                    "Brain documents per pack.",
                ),
                opt(
                    "history_limit",
                    TypeSchema::BoundedU64 { min: 0, max: 50 },
                    "This agent's earlier turns per pack.",
                ),
                opt(
                    "team_limit",
                    TypeSchema::BoundedU64 { min: 0, max: 50 },
                    "Other agents' turns per pack; 0 leaves them out.",
                ),
                opt(
                    "build_beliefs_every",
                    TypeSchema::BoundedU64 { min: 0, max: 1000 },
                    "Turns between belief builds; 0 turns them off.",
                ),
                opt(
                    "pre_turn_timeout_ms",
                    TypeSchema::BoundedU64 {
                        min: 100,
                        max: 30_000,
                    },
                    "How long a turn waits for its pack.",
                ),
                opt(
                    "date_hint",
                    TypeSchema::Bool,
                    "Use a small model call to identify dates in each turn.",
                ),
                opt(
                    "compaction_timeout_ms",
                    TypeSchema::U64,
                    "How long compaction waits for recalled context, in milliseconds.",
                ),
                opt(
                    "build_delay_secs",
                    TypeSchema::U64,
                    "How long queued belief builds wait for newer writes, in seconds; must fit within the supported date range.",
                ),
            ],
            outputs: out("Same as memory_policy_get."),
        }),
        "pack_preview" => Some(ControllerSchema {
            namespace: "memory",
            function: "pack_preview",
            description: "The memory pack a turn would be given (with a query) or a session would start with (without one). Reads only.",
            inputs: vec![
                opt(
                    "query",
                    TypeSchema::String,
                    "What the turn says; omit to preview a session start.",
                ),
                opt(
                    "thread_id",
                    TypeSchema::String,
                    "Session start: the thread being resumed.",
                ),
                opt(
                    "agent_id",
                    TypeSchema::String,
                    "The memory agent to preview as; the default agent when omitted.",
                ),
            ],
            outputs: out(
                "{agent_id, root, mode: turn|session, pack: {markdown, tokens, refs, sections, skipped, engine}}",
            ),
        }),
        _ => None,
    }
}
