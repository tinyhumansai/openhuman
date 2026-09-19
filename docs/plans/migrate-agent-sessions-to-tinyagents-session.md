# Migrate host-independent sessions to `tinyagents-session`

This plan moves durable, host-neutral agent-session mechanics into
`vendor/tinyagents/crates/tinyagents-session`. It is intentionally separate
from agent execution: the session crate is queryable persistence, not a model
runner. Every consumer imports `tinyagents_session` directly; neither
OpenHuman nor `tinyagents-harness` may retain a forwarding module, alias, or
compatibility re-export.

## Boundary

Move the transcript model, JSONL codec, transcript/history interfaces,
workspace-relative transcript discovery, append/compaction mechanics, and
generic retention/import conversion. The target crate already owns SQLite
session records, FTS, retention, and the run ledger.

Keep in OpenHuman: `Config` and workspace selection; `Agent` construction and
model/tool execution; `ChatMessage`/provider conversion; prompt-injection and
security policy; memory extraction; BUS/progress/RPC/UI projection; backend
import selection; and every product-specific transcript field or migration.

In particular, do not move `session_memory`, `task_session`, web-chat's live
session cache, session-import controller schemas, `Agent::run_single`, or
anything in `turn/` that constructs a host turn. The target must never depend
on `openhuman`, `tinyagents-graph`, provider credentials, or product prompts.

## Target API and dependencies

Add these public modules to `tinyagents-session`:

```text
transcript::{
  TranscriptMeta, TranscriptMessage, TranscriptToolCall, SessionTranscript,
  DisplaySessionTranscript, TranscriptRecord, TurnUsage, MessageUsage,
  TranscriptHistory, TranscriptRead, TranscriptLocator,
  read_transcript, read_transcript_display, write_transcript,
  append_turn, compact_context, transcript_path, latest_for_agent,
  root_for_thread, thread_scoped_for_agent
}
```

Use the neutral `TranscriptMessage`/`TranscriptToolCall` durable model, not a
`tinyinference_llm::message::Message` or `ChatHistory` conversion. The durable
model preserves raw tool argument strings (including malformed streamed JSON),
opaque provider `extra_content`, tool-result correlation ids, usage, thinking,
trusted/verbatim metadata and caller-owned JSON metadata. Runtime-message
conversion is an explicit host-boundary operation after read / before write.
Keep stable JSONL `kind` tags forward-compatible: unknown records are skipped,
not fatal. `tinyagents-session` may depend on serde, serde_json, chrono and
std I/O (and its existing shared error substrate), but must not depend on graph,
the new orchestration crate, or a provider dialect for transcript persistence.

OpenHuman replaces its local types and helpers with direct imports, then
implements only an adapter at the existing host boundary where conversion from
`ChatMessage` to crate-native messages is unavoidable. Do not retain
`agent::session_host::codec::*` as the OpenHuman transcript adapter.

## File map

| Current OpenHuman source | Destination / action |
| --- | --- |
| `agent/harness/session/transcript/{types,jsonl,metadata,paths,reader,writer,legacy_md,markdown,thread_lookup}.rs` | Move/split into `tinyagents-session/src/transcript/`; preserve on-disk JSONL and legacy Markdown reader behavior. |
| Deleted transcript facade | Consumers import `tinyagents_session::transcript` directly; no OpenHuman forwarding module remains. |
| `agent/harness/session/transcript_history.rs` | Move generic history/read/locator traits and file implementation to `tinyagents-session/src/transcript/history.rs`; keep any OpenHuman message conversion adapter locally. |
| `agent/harness/session/runtime/resume.rs` | Split pure transcript-to-model replay/deduplication into session; keep `Agent` mutation and host message-log fallback in OpenHuman. |
| `agent/session_import/{convert,ops,live}.rs` | Move only pure descriptor/transcript conversion and generic source iteration; retain OpenHuman namespaces, controller/live backend wiring, run-ledger links, and memory import. |
| `agent/harness/session/{mod.rs,types.rs,builder/**,runtime/**,turn/**,tool_progress.rs,migration.rs}` | Keep: host session construction/execution, tools, policy, memory, progress, and migration policy. |
| `agent/context/session_memory.rs`, `agent/task_session.rs`, `web_chat/**`, `threads/**` | Keep. |
| All transcript/session-import tests named below | Port their host-independent cases to the matching TinyAgents module; retain OpenHuman adapter/integration tests. |

## TDD slices

### S1 — crate-native transcript model and codec

**RED:** Port `transcript_roundtrip_and_paths_tests.rs`,
`transcript_forward_compat_tests.rs`, `transcript_tests.rs`, and
`transcript_thread_and_append_tests.rs` into
`tinyagents-session/src/transcript/test.rs`. Assert byte-stable JSONL round
trip; tool calls, tool results, reasoning and usage survive; unknown kinds are
ignored; malformed required records fail; append writes only deltas; and path
derivation is deterministic.

**GREEN:** Add `transcript/{types,jsonl,metadata,paths,reader,writer}.rs` and
the public API above. `TranscriptMessage` is deliberately neutral and
lossless; do not add a public convenience conversion through a narrower
provider/harness message. Do not change established OpenHuman file bytes in
this slice.

### S2 — history, compaction, and discovery

**RED:** Port `transcript_history_tests.rs` and the relevant
`session_thread_resume*_tests.rs` cases. Test model-context rather than display
replay, compaction without destructive rewrite, clear semantics, latest-agent
lookup, root-thread lookup, thread-scoped lookup, and concurrent append safety.

**GREEN:** Move `TranscriptHistory`, `TranscriptRead`, `TranscriptLocator`,
and file implementation to `tinyagents-session::transcript::history`. Make the
history trait a direct dependency of the harness/session user; no OpenHuman
trait wrapper remains.

### S3 — replay and import conversion

**RED:** Add crate tests for replaying full-fidelity tool/reasoning messages,
dropping only the duplicated trailing user input, empty/missing transcript
behavior, legacy Markdown read-only import, and deterministic descriptor
conversion. Port only pure cases from `session_import/ops_tests.rs` and
`live_tests.rs`.

**GREEN:** Export pure replay/descriptor utilities from session. Keep
OpenHuman's `Agent::seed_resume_from_messages`, external conversation fallback,
backend import and memory namespace decisions as thin host calls around them.

### S4 — OpenHuman direct-import cutover

**RED:** Add OpenHuman adapter tests proving its `ChatMessage` conversion
preserves tool calls/reasoning/usage and that an old transcript resumes without
loss. Add a boundary check that fails if production code restores an
OpenHuman transcript forwarding facade.

**GREEN:** Update `agent/harness/session/**`, `agent/session_import/**`,
`agent/tinyagents/{journal,reaper,turn_runner}.rs`, and direct consumers to
import `tinyagents_session::transcript::*`. Delete moved source and tests; do
not add `pub use` compatibility paths.

## Verification

Run after each slice:

```bash
cargo test --manifest-path vendor/tinyagents/Cargo.toml -p tinyagents-session transcript
cargo test --manifest-path vendor/tinyagents/Cargo.toml -p tinyagents-session
pnpm debug rust session
cargo check --manifest-path Cargo.toml
pnpm rust:layout
```

Before the PR, run `scripts/ci-cancel-aware.sh cargo test --manifest-path
vendor/tinyagents/Cargo.toml --workspace`, `pnpm docs:check`, and the direct-
import/boundary grep. Do not export `CARGO_TARGET_DIR`.

## Integration order

1. Land the TinyAgents session-crate PR, including its direct `tinyinference`
   dependency if required.
2. Update the OpenHuman `vendor/tinyagents` gitlink to that landed/reviewed
   commit, cut over consumers, delete old paths, and open the OpenHuman PR.
3. The orchestration extraction may depend on the landed session API; do not
   start its session-backed adapter before S1–S3 API names are fixed.
