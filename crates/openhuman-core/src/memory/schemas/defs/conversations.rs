use super::common::{out, req};
use crate::core::{ControllerSchema, TypeSchema};

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    match function {
        "agents_list" => Some(ControllerSchema {
            namespace: "memory",
            function: "agents_list",
            description: "The agents with logged conversations under the memory root, and how many turns each.",
            inputs: vec![],
            outputs: out("{root, agents: {agent_id, turns}[]}"),
        }),
        "conversations_backfill_status" => Some(ControllerSchema {
            namespace: "memory",
            function: "conversations_backfill_status",
            description: "Progress of storing past chats, and how many threads and turns from before turn logging are still unstored.",
            inputs: vec![],
            outputs: out(
                "{state: {phase, threads_total, threads_done, turns_stored, items_stored, error?, finished_at?}, pending_threads, pending_turns}",
            ),
        }),
        "conversations_backfill_start" => Some(ControllerSchema {
            namespace: "memory",
            function: "conversations_backfill_start",
            description: "Store past chats (turns from before turn logging) as the main agent's conversations, in the background. Uploads chat history to the selected engine, so it requires consent: true.",
            inputs: vec![req(
                "consent",
                TypeSchema::Bool,
                "The user agreed to upload past chats to the engine.",
            )],
            outputs: out("Same as memory_conversations_backfill_status."),
        }),
        _ => None,
    }
}
