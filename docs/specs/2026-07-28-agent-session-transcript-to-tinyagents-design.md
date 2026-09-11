# Agent session persistence → TinyAgents

**Status:** spec + plan, awaiting a shape decision (§4). Written 2026-07-28
against `main` (`ce6c3e9b5`).
**Scope:** `src/openhuman/agent/harness/session/` — specifically the durable
transcript layer. Everything else in `session/` is out of scope and stays.
**Superseded in part (2026-07-28):** the maintainer has since decided to move
`agent/` wholesale into TinyAgents — see
[`plan-agents.md`](plan-agents.md).
That decision reopens this document's §6 "permanent host" rows for `builder/`,
`turn/`, `runtime.rs`, and `types.rs`, and selects **Option B** in §4 (the
transcript format becomes crate-owned). The state map, crate gap table, and
§3.5 analysis below remain accurate and are the input to that program's
Phases 2–4.

**Parent spec:** `2026-07-28-deshim-agent-inference-memory-seams-design.md`
(this document is the expansion of its DS-8).
**Related:** `docs/tinyagents-migration-plan-2026-07-22.md` (WP-1's
`ChatMessage` decision is the governing precedent), `99-deletion-ledger.md`.

---

## 1. The question this answers

Should `agent/harness/session/` become a `tinyagents::sessions` module?

**No — not as a unit.** But its durable transcript layer is a parallel
implementation of an abstraction the crate already ships, and that part should
converge. This document draws the line precisely, because drawing it wrong in
either direction is expensive: too aggressive and a GPL crate ends up importing
Composio; too timid and OpenHuman keeps two conversation-history models forever.

---

## 2. Current state

### 2.1 Size and split

`session/` is **18,142 LOC** across 17 files (~11,073 production, ~7,069 tests).
The transcript layer is:

| File | Prod LOC | Tests |
| --- | ---: | ---: |
| `transcript.rs` | 1,997 | 1,384 (`transcript_tests.rs`) |
| `turn_checkpoint.rs` | 105 | — |
| `migration.rs` | 373 | 170 (`migration_tests.rs`) |
| **Total** | **2,475** | **1,554** |

### 2.2 Why only these three

Measured host-domain fan-out (`grep -o "crate::openhuman::[a-z_]*" | sort -u`):

| File | Prod LOC | Host domains | In scope? |
| --- | ---: | ---: | --- |
| `transcript.rs` | 1,997 | **2** (`agent::messages::ChatMessage`, `inference::provider::ToolCall`) | **yes** |
| `turn_checkpoint.rs` | 105 | **2** (`ChatMessage`, `hooks::ToolCallRecord`) | **yes** |
| `migration.rs` | 373 | **0** (`anyhow`, `std::fs`, `std::path`) | **yes** |
| `builder/factory.rs` | 1,699 | 21 — composio, security, skills, subconscious, profiles, memory_store, memory_tools, agent_registry, agent_experience, agent_memory, embeddings, tokenjuice, learning, app_state, config, context, inference, tools, agent, tinyagents, … | no |
| `turn/core.rs` | 2,207 | 10 — mcp_registry, thread_goals, agent_orchestration, agent_experience, agent_memory, composio, memory, util, agent, tinyagents | no |
| `turn/session_io.rs` | 854 | 7 — learning, session_import, config, context, inference, agent, tinyagents | no |
| `runtime.rs` | 817 | 14 — channels, composio, prompt_injection, agent_tool_policy, skills, memory, … | no |
| `builder/setters.rs` | 666 | 10 | no |
| `turn/tools.rs` | 650 | 6 — composio, profiles, skills | no |
| `types.rs` | 477 | 11 | no |
| `turn/context.rs` | 343 | 7 — app_state, learning, memory | no |
| `turn/mod.rs`, `turn/graph.rs`, `builder/*`, `mod.rs`, `tool_progress.rs` | ~840 | 3–7 each | no (see §6) |

**~8,600 of 11,073 production lines are host wiring.** `builder/factory.rs`
reaches 21 OpenHuman domains; its job is literally "assemble OpenHuman's product
surface into a harness". Moving it down inverts the dependency and violates the
port plan's standing GPL/crates.io rule: only genuinely generic code goes into a
publicly redistributed crate.

### 2.3 What `transcript.rs` actually implements

Storage layout:

```text
{workspace}/session_raw/{stem}.jsonl        ← source of truth (flat dir)
{workspace}/sessions/YYYY_MM_DD/{stem}.md   ← human view, never read back
```

`stem` = `{unix_ts}_{agent_id}`, or `{parent_chain}__{unix_ts}_{agent_id}` for a
sub-agent — timestamp-first so a plain directory listing sorts by creation time
and `find_latest_transcript` is one directory scan.

Semantics, in the order they matter:

1. **Append-only event log.** `append_transcript_turn` never rewrites existing
   lines. It classifies the new logical message set against what was persisted:
   pure extension → append the tail; reduction/rewrite → append a single
   `{"kind":"compaction","replacement":[…]}` record carrying the full reduced
   set, leaving earlier turns on disk.
2. **Two read paths from one log.** `read_transcript` replays for *model
   context* (compaction records replace the accumulator; `interrupted:true`
   partials are skipped). `read_transcript_display` returns **every** record in
   file order so the UI can render pre-compaction history, compaction markers,
   and interrupted partials.
3. **Rewrite-free cumulative meta.** A fresh `{"_meta":{…}}` line is appended
   each turn; readers take the **last** `_meta` as authoritative, with line 1 as
   a valid fallback for older cores.
4. **Forward/backward compatibility by construction.** `MessageLine` carries
   `#[serde(flatten)] _extra`; `MetaPayload` does not set
   `deny_unknown_fields`; `_meta.version` is `TRANSCRIPT_SCHEMA_VERSION`. Old
   cores skip unknown-kind lines instead of crashing. Legacy
   `session_raw/DDMMYYYY/` layout still resolves for resume.
5. **Usage rollups.** `read_thread_usage_summary` →
   `ThreadUsageSummary` + `SubagentArchetypeUsage`.

Public surface: 26 items — `SessionTranscript`, `TranscriptMeta`,
`DisplaySessionTranscript`, `DisplayRecord`, `DisplayMessage`,
`CompactionMarker`, `TurnUsage`, `MessageUsage`, `ThreadUsageSummary`,
`SubagentArchetypeUsage`, plus `write_transcript`, `append_transcript_turn`,
`append_interrupted_partial`, `read_transcript`, `read_transcript_display`,
`read_transcript_legacy_md`, `read_thread_usage_summary`,
`find_root_transcript_for_thread(_in_dir)`, `resolve_keyed_transcript_path(_in_dir)`,
`resolve_new_transcript_path`, `find_latest_transcript(_in_subdir)`.

### 2.4 Consumers

**24 files across 5 domains** import `transcript::`: `agent`, `threads`,
`session_import`, `learning`, `migrations`. Notable non-agent callers:

- `threads/transcript_view/project.rs` — the derived-view projection
- `threads/turn_state/mirror.rs` — `find_root_transcript_for_thread`, `append_interrupted_partial`
- `threads/ops.rs` — `read_thread_usage_summary`
- `session_import/{ops,convert,live}.rs` — external session import
- `learning/transcript_ingest/mod.rs` — lesson extraction
- `migrations/phase_out_profile_md.rs`

This is the **transcript-derived-view architecture**: `session_raw` JSONL is the
source of truth, `turn_state` is a derived cache. Any change here is a change to
that contract, not just to one module.

---

## 3. What the crate already ships

`vendor/tinyagents` v2.1.0:

| Crate capability | Module | Relevance |
| --- | --- | --- |
| `ChatHistory` trait — `messages(thread_id) -> Vec<Message>`, `append`, `replace`, `clear` | `harness::memory::types:48` | **The direct analogue.** Thread-scoped conversation history surviving across runs. |
| `InMemoryChatHistory`, `StoreChatHistory<S: Store>`, `ShortTermMemory<H>` (trim policy), `MemoryScope` | `harness::memory` | Backends + short/long-term layering |
| `Store` — kv (`get`/`put`/`delete`/`list`) **plus an append-only stream API**: `append(stream, value) -> u64`, `read_from(stream, offset)`, `len(stream)` | `harness::store` | The stream half is a closer structural match to the JSONL log than `ChatHistory` is |
| `InMemoryStore`, `FileStore`, `StoreRegistry` | `harness::store` | Backends |
| `Checkpointer`, `FileCheckpointer`, `SqliteCheckpointer`, `DurabilityMode` | `graph::checkpoint` | Superstep state snapshots — **different concern**, not a transcript |

**So OpenHuman is not missing a home; it has a second implementation.** That is
the whole argument for this work.

### 3.1 Gap table — what crate `ChatHistory` cannot express today

| OpenHuman semantic | Crate equivalent | Gap |
| --- | --- | --- |
| Append-only log, never rewrite | `Store::append`/`read_from` (stream API) | present on `Store`, **absent from `ChatHistory`** (`replace` is a bulk overwrite) |
| Compaction record with `replacement` set | — | **missing** |
| `interrupted:true` partials, skipped on model read | — | **missing** |
| Dual read paths (model-context replay vs display-order) | — | **missing** — `messages()` is single-view |
| Cumulative `_meta` totals appended per turn | — | **missing** |
| Schema versioning + unknown-line tolerance | — | **missing** (a policy, easily added) |
| `.md` companion render | — | **host** — product surface, never upstream |
| Stem naming, latest/resume discovery, legacy dir fallback | — | **host** — OpenHuman workspace layout |
| Thread/subagent usage rollups | `harness::usage`, `harness::cost` | partial; the archetype rollup is product |

`ChatHistory` is a **strictly weaker interface** than what OpenHuman needs. A
naive "just implement `ChatHistory`" that routes reads through `messages()`
would silently drop compaction and interrupted-partial semantics — i.e. corrupt
the model context on any compacted thread. This is the single most important
constraint in this document.

---

## 3.5 Re-examined: `builder/factory.rs` and `turn/core.rs`

The §2.2 verdict ("21 imports, therefore host") describes the *wiring*, not the
*shape*. A shape can be generic even when every value flowing through it is
product-specific, so both files were re-read rather than dismissed on the import
count. Result: **the structure is already in the crate; the residue is genuinely
product — with two concrete exceptions worth acting on.**

### 3.5.1 `Agent` is not a duplicate of `AgentHarness`

`AgentHarness<State, Ctx>` (`harness/runtime/types.rs:259`) has **six** fields —
`models`, `tools`, `middleware`, `policy`, `tool_timeouts`, `response_cache` —
and a complete builder API: `new`, `register_model`, `set_default_model`,
`register_tool`, `push_middleware`, `push_model_middleware`,
`push_tool_middleware`, `with_policy`, `with_tool_timeout_settings`,
`with_response_cache`. The seam already drives exactly this API
(`tinyagents/mod.rs:1793–2302`).

The host `Agent` (`session/types.rs:31`) has **40+ fields**, and the ones that
define it are not execution config at all:

- **Per-turn product accumulators:** `last_memory_context`,
  `last_turn_citations`, `last_turn_usage_totals`, `last_turn_hit_cap` — read
  by web-channel delivery to render citation chips, token/cost meters, and to
  distinguish "paused at the iteration cap" from "asked a question".
- **Host service handles:** `memory`, `shared_experience_memory`,
  `memory_loader`, `tool_policy_session`, `workflows`.
- **Product policy:** `learning_enabled`, `explicit_preferences_enabled`,
  `subagent_tool_ceiling_names`, `visible_tool_names`.
- **Event identity:** `event_session_id`, `event_channel`.

`Agent` is OpenHuman's **session-state + turn-result object**; `AgentHarness` is
the crate's execution configuration. They are different things that both happen
to hold tools and a model. That single overlap is already bridged by
`SharedToolAdapter`, which the WP-4 decision fixed as a permanent boundary. And
`Agent` already holds a crate type directly — `workspace_descriptor:
tinyagents::harness::workspace::WorkspaceDescriptor` — which is what convergence
looks like in practice: adopt crate types field by field, don't relocate the
struct.

**So there are not two builders competing.** There are two builders in series
(host `Agent::from_config` → seam → crate `AgentHarness`), building two
different objects. Moving `factory.rs` down would require the crate to import
Composio, OpenHuman `SecurityPolicy`, `memory_store`, `skills`, `profiles`, and
`subconscious` — the exact GPL/crates.io boundary violation the port plan
forbids.

### 3.5.2 `turn/core.rs` is preparation, and the engine already left

WP-3 established that `turn/core.rs` "performs OpenHuman turn preparation and
calls the TinyAgents session path" — it contains no turn engine. Reading it
confirms that: `impl Agent` starts at line 432, and everything above it is
product gating. What `turn()` actually does before delegating:

- agent-experience retrieval and `prepend_experience_block`;
- memory recall citation collection;
- integration / MCP / skill announcement + retraction notes.

None of that is framework-shaped. **But ~150 LOC of it is:**
`tool_records_from_conversation`, `stamp_tool_failures`, `parse_tool_call_id`,
`short_failure_detail`, `replace_last_assistant_reply` are pure message-list
manipulation over roles and tool-call ids. Those are candidates for crate
`harness::message` helpers — small, but genuinely generic.

### 3.5.3 The one real duplicate: dispatcher-kind resolution

`resolve_dispatcher_kind` (`factory.rs:1398`) picks Native / Xml / PFormat from
`supports_native`, plus an `integrations_agent` override. The three
`ToolDispatcher` impls then render and parse tool calls accordingly.

**The crate already makes this decision.** `OpenAiModel` exposes
`with_native_tool_calling(bool)` and internally computes
`prompt_guided_tools = !self.profile.tool_calling && !request.tools.is_empty()`
(`providers/openai/transport.rs:951`), replaying calls and results as text when
prompt-guided — that is #55. So OpenHuman decides native-vs-prompt-guided at the
session-build layer, and the crate decides it again at the model layer.

This is the **same subsystem** as the parent spec's DS-5b (`harness/parse.rs`
duplicating #55/#57 parsing). Dispatcher selection, tool-call rendering, and
tool-call parsing are one concern split across two layers and two owners. They
should be resolved together, and the answer is almost certainly: the crate owns
the native-vs-prompt-guided decision and both directions of the wire format;
OpenHuman keeps only the `integrations_agent` override as an explicit policy
input, and the durable `to_provider_messages` serialization.

### 3.5.4 The generalizable conclusion

The productive question is not *"which file moves down"* but *"which extension
point is missing upstream"*. `factory.rs` and `turn/core.rs` are large because
the crate offers registries and middleware but no typed **turn-preparation**
seam — so every product enrichment (experience, citations, and announcements)
is hand-written into one `turn()` body instead of registered.

If the crate grew a `ContextEnricher` / `TurnPreparation` pipeline —
ordered, fallible, each returning prompt fragments plus metadata — OpenHuman's
enrichment becomes registrations rather than a bespoke method, and the *shape*
moves down while the *wiring* stays up. That is worth designing (S6), and it is
what "enrich it as a library" should mean here. It is explicitly **not** a
licence to relocate 3,900 lines of host wiring.

---

## 4. The decision (§4 is the gate — pick one before writing code)

### Option A — Host backend behind the crate trait *(recommended)*

Keep `ChatMessage`, the `session_raw` format, and every host-only helper exactly
as they are. Add `impl tinyagents::harness::memory::ChatHistory for
SessionTranscriptHistory`, converting at the boundary via the existing
`agent/message_convert.rs`. The harness talks to the crate trait; OpenHuman owns
the format. Host-only semantics (display read, compaction markers, usage
rollups, path resolution) stay on the concrete type, reached directly by the 24
consumers that need them.

- **On-disk change:** none. **Migration risk:** none.
- **Removes:** ~15 LOC — the duplicated path-resolution block in
  `persist_session_transcript`. **Adds** ~90 for the read + locator seam
  (S2–S4 as a whole: +782 / −0). Option A buys a single documented,
  *substitutable* seam, not a line reduction. The original "~400 LOC of parallel
  abstraction" figure was measured and refuted — see
  [Where "~400 LOC" came from](#where-400-loc-came-from-and-why-it-is-struck)
  — and it contradicted this option's own **Weakness** bullet two lines below.
- **Cost:** low. Reversible.
- **Weakness:** the crate trait is only used on the narrow runtime path; most of
  `transcript.rs` stays. Honest framing: this fixes *"two abstractions"*, not
  *"two implementations"*.

### Option B — Upstream a generic append-only history backend

Add `JsonlChatHistory` to the crate beside `StoreChatHistory`, and extend
`ChatHistory` (or add a `ReplayableChatHistory` supertrait) with the three
missing generic semantics: compaction-replacement records, skippable partials,
and a display-order read. `ChatMessage`'s durable fields survive as crate
`Message` + a `raw` passthrough, mirroring how the tool-model decision preserves
`ToolResult::raw`.

- **Removes:** ~2,100 host LOC.
- **Cost:** high — a trait extension in a published GPL crate, a durable format
  becoming public API, and an on-disk parity soak.
- **Only justified if** a second host will use it. Compaction-aware append-only
  transcripts are genuinely generic agent-framework machinery, so this is
  defensible — but it is a crate-roadmap decision, not a cleanup.

### Option C — Declare host-owned, close the question

Record `transcript.rs` in the deletion ledger as HOST-OWNED (durable on-disk
format + product read paths), same disposition as `tool_status` and
`namespace_store`. Costs nothing, keeps two abstractions.

**Recommendation: A now, B only if the crate roadmap wants a durable transcript
primitive.** A is cheap, reversible, and removes the thing that actually
confuses readers. B should not be smuggled in as refactoring.

---

## 5. Plan (Option A)

Every slice: failing-before/passing-after test, small validated commit on a
feature branch, `atomic-commit` with explicit paths.

### S0 — Ledger + decision record (no code)

Add a ledger row for `harness/session/` recording the §2.2 split and the §4
choice. Without this, a future audit re-opens "why didn't session move?" — which
has already happened twice in this migration.

**Exit:** row present; this doc linked from the parent spec's DS-8.

### S1 — `migration.rs` disposition

Zero host imports, but it migrates *OpenHuman's* directory layout
(`session_raw/DDMMYYYY/` → flat). Generic code for a host-specific format.

Expected outcome: **stays host**, recorded with that reason. Do the 10-minute
check rather than assuming; if it turns out to be a general "flatten a
date-bucketed log dir" utility with no OpenHuman naming, it can go down.

**Exit:** one ledger row, either way.

### S2 — Extract the trait-shaped surface

Introduce `SessionTranscriptHistory { workspace_dir, stem }` in
`session/transcript_history.rs`, wrapping the existing free functions. No
behaviour change; purely a handle where a trait impl can live.

**Exit:** `cargo check` + existing `transcript_tests.rs` green, untouched.

### S3 — Implement crate `ChatHistory`

```rust
impl tinyagents::harness::memory::ChatHistory for SessionTranscriptHistory {
    async fn messages(&self, thread_id: &str) -> Result<Vec<Message>>;  // read_transcript (model-context replay) → message_convert
    async fn append(&self, thread_id: &str, message: Message) -> Result<()>;
    async fn replace(&self, thread_id: &str, messages: Vec<Message>) -> Result<()>;
    async fn clear(&self, thread_id: &str) -> Result<()>;
}
```

Hard requirements, each with its own test:

- `messages()` MUST route through the **model-context** replay path, so
  compaction records replace the accumulator and `interrupted` partials are
  skipped. A test must construct a compacted transcript and assert
  `messages()` == `read_transcript()`, not the raw line set.
- `replace()` MUST map onto the compaction-record path
  (`append_transcript_turn`'s reduction branch), **not** a file rewrite. The
  append-only invariant is the format's core property; a trait default that
  clears-then-appends would destroy history.
- `clear()` semantics must be decided explicitly — truncate vs. start a new
  stem. Whichever, write it in the doc comment.

**Exit:** compaction and interrupted-partial round-trip tests green; the
byte-identity assertion against the pre-change reader passes.

### S4 — Route the harness through the trait

The turn path takes `Arc<dyn SessionHistory>` instead of calling transcript free
functions. The 24 consumers that need display records, usage rollups, or path
resolution keep using the concrete type — that is correct, not debt.

**Exit:** `agent_harness_e2e` + `scripts/test-rust-with-mock.sh` green;
`threads/transcript_view` projection output unchanged (golden test).

#### Landed as `SessionHistory: ChatHistory`, not `ChatHistory` — and why

S4's two halves as originally written contradict each other. `ChatHistory`
(`vendor/tinyagents/src/harness/memory/types.rs`) has four methods, each
carrying only a `thread_id: &str` plus `Message` / `Vec<Message>`. The turn
path's write carries three things none of them can express:

- **`request_id`**, stamped on every line. Drives `DisplayItem::TurnBoundary`
  and the `(request_id, ts)` root-turn segments that anchor every
  `DisplayItem::Subagent` in `threads/transcript_view/project.rs`.
- **`turn_usage`**, attributed to the turn's last assistant row. Carries
  `model`, `iteration`, `ts`, `reasoning_content` and the native `tool_calls`.
  The projection reads **every** `DisplayItem::ToolCall` off
  `turn_usage.tool_calls`, so losing it deletes the tool rows outright (each
  following `role:"tool"` line then falls to the orphan branch), along with
  `Reasoning`, `AssistantMessage.{model,iteration}` and `interim`.
- **`TranscriptMeta`'s cumulative fields** — `turn_count` plus the four
  token/cost rollups `read_thread_usage_summary` reports. The turn path computes
  these fresh each turn; the trait path can only re-read the file's existing
  `_meta`, which would freeze them at the previous turn's values.

A literal `Arc<dyn ChatHistory>` write would therefore have failed S4's own exit
criterion while looking complete. The criterion wins: what landed is
`pub(crate) trait SessionHistory: ChatHistory`, declared in
`agent/harness/session/transcript_history.rs`, whose single `append_turn` method
forwards the same six arguments `append_transcript_turn` already takes. The
indirection is real, `ChatHistory` stays in the bound so S2/S3 are not orphaned,
and the on-disk bytes are unchanged by construction —
`append_turn_is_byte_identical_to_the_free_function` writes one turn both ways
and compares the files byte for byte.

The **read** path does not cross `ChatHistory` either, and that half is settled
for the same shape of reason: `ChatHistory::messages()` returns `Vec<Message>`,
and converting back with `message_to_chat_message` flattens
`Assistant.tool_calls` into plain text — exactly what
`bound_cached_transcript_messages`' TAURI-RUST-7 trailing strip inspects, and
what native providers reject with `400 assistant message with 'tool_calls' must
be followed by tool messages`. (A round-trip probe confirmed the loss set:
assistant `tool_calls`, plus `openhuman_turn_usage` `extra_metadata` and
`AssistantMessage.id`, both inert here. The tool-failure marker is *not* in it —
that is a write-side/display-side field `read_transcript` never re-emits.)

An earlier revision of this section concluded from that the read must stay on
the concrete free functions. That conclusion was wrong, and the reasoning behind
it rested on a premise that is false of the type as it stands:
`SessionTranscriptHistory` is bound to a resolved `PathBuf`, not to a stem — its
two constructors merely *resolve* one — so a discovered path can be bound
verbatim. What landed instead:

- **`SessionTranscriptRead { path(); read_session() -> Option<SessionTranscript> }`**,
  a second supertrait of `SessionHistory`. `read_session` is the same
  `read_transcript` call the free-function readers made, returning the same
  struct, so losslessness is *structural*: nothing crosses `Message`, and
  compaction replay, `interrupted: true` partial skipping and the `_meta` header
  `maybe_shadow_read_session_store` needs all survive by construction. Split
  from `SessionHistory` rather than added to it because a discovered transcript
  can still be a legacy `.md` file, and handing read results out as
  `Arc<dyn SessionTranscriptRead>` makes appending JSONL into one impossible by
  construction rather than by convention.
- **`SessionTranscriptHistory::opened_at(path, seed_meta)`**, which stores the
  discovered path verbatim. It deliberately bypasses
  `resolve_keyed_transcript_path_in_dir`, which `create_dir_all`s and forces a
  `.jsonl` extension — that would mangle the legacy `.md` case and create stray
  directories on a pure read.
- **`SessionHistoryLocator`** (`latest_for_agent` / `root_for_thread` /
  `open_stem`), with `FileTranscriptLocator` as the default. Discovery *is* the
  thing `ChatHistory` cannot express — it is `thread_id`-keyed and returns
  messages, never a location — so it belongs on an OpenHuman-side object.
  Leaving it as free functions was what kept the read half on the filesystem no
  matter what handle was injected.

**The injection point now exists**, which is what makes the `Arc<dyn …>`
non-decorative. `AgentBuilder::with_session_history_locator` sets
`Agent::session_history_locator`; `Agent::session_locator()` resolves `None`
*lazily* into a `FileTranscriptLocator` over the **current** `workspace_dir` /
`session_raw_subdir` (never frozen at build time — callers reassign
`workspace_dir` after `build()`, and a captured locator would silently keep
reading the old directory). One injected object now covers both resume reads
*and* the session's own write handle, and
`fake_locator_substitutes_the_whole_turn_path` drives all three through a fake
and asserts nothing is written under the workspace.

`persist_session_transcript`'s own path resolution went with it:
`session_transcript_path` is now simply the bound handle's `path()`, so the two
can no longer drift.

**Widening `ChatHistory` upstream is REJECTED, not deferred.** S0's rationale
notes this question has already been re-opened twice, so the finding is recorded
here to stop a third round: the crate's `Usage` has no `cost_usd` /
`context_window`; `TranscriptMeta` is a cumulative *file header*, not turn
provenance; and the per-message tool-failure `extra_metadata` that
`message_to_chat_message` drops is untouchable by any turn-level record. You
would pay a tinyagents release and still need a `serde_json::Value` escape
hatch — for a trait that has no consumer inside the vendored crate outside
`harness/memory/`.

One live defect was fixed on the way in: the handle resolved its path through
`resolve_keyed_transcript_path`, which hardcodes `{workspace}/session_raw/`. A
dedicated-memory profile's sessions live in `session_raw-<id>/`, so wiring the
handle into the turn path as-written would have silently cross-written profile
sessions into the shared profile's directory. `new_in_dir` takes the raw dir
explicitly and is what the turn path uses.

Deliberately **not** relocated: `persisted_transcript_messages` and
`session_transcript_path` stay on `Agent`. The former is the in-memory diff
cache the append-only writer needs; substituting the handle's disk re-read is
lossy against `common_prefix_len` (`read_transcript` lifts `failure` /
`failure_detail` out of `extra_metadata` and hoists turn-usage to top-level line
fields), so the writer would emit a full compaction record every turn. The
latter is what `maybe_dual_write_session_store` needs a concrete `&Path` for.

Also deliberately kept: the `impl ChatHistory for SessionTranscriptHistory` from
S3 still has **no production caller** after S4 — reads go through
`read_session`, writes through `append_turn`. It is not deleted, because it is
the crate-side seam Option A exists to establish and it supplies the
`Send + Sync + 'static` bounds the shared handle needs. The trigger that would
delete it is an explicit decision to drop `ChatHistory` from the
`SessionHistory` bound, which frees `transcript_history.rs`'s
`read`/`persisted`/`meta_for_write`/`write_logical_set`/`impl ChatHistory` plus
most of its test module (~570 lines together). That is the only ~400-scale
removal S4 can actually make — and it removes abstraction this work itself
added, which is not what Option A was promising. Recorded so a future audit
finds the decision rather than re-deriving it.

#### Where "~400 LOC" came from, and why it is struck

§4 Option A originally claimed it "Removes: ~400 LOC of parallel abstraction".
The figure has no derivation anywhere in this document or its parent, and it is
not achievable. Its arithmetic origin is recoverable: §2.1's in-scope table
totals **2,475** LOC at this document's base commit (`transcript.rs` 1,997 +
`turn_checkpoint.rs` 105 + `migration.rs` 373). Option B's "~2,100 host LOC" is
exactly 1,997 + 105. The residual is **373 ≈ "~400" = `migration.rs`** — which
§5 S1 and `docs/tinyagents-full-migration-plan/99-deletion-ledger.md:33` both
resolve as HOST-OWNED, no deletion. Under Option A the §2.1 table loses **zero**
lines.

Every other candidate was checked and refuted:

- **No host trait duplicates `ChatHistory`.** The only other match in `src/` is
  a `MemorySource::ChatHistory` *enum variant* in `memory/remember.rs`.
  `memory/store/memory_trait.rs` is the long-term semantic `Memory` trait — a
  different concern with a different shape.
- **`ShortTermMemory`'s `trim` is an empty hook slot**
  (`vendor/tinyagents/src/harness/memory/types.rs`), so there is no crate-side
  policy for the host to be parallel to. The host side is 104 LOC of
  provider-400 defences (`trim_history`, `bound_cached_transcript_messages`)
  with no crate analogue — not duplicated, not deletable.
- **`agent/context/`'s reducer was already deleted under #4249**, before this
  document was written (`context/manager.rs`: "Live history reduction/
  summarization moved to the tinyagents graph"). What remains is prompt
  assembly + stats.

#### The one genuine parallel abstraction, and why S4/S5 cannot remove it

The #4249 JSONL↔store mirror **is** a second session-persistence implementation,
over crate `Store`/`AppendStore` rather than `ChatHistory`: `session_import/
live.rs` (353), `Agent::maybe_shadow_read_session_store` /
`maybe_dual_write_session_store` (119), the `StoreRegistry` registration in
`agent/tinyagents/mod.rs`, two `AgentConfig` flags, and
`config/migrations/enable_session_shadow_reads.rs` — ~565 prod LOC. It is the
closest thing in the tree to "~400 LOC of parallel abstraction".

It is out of scope here for two reasons. It is #4249's own 04.1/04.2 program,
gated on that issue's Phase-2 parity soak (#5396, which flipped
`session_shadow_reads` default-ON with a config migration); and its terminus —
serving reads from the store — points the opposite way from this branch's
non-negotiable zero-on-disk-change constraint. **It is also not S5's soak:** S5
compares free-function reads against trait reads, an entirely different
comparison. Track it as a #4249 phase-3 item ("retire the JSONL↔store dual path
once the Phase-2 parity soak declares parity") with a deletion-ledger row naming
the six sites above.

### S5 — Shadow soak, then remove the parallel path

One release with both paths live and a read-side comparison logged on mismatch
(never panic — a mismatch on a user's real transcript must degrade, not crash).
Then delete the redundant abstraction and update the ledger.

**Exit:** ledger row terminal; `docs/` transcript-derived-view note restated in
terms of the crate trait.

---

### S6 — Follow-ups from the §3.5 re-examination

Independent of S0–S5; each is separately shippable and none requires the
transcript decision.

1. **Unify dispatcher selection with the model layer** (§3.5.3). Merge with the
   parent spec's DS-5b — dispatcher choice, tool-call rendering, and tool-call
   parsing are one concern. Deliverable: the crate owns native-vs-prompt-guided;
   OpenHuman passes the `integrations_agent` override as policy and keeps
   `to_provider_messages` for the durable envelope. Est. host LOC removed when
   combined with DS-5b: **~2,400**.
2. **Upstream the message-list helpers** (§3.5.2): `tool_records_from_conversation`,
   `stamp_tool_failures`, `parse_tool_call_id`, `short_failure_detail`,
   `replace_last_assistant_reply` → crate `harness::message`. ~150 LOC. Small,
   uncontroversial, do it alongside DS-5b's parity-test port.
3. **Design a crate turn-preparation seam** (§3.5.4). A `ContextEnricher` /
   `TurnPreparation` pipeline — ordered, fallible, returning prompt fragments +
   metadata — so product enrichment registers instead of being hand-written into
   `turn()`. This is a **crate roadmap proposal, not a refactor**: write the
   design, get it accepted upstream, then migrate OpenHuman's enrichers
   (agent experience, recall citations, and announcements) onto it.
   Do not start by moving code.
4. **Continue adopting crate types field-by-field on `Agent`**, following the
   `workspace_descriptor: tinyagents::harness::workspace::WorkspaceDescriptor`
   precedent. This is how `Agent` converges without ever relocating — record
   each adopted field in the ledger.

**Exit:** items 1–2 landed; item 3 is an accepted-or-rejected upstream design
doc, not an open question; item 4 has a standing ledger section.

## 6. Explicitly out of scope

Recorded so a later audit does not re-litigate:

- `builder/` (1,699 + 666 + 96 + 55) — 21-domain host wiring, and it builds a
  *different object* than the crate's `AgentHarness` (§3.5.1). **Permanent
  host**, minus the dispatcher-selection carve-out in S6.1.
- `turn/core.rs`, `turn/tools.rs`, `turn/context.rs`, `turn/session_io.rs`,
  `turn/mod.rs`, `turn/graph.rs` — product turn preparation; the engine already
  left in WP-3 (§3.5.2). **Permanent host**, minus the ~150 LOC of message-list
  helpers in S6.2 and whatever S6.3's preparation seam later absorbs.
- `runtime.rs`, `types.rs` — `AgentSession` is a bag of host handles. **Permanent host.**
- `tool_progress.rs` (256) — already the C4 Step-5 deletion target; belongs to
  the progress-tracing workstream (parent spec DS-5), not here.
- `ChatMessage` itself — WP-1 settled this: it is the versioned on-disk record,
  and replacing it with crate `Message` changes existing users' data. Under
  Option A it does not move. Only Option B reopens it.

---

## 7. Risks

- **Silent model-context corruption is the top risk.** If `messages()` is wired
  to the display read path (or to a naive line replay), every compacted thread
  feeds the model duplicated pre-compaction history. It will not throw; it will
  degrade answers and inflate token cost. S3's compaction test is the gate.
- **`replace()`'s trait default is dangerous here.** The crate's default clears
  then re-appends, which the crate's own docs flag as non-atomic. Against an
  append-only durable log it is worse than non-atomic — it is destructive.
  Override it; never inherit it.
- **On-disk compatibility.** Existing installs have live `session_raw` files,
  including legacy `DDMMYYYY/` dirs. Resume must keep working across the change;
  the `read_transcript_legacy_md` path and the flat/dated fallback both need
  coverage in the soak.
- **Blast radius beyond `agent/`.** 24 files across `threads`, `session_import`,
  `learning`, `migrations`. The `threads/turn_state` derived-view contract is
  the fragile one.
- **≥ 80% diff-coverage merge gate.** S2/S4 touch many call sites; check
  `diff-cover` locally before pushing rather than discovering it in CI.
- **Two Cargo worlds** — any vendored bump (only under Option B) regenerates
  root **and** `app/src-tauri` lockfiles.
- **`GGML_NATIVE=OFF`** for local root-crate `cargo` runs on Apple Silicon.
- **GPL/crates.io boundary** — under Option B, the `session_raw` format becomes
  public API of a redistributed crate. Nothing product-specific (agent ids,
  OpenHuman path conventions, `.md` rendering) may cross.

---

## 8. Summary

| | |
| --- | --- |
| Proposed | move `harness/session/` → `tinyagents::sessions` |
| Verdict | **rejected as a unit** — ~8,600 of 11,073 prod LOC is host wiring; `builder/factory.rs` alone imports 21 OpenHuman domains |
| In scope | `transcript.rs` (1,997), `turn_checkpoint.rs` (105), `migration.rs` (373) — ≤ 2 host imports each |
| Key finding | the crate already ships `harness::memory::ChatHistory` + `harness::store` stream API; OpenHuman has a **second implementation**, not a missing home |
| Key constraint | crate `ChatHistory` cannot express compaction records, interrupted partials, or dual read paths — a naive impl corrupts model context |
| Recommendation | **Option A** — host backend behind the crate trait; zero on-disk change, reversible. Ledger is ≈ **−15 / +90 LOC** (S2–S4 overall +782 / −0), *not* the "~400 LOC removed" originally claimed — see §5 S4, [Where "~400 LOC" came from](#where-400-loc-came-from-and-why-it-is-struck) |
| Escalation | **Option B** (upstream `JsonlChatHistory`, ~2,100 LOC) only as a deliberate crate-roadmap decision |
| `builder/factory.rs` re-check (§3.5.1) | stays — builds `Agent` (40+ fields of product session state), not `AgentHarness` (6 fields of execution config); one real carve-out: dispatcher selection duplicates crate `with_native_tool_calling` |
| `turn/core.rs` re-check (§3.5.2) | stays — the engine left in WP-3; residue is product enrichment. ~150 LOC of message-list helpers are upstreamable |
| The generalizable ask | the missing artifact is a crate **turn-preparation seam** (S6.3), not a relocated file — move the shape down, keep the wiring up |
