# Session host

`OpenHumanSessionHost` is the stateful entry point for a chat turn: the single
execution tier that the `channels`, `local_ai`, and `cron` layers call when a
conversation needs to make progress. It is a product shell over TinyAgents,
not a second implementation of the agent loop. `tinyagents_runtime::Session`
owns generic history, raw transcript reconciliation, prefix stability, tool
snapshots, resume, and persistence; this module supplies the model and tool
execution, the product transcript dialect, prompt policy, and the hooks that
run after each turn commits.

## Layout

| File | Role |
| --- | --- |
| [`types.rs`](./types.rs) | `OpenHumanSessionHost` and `SessionHostBuilder` struct definitions, no logic |
| `builder/` | `SessionHostBuilder` fluent API and the `from_config` factory |
| [`factory.rs`](./factory.rs) | `OpenHumanSessionFactory` |
| [`runtime_session.rs`](./runtime_session.rs) | Runtime session composition and the public `turn()` |
| `runtime/` | Public accessors, `run_single` |
| [`session_api.rs`](./session_api.rs) | Start, read back, and list the generations of a durable session |
| [`driver.rs`](./driver.rs) | The OpenHuman `SessionDriver` that runs the model and tools |
| [`hooks.rs`](./hooks.rs) | Product prepare, commit, and terminal hooks |
| [`codec.rs`](./codec.rs) | `OpenHumanTranscriptCodec`, the product transcript dialect |
| [`policy.rs`](./policy.rs) | Prompt and turn policy applied before dispatch |
| [`prefix_snapshot.rs`](./prefix_snapshot.rs) | Cache-prefix bookkeeping across resumes |
| [`recorded_tools.rs`](./recorded_tools.rs) | Rebuilds recorded Composio actions as deferred executors on resume |
| [`prelude_integrations.rs`](./prelude_integrations.rs) | Fetches connected integrations on the first turn of a session instance |
| [`announcement_notes.rs`](./announcement_notes.rs), [`artifact_wiring.rs`](./artifact_wiring.rs) | Smaller product wiring pulled out of the builder |
| `turn/`, [`turn_checkpoint.rs`](./turn_checkpoint.rs) | Per-turn state and checkpointing |

Import `OpenHumanSessionHost` and `SessionHostBuilder` from `crate::agent`,
which re-exports them from here. The child files are an implementation
detail.

## Sessions and resume

A conversation's durable identity is `tinyagents_session::transcript::SessionRef`,
derived from the thread id and the agent id, with no timestamp in the stem,
so one conversation always resolves to the same file. `session_api.rs` binds
that identity with `set_thread_id` and resumes with `ResumeMode::Session`; it
does not mint stems, seed history by hand, or pick a transcript by recency.

A resumed session's prompt and tool list are frozen: both were recorded on
the transcript, and restoring them verbatim is what keeps the provider's
prefix cache warm across a restart. `recorded_tools.rs` exists because of
that freeze: it turns previously recorded Composio actions back into
deferred executors instead of dropping them when a cache goes cold, and
`prelude_integrations.rs` re-fetches integrations on the first turn of every
session instance rather than only on a brand-new thread.

Compaction never erases history. It seals the current generation and opens
the next one, recording the sealed generation as the new one's parent; the
sealed file stays on disk, and `session_api.rs` is what lets a caller list a
long conversation's generations without reading the whole chain.

## Where next

- `crate::agent::tinyagents::host` for the run-context type this module
  builds from OpenHuman state, and for the harness call site itself.
- `crate::agent::subagent_host` for the same kind of adapter work, but over a
  child agent run instead of the top-level session.
- [`vendor/tinyagents`](../../../../../vendor/tinyagents/) for the session, resume, and transcript mechanics this
  module wraps rather than reimplements.

## Further reading

- [Parent module README](../README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Memory architecture](../../../../../docs/gitbooks/en/developing/architecture/memory.md)
