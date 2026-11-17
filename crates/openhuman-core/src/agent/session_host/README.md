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
| [`runtime_session.rs`](./runtime_session.rs) | Runtime session composition and the public `turn()` |
| `runtime/` | Public accessors, `run_single` |
| [`driver.rs`](./driver.rs) | The OpenHuman `SessionDriver` that runs the model and tools |
| [`hooks.rs`](./hooks.rs) | Product prepare, commit, and terminal hooks |
| [`codec.rs`](./codec.rs) | `OpenHumanTranscriptCodec`, the product transcript dialect |
| [`integrations/recorded_tools.rs`](../../integrations/recorded_tools.rs) | Rebuilds recorded Composio actions as deferred executors on resume |
| [`integrations/prelude_integrations.rs`](../../integrations/prelude_integrations.rs) | Fetches connected integrations on the first turn of a session instance |
| [`announcement_notes.rs`](../../integrations/announcement_notes.rs) | Smaller product wiring pulled out of the builder |
| `turn/`, [`product_copy.rs`](./product_copy.rs) | Product copy for TinyAgents checkpoint and close operations |

Import `OpenHumanSessionHost` and `SessionHostBuilder` from `crate::agent`,
which re-exports them from here. The child files are an implementation
detail.

## Sessions and resume

A conversation's durable identity is `tinyagents_session::transcript::SessionRef`,
derived from the thread id and the agent id, with no timestamp in the stem,
so one conversation always resolves to the same file. The runtime binds
that identity with `set_thread_id` and resumes with `ResumeMode::Session`; it
does not mint stems, seed history by hand, or pick a transcript by recency.

A resumed session's prompt and tool list are frozen: both were recorded on
the transcript, and restoring them verbatim is what keeps the provider's
prefix cache warm across a restart. `integrations/recorded_tools.rs` exists because of
that freeze: it turns previously recorded Composio actions back into
deferred executors instead of dropping them when a cache goes cold, and
`integrations/prelude_integrations.rs` re-fetches integrations on the first turn of every
session instance rather than only on a brand-new thread.

An agent whose definition sets `refresh_prompt_on_resume` opts out of the
prompt half of that freeze. `runtime_session/prompt_refresh.rs` renders its
prompt again on every resumed turn and offers it as a refreshing prefix; the
runtime applies it only when it differs from the recorded one, and a
thread-bound session then seals the current generation and carries the
conversation into the next. That turn's user message opens with a one-time
`[instructions update]` note: the conversation before it was written under the
old prompt, and a model otherwise keeps to what it said there (that a feature
is "coming soon") over the new instructions. A verbatim prompt therefore costs
one uncached prompt per thread per edit. A composed one also refreshes when
the live state it renders changes, but never from integrations the session has
not read yet. A prompt that fails to render, or a recorded prompt of unknown
extent, keeps the recorded prompt. The recorded tool list is unaffected.

An agent whose definition sets `memory_block` ends its system prompt with
what long-term memory knows about the user, a `<memory>` … `</memory>`
system message after every static prompt byte
(`runtime_session/memory_block.rs`). The block belongs to the frozen prefix,
so it is rendered only when a transcript generation starts: a new thread, a
resumed thread recorded without one, a turn whose prefix changes anyway (a
replaced prompt, a changed permanent-tool catalogue), and the first turn of a
generation that carried its parent's block over unchanged (an in-turn or
out-of-band compaction). Every other turn re-sends the recorded block
verbatim. The block is excluded from the "did the prompt change" comparison
like the catalogue, so a memory change alone never seals a generation or adds
the note. Code that opens a generation outside a turn can put a fresh block
into the successor's prefix with
`OpenHumanSessionHost::memory_block_for_new_generation` and
`memory_block::with_memory_block`; without it the next turn renders one.

Compaction never erases history. It seals the current generation and opens
the next one, recording the sealed generation as the new one's parent; the
sealed file stays on disk. TinyAgents owns the generation chain and its
transcript lookup.

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
