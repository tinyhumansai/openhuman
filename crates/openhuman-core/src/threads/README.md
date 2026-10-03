# threads

Conversation and message management. This domain owns the `threads` RPC
namespace used by chat history, titles, labels, transcript projection, token
usage, and persisted per-turn processing snapshots. It does not own agent goals
or todos; those are TinyAgents capabilities integrated under `agent/`.

## Responsibilities

- List, create, update, delete, and purge conversation threads.
- List, append, and metadata-patch messages.
- Generate durable conversation titles with deterministic fallbacks.
- Persist restart-survivable turn snapshots for transcript/process replay.
- Project `session_raw/*.jsonl` into paginated chat display items.
- Total persisted token and cost usage for a conversation.
- Migrate legacy welcome-agent conversations to orchestrator naming.

## RPC surface

The `openhuman.threads_*` methods cover conversation lifecycle, messages,
titles, labels, turn-state reads, token usage, and transcript projection. There
is intentionally no task-board, todo, or goal RPC surface here.

## Persistence

Conversation and message storage is delegated to
`threads::store::blocking` (the store is `tinyagents_session::threads`); do not call the synchronous conversation
store from async handlers. Turn snapshots live under
`memory/conversations/turn_states/` and are retained for settled processing
replay. The welcome migration is guarded by
`state/migrations/welcome_to_orchestrator_v1.done`.

## Key modules

- `ops/`: conversation, message, title, usage, transcript, and turn-state ops.
- `schemas/`: controller schemas and thin handlers.
- `turn_state/`: persisted turn snapshots and the progress-event mirror.
- Transcript projection for the renderer lives in `tinyagents_session::transcript::view`; `ops/transcript.rs` serves it over RPC.
- `welcome_migration.rs`: one-shot legacy conversation migration.

Agent goal and todo integration lives in `crate::agent::{goals,todos}` and is
backed directly by `tinyagents_graph`.
