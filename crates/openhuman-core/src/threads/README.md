# threads

Chat threads and their messages: the conversation list in the sidebar, each
thread's message log, titles, labels, working folder, edit and regenerate,
global search, token usage, and the persisted snapshots of in-flight turns.
This folder owns the `threads` RPC namespace the frontend reads chat history
through, plus a bus subscriber that persists channel conversations (Slack,
Telegram and the rest) into the same store.

The storage engines are not here. The thread and message store, the turn-state
snapshot store, transcript projection and spend accounting all live in
`tinyagents_session` (`vendor/tinyagents`). This folder is the host layer over
them: async wrappers, RPC handlers, product rules (attachment limits, title
replacement, the fixed working folder) and the cleanup that ties a thread to
the rest of the core (sub-agents, web-channel sessions, turn snapshots).

## How it works

### Three kinds of per-thread data

A thread has three separate records on disk, all under the workspace:

```text
<workspace>/memory/conversations/
  threads.jsonl                      thread metadata, append-only upsert/delete log
  threads/<hex(thread_id)>.jsonl     the thread's message log (what the UI lists)
  turn_states/<hex(thread)>/<hex(request)>.json
                                     snapshot of one turn's progress
session store (agent::session_store) model-facing transcript (agent session host)
```

The message log is the user-facing history: one row per user message and
agent reply, which `threads.messages_list` returns. The transcript is what
the model sees, written by the agent session host through the session store
port, and is the source for `threads.transcript_get` and `threads.token_usage`.
Every transcript read here goes through that port under the key the session
host wrote with. The web-chat checkout records that key with the thread the
first time a session is bound to it (`threads::store::bind_transcript_key`,
first writer wins, the `threads_transcript_keys` collection in the thread's
scope), and `ops::support::transcript_store(ctx, thread_id)` reads it back, so
a later `[agent] chat_agent_id` change cannot strand the thread's earlier
turns. A thread with no record resolves the key as the session host would
today, from the request's own context (`<profile>~<agent>` for a SaaS profile,
else the context's agent, else the chat agent id), never from ambient state. The turn-state snapshots
record what a turn was doing (phase, tool timeline, streamed text) so a client
that reloads mid-turn, or after a restart, can rebuild the process view.

These are distinct id spaces. Message-log rows have their own ids; transcript
rows are keyed by a turn's `request_id`. The one bridge between them is the
assistant reply, whose store id is minted deterministically as
`agent:<request_id>` (`store::run_reply_message_id`, reversed by
`store::reply_run_id`). Edit and regenerate depend on that.

### A request through the namespace

```text
 frontend / CLI
     |  openhuman.threads_<fn>
     v
 schemas/registry.rs  -> handlers.rs (parse params)
     |
     v
 ops/<area>.rs        (product rules, Outcome<ApiEnvelope<T>>)
     |
     +--> store::blocking::*   (spawn_blocking)  --> tinyagents_session::threads
     +--> tinyagents_session::turn_state::store
     +--> tinyagents_session::transcript::{view, spend}
     +--> web_chat / agent::orchestration      (cancel, invalidate, restart)
```

Every store entry point is synchronous and takes `parking_lot` locks across
fsync'd file I/O. Calling one straight from an `async fn` parks a tokio worker
thread for the whole wait, so all request paths go through
`store::blocking`, which runs each call on the blocking pool. Responses are
wrapped in `ApiEnvelope` by [`ops/support.rs`](./ops/support.rs) (`envelope`, `counts`), which also
resolves the workspace dir and converts between store types and wire types.

### Deleting and purging

`thread_delete` and `threads_purge` touch more than the store, so each runs
as one `run_to_completion` unit: a caller that drops the request cannot leave
the work half done. Deleting a thread removes it from the store, invalidates
any web-channel session bound to it, cancels the detached sub-agents it
spawned (`agent::orchestration::running_subagents`) before discarding their
queued results (`background_completions`, which cancels the thread on the harness completion router), then deletes its turn snapshots.
Purge does the same for every thread and calls the parse-independent
`turn_state::store::clear_all`, so a corrupt snapshot file is removed too.

### Edit and regenerate

[`ops/edit.rs`](./ops/edit.rs) implements `threads.edit_message` and `threads.regenerate`. Both
cancel the thread's in-flight turn with `web_chat::cancel_chat`, fork the
session transcript at a cut point, truncate the message log to match
(`delete_after`), drop the turn snapshots for every turn the fork removed,
invalidate the web-channel session, and start a new turn with
`web_chat::start_chat`. Forking never edits the sealed transcript generation;
it carries the same guarantee as a compaction.

The cut point comes from the reply id bridge described above:

- `regenerate` with a `message_id` takes that reply's `request_id`, keeps the
  turn's user prompt and drops the answer and everything after.
- `regenerate` without one redoes the last turn
  (`TruncateCut::LastAssistantTurn`).
- `edit_message` names a user message, which has no correlation of its own. It
  finds the next deterministic reply id after it in the message log, recovers
  that turn's `request_id`, and cuts just before the turn. A user message with
  no reply yet only truncates the message-log tail.

### Titles

A new thread starts with a placeholder title. As soon as the user sends, an
interim title derived from the first message is written
(`support::update_thread_with_fallback_title`). `thread_generate_title`
([`ops/title_generation.rs`](./ops/title_generation.rs)) later asks the model for a summary title using the
`tinyagents_harness::title` helpers. `is_replaceable_title` only allows it to
replace the placeholder, or the interim title during the first exchange;
anything the user typed is left alone. Logs use the `[threads:title]` prefix
and a fingerprint, never the title text.

### Working folder

A thread can be bound to an `action_dir` before its first message
(`threads.update_working_dir`). `validate_working_dir` requires an absolute,
existing directory with no null byte and refuses anything
`SecurityPolicy::is_always_forbidden` covers. After the first message the
folder is fixed: a resumed session keeps its first system prompt and tool list
verbatim, so moving it would leave the model describing one directory while
its tools act in another. `web_chat::run_task` reads the binding through
`ops::thread_working_dir`.

### Turn-state snapshots

`web_chat::progress_bridge` drives a `tinyagents_session::turn_state::TurnStateMirror`
for each web turn. `turn_state::mirror::ObserveProgress` (`mirror/observe.rs`)
is the host projection: it turns each `agent::progress::AgentProgress` event
into a `TurnState` mutation. Streaming deltas only update memory; the mirror
flushes at iteration and tool boundaries. A finished turn is kept as
`Completed` so the UI can replay it. A turn whose bridge exits without
`TurnCompleted` is marked `Interrupted`, and at cold boot any non-terminal
snapshot from an unclean shutdown is marked the same way. The
`threads.turn_state_*` methods read these back.

### Channel persistence

`store::register_conversation_persistence_subscriber` ([`store/bus.rs`](./store/bus.rs))
subscribes to `DomainEvent::ChannelMessage*` on the core bus and mirrors
inbound and processed channel turns into the store, using
`conversation_history_key` for the thread id. It is registered from
`core/runtime/subscribers.rs`, `channels/runtime/startup/start_channels.rs`
and `security/credentials/ops/user_scope.rs` (when the user dir activates).
See [store/README.md](store/README.md).

### Welcome migration

`welcome_migration::migrate_welcome_agent_artifacts` is a one-shot cleanup for
the removed welcome-agent onboarding: it strips the obsolete `"onboarding"`
label from threads and renames `welcome*` session transcripts to
`orchestrator*`. It is idempotent, guarded by
`state/migrations/welcome_to_orchestrator_v1.done`, and called from
`platform/startup/ops.rs`.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Module declarations, re-exports of the RPC models, error type, controller aggregators and the welcome migration. |
| [`rpc_models.rs`](./rpc_models.rs) | Serde request and response types for the namespace (`ConversationThreadSummary`, `ConversationMessageRecord`, the `*Request` types). |
| [`error.rs`](./error.rs) | `ThreadsError` (`NotFound`, `Message`) and its structured RPC error with kind `ThreadNotFound`, so the frontend can drop a stale thread. |
| [`ops.rs`](./ops.rs) | Re-exports the operations from [`ops/`](./ops/). |
| [`ops/crud.rs`](./ops/crud.rs) | List, upsert, create, message list/append/update, `delete_after`, delete, labels, title, and `transcript_search`. `message_append` enforces the multimodal attachment limits for user messages. |
| [`ops/edit.rs`](./ops/edit.rs) | Edit and regenerate (see above). |
| [`ops/title_generation.rs`](./ops/title_generation.rs) | Model-generated titles and the replaceable-title rule. |
| [`ops/working_dir.rs`](./ops/working_dir.rs) | Validating and binding a thread's working folder. |
| [`ops/search.rs`](./ops/search.rs) | `threads.search`: cross-thread search with snippets for the global `Cmd`/`Ctrl+K` search. |
| [`ops/transcript.rs`](./ops/transcript.rs) | `threads.transcript_get`: paginated display items from `tinyagents_session::transcript::view::get_page_in` over the request's transcript store. |
| [`ops/usage.rs`](./ops/usage.rs) | `threads.token_usage`: token and cost totals from `transcript::spend::thread_spend_in` over the request's transcript store, re-priced, with per-sub-agent rows and the last turn's tokens for the context gauge. |
| [`ops/live_state.rs`](./ops/live_state.rs) | `threads.goal_get` and `threads.todos_get`: one-shot reads of the thread's agent goal and todo list for hydration on load. |
| [`ops/turn_state_ops.rs`](./ops/turn_state_ops.rs) | `threads.turn_state_*` over the snapshot store. |
| [`ops/purge.rs`](./ops/purge.rs) | Workspace-wide purge. |
| [`ops/support.rs`](./ops/support.rs) | Envelopes, workspace and transcript-store resolution (`transcript_store`), `run_to_completion`, type conversions, fallback titles. |
| [`schemas/`](./schemas/) | `schema_defs.rs` (controller schemas), `handlers.rs` (param parsing, thin handlers), `registry.rs` (the controller list). |
| [`store/`](./store/) | Re-export of `tinyagents_session::threads`, the `blocking` wrappers and the channel bus subscriber. See [store/README.md](store/README.md). |
| [`turn_state/`](./turn_state/) | `mirror/observe.rs` (progress projection) and `rpc_types.rs` (turn-state RPC payloads). |
| [`welcome_migration.rs`](./welcome_migration.rs) | The one-shot welcome-agent migration. |

## Key types and entry points

- `store::blocking::*` ([`store/blocking.rs`](./store/blocking.rs)): the async API every caller in the core should use for thread and message storage.
- `ConversationThread`, `ConversationMessage` ([`store/mod.rs`](./store/mod.rs)): the store types. `ConversationMessage` and `ConversationMessagePatch` are host names for `ThreadMessage` and `ThreadMessagePatch`.
- `ConversationThreadSummary`, `ConversationMessageRecord` ([`rpc_models.rs`](./rpc_models.rs)): their wire forms.
- `ThreadsError` ([`error.rs`](./error.rs)): the error type for handlers that can report a missing thread.
- `ops::token_usage` ([`ops/usage.rs`](./ops/usage.rs)): also called by `agent/context_breakdown.rs`.
- `ObserveProgress` ([`turn_state/mirror/observe.rs`](./turn_state/mirror/observe.rs)): the trait `web_chat::progress_bridge` uses to feed the mirror.

## RPC surface

Namespace `threads` (wire methods `openhuman.threads_<function>`), registered
through `all_threads_registered_controllers`:

| Group | Functions |
| --- | --- |
| Threads | `list`, `upsert`, `create_new`, `update_title`, `update_labels`, `update_working_dir`, `generate_title`, `delete`, `purge` |
| Messages | `messages_list`, `message_append`, `message_update`, `edit_message`, `regenerate` |
| Search and transcript | `search`, `transcript_get`, `token_usage` |
| Turn state | `turn_state_get`, `turn_state_list`, `turn_state_history`, `turn_state_get_turn`, `turn_state_clear` |
| Live agent state | `goal_get`, `todos_get` |

`goal_get` and `todos_get` are read-only. Goals and todos are written by the
agent (`agent::goals`, `agent::todos`, backed by `tinyagents_graph`) and their
live updates stream over the web channel (`thread_goal_updated`,
`thread_goal_cleared`, `thread_todos_changed`). There is no task-board or
todo-editing surface here.

## Boundaries

- The thread store (format, locking, trigram/CJK-bigram search index), the
  turn-state store and mirror, transcript projection and spend live in
  `tinyagents_session` in the `tinyagents` repo. Change them there.
- Memory is separate. `crate::memory` ingests committed turns through its own
  subscriber; this folder is transcript persistence, not memory.
- Running turns, cancelling them and streaming progress belong to `web_chat/`
  and `agent/`. This folder calls into them for edit, regenerate and delete.
- Agent goals and todos belong to `agent::goals` and `agent::todos`.

## Gotchas

- Never call `tinyagents_session::threads` functions directly from async
  code. Use `store::blocking`.
- The working folder can only change while `message_count` is 0. The RPC
  rejects later changes on purpose.
- Delete and purge cancel sub-agents before discarding their completions. In
  the reverse order a child could record a result into a thread that no
  longer exists.
- `ThreadsError::from_thread_scoped_store_error` only maps a store error to
  `NotFound` when the missing id matches the requested thread, so the
  frontend never clears the wrong thread.

## Tests

Tests sit beside their modules as `*_tests.rs` (for example
[`ops/edit_tests.rs`](./ops/edit_tests.rs), [`ops/search_tests.rs`](./ops/search_tests.rs), [`store/bus_tests.rs`](./store/bus_tests.rs),
[`transcript_host_tests.rs`](./transcript_host_tests.rs)). Tests that read the process config serialize on
`crate::config::TEST_ENV_LOCK` and point `OPENHUMAN_WORKSPACE` at a temp dir.
Run them with `cargo test -p openhuman threads::` or
`pnpm debug rust threads::`. The store's own tests live in
`vendor/tinyagents/crates/tinyagents-session/src/threads/`.

## Further reading

- [Chat](../../../../docs/gitbooks/en/features/chat.md)
- [Agent harness architecture](../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Frontend architecture](../../../../docs/gitbooks/en/developing/architecture/frontend.md)
