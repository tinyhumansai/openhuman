# Master Chat — scoping (read-only investigation)

**Feature target.** A human asks the OpenHuman agent a question in a "Master chat".
OpenHuman either (a) answers from its own persisted session history with other agents,
or (b) asks an external agent on the human's behalf by DMing it under a session id
(new or existing), with the external reply threading back into the Master-chat answer.

**Grounding.** All `EXISTS` refs are against `upstream/main` (`git show upstream/main:<path>`,
fetched at investigation time — `upstream/main` @ `f49249360`). In-flight work accounted
for: **PR #4582 is already MERGED into main** (shared per-pair `session_key`, plus the
warn-only seq guard); **issue #4583 is OPEN** (robust monotonic ingest cursor — *not* yet
implemented, only the interim warn shipped); **plugin PR tiny.place#227** (one shared
session id per thread, reused on reply) is the peer-side counterpart the merged core code
already assumes.

> Terminology caution: in the code today, **"master" (`ChatKind::Master`)** is the window
> that aggregates *plain (non-envelope) DMs* from peers, and `orchestration_send_master_message`
> is the human→front-end **steering** send. The task's "Master chat" (human asks OpenHuman,
> OpenHuman answers *back to the human*) is a **superset** that is only partially realized by
> this window — see the gap analysis.

---

## WHAT EXISTS TODAY

Domain root: `src/openhuman/orchestration/` (declared in `mod.rs:16-34`; enabled behind
`config.orchestration.enabled`, schema `src/openhuman/config/schema/orchestration.rs:39`).
Startup wiring: `src/core/jsonrpc.rs:2294-2300` (ingest subscriber, wake subscriber, drain
supervisor).

### 1. Master vs Session windows model

- **`ChatKind`** — `Master | Subconscious | Session` (`types.rs:113-155`, `as_str`/`from_str`
  with Master as the safe default at `:150`).
- **`SessionEnvelopeV1`** — hand-rolled mirror of the tiny.place harness envelope
  (`types.rs:79-96`); `is_valid_v1` requires `envelope_version == "tinyplace.harness.session.v1"`
  and a non-empty `harness_session_id` (`types.rs:99-104`); `parse` returns `None` for any
  non-envelope body so it routes to Master (`types.rs:108-112`).
- **`session_key()`** (`types.rs:123-135`) — **PR #4582, on main**: returns
  `scope.wrapper_session_id` (the shared per-pair conversation id both peers stamp on every
  message for a thread), falling back to `harness_session_id` only for a legacy envelope.
  This is the sole inbound routing key.
- **`SessionEnvelopeV1::outgoing()`** (`types.rs:139-162`) — builds a v1 envelope with both
  `wrapper_session_id` and `harness_session_id` set to the same `session_id`, `role="owner"`,
  so a compliant peer threads its reply under the same id.
- **`classify_message`** (`ingest.rs:44-90`) — pure classifier: a parseable envelope →
  `ChatKind::Session` keyed by `session_key()` (`ingest.rs:47-75`, seq = `env.message.line`,
  source = `harness.provider`, label from folder scope, workspace from cwd); anything else →
  `ChatKind::Master`, `session_id="master"`, `role="user"`, `seq=0` (`ingest.rs:76-88`).
- **Persisted rows** — `OrchestrationSession` (`types.rs` `sessionId/agentId/source/label/
  workspace/lastSeq/...`) and `OrchestrationMessage` (`id/agentId/sessionId/chatKind/role/
  body/timestamp/seq`); `body` is decrypted plaintext ⇒ workspace-internal only.

### 2. Ingest → persist → wake pipeline

- **Bus subscribers** (`bus.rs`): `OrchestrationIngestSubscriber` (domain `tinyplace`,
  `bus.rs:60-92`) feeds `ingest_stream_message`; `OrchestrationWakeSubscriber` (domain `agent`,
  `bus.rs:112-133`) fans each persisted `OrchestrationSessionMessage` to the renderer socket
  **and** calls `ops::schedule_wake` — for **all** kinds except subconscious.
- **`ingest_one`** (`ingest.rs:171-247`):
  1. **linked-sender gate** — only DMs from paired (linked) agents are decrypted; unpaired
     senders are skipped *without* decrypt/ack so they stay in the Messaging UI mailbox
     (`ingest.rs:187-198`).
  2. **dedupe-before-decrypt** by relay `msg_id`, protecting the non-idempotent Signal ratchet;
     a duplicate re-acks but never re-decrypts (`ingest.rs:200-217`).
  3. decrypt-once → `classify_message` → `persist_message` (`ingest.rs:219-224`).
  4. on a new row: `acknowledge_message` + `publish_global(OrchestrationSessionMessage{...})`
     (`ingest.rs:226-242`).
- **`persist_message`** (`ingest.rs:92-149`): upserts the session row and inserts the message
  idempotently. Contains the **#4583 interim guard** — if `chat_kind == Session` and the inbound
  `seq <= existing last_seq`, it logs a `warn` (`ingest.rs:107-127`) but still persists
  (proved by test `persist_still_stores_a_lower_seq_in_the_same_wrapper_session`).
- **Delivery is poll-only**: `drain_mailbox_once` (`ingest.rs:264-310`) lists `/messages` every
  15 s via `start_message_drain_supervisor` (`ops.rs:129-145`) — relay DMs never hit the
  `/inbox/stream` WS, so the poller is the real inbound path.
- **`schedule_wake`** (`ops.rs:83-118`): per-session debounce via a generation counter
  (`ops.rs:58-78`); subconscious kind returns early (`ops.rs:96-98`); the last trigger within
  `debounce_ms` wins → `invoke_orchestration_graph`.
- **Per-session idempotence cursor** (`ops.rs:51-54`, `192-211`): `has_new_work` compares
  `latest_seq` against `kv["cursor:{agent}:{session}"]`; `advance_cursor` moves it **only after
  a completed, DM-sent cycle** (`ops.rs:518-522`). Store: `session_last_seq` (`store.rs:219`),
  `upsert_session` clamps `last_seq = MAX(old,new)` (`store.rs:169`), `ingest_cursor_lag`
  (`store.rs:385`). **There is no monotonic `next_session_seq` on main** (confirmed absent) —
  cursor and lag both key on `env.message.line`, which is the #4583 defect.

### 3. Wake graph nodes (what runs the LLM)

- **Graph shape** (`graph/mod.rs:1-30`, wired in `graph/build.rs:328-345`):
  `normalize → frontend → execute → compress → world_diff → frontend → send_dm →
  context_guard → done`. One `OrchestrationState` (`graph/state.rs:49-95`) is threaded and
  SQLite-checkpointed per super-step under thread `orchestration:<session_id>`
  (`build.rs:351-383`).
- **Injected `OrchestrationRuntime` seam** (`build.rs:44-61`) — every LLM/effect op is behind
  this trait; production impl `ProductionRuntime` in `ops.rs:550-817`.
- **`frontend` (router)** (`build.rs:149-211`): two-pass Quick-LLM. Pass 1 → `frontend_instruct`
  (`ops.rs:595-604`) → macro-instructions → `execute`. Pass 2 (once `agent_reply` present) →
  `frontend_compile` (`ops.rs:606-617`) → `channel_response` → `send_dm`. `max_supersteps`
  backstop guarantees termination (`build.rs:166-181`). Agent package: `frontend_agent/`
  (`prompt.md`, `agent.toml` tier `chat`, tools `defer_to_orchestrator` + `reply_to_channel`
  from `tools.rs`).
- **`execute` (reasoning core)** (`build.rs:217-231` / `ops.rs:619-640`): runs `reasoning_agent`
  with the current steering directive scoped in (`reasoning_agent/steering.rs`, `ops.rs:626-633`);
  produces `reply` + a `trace`. Package `reasoning_agent/agent.toml`: tier `reasoning`,
  subagents `researcher/code_executor/tools_agent`, tools **`spawn_async_subagent, wait,
  tinyplace_whoami, tinyplace_status, tinyplace_feed, current_time, resolve_time`** — i.e. only
  tiny.place **read** tools; **no DM-send tool, no session-mint tool.**
- **`compress`** (`ops.rs:642-707`): 20:1 summary of the trace → `compressed_history` row
  (`store.rs:474`).
- **`world_diff`** (`ops.rs:709-736`): append-only per-`(agent,session)` mutation timeline
  (`store.rs:515` `append_world_diff`, monotonic `seq`).
- **`context_guard`/`evict`** (`ops.rs:738-802`): utilization estimate; over threshold, evicts
  the oldest compressed summaries into memory-RAG under `path_scope = orchestration/<session>`
  (`ops.rs:764-784`).
- **`send_dm`** (`build.rs:273-294` / `ops.rs:804-816`): sends `channel_response` back to
  **`state.counterpart_agent_id`** (the session's peer). `dm_sent` latch prevents double-send.

### 4. The SEND path (OpenHuman → peer under a sessionId)

Two send surfaces exist, both routing through `tinyplace::handle_tinyplace_signal_send_message`:

- **Graph reply** — `ProductionRuntime::send_dm` (`ops.rs:804-816`) → `session_send_plaintext`
  (`ops.rs:822-835`): for a real session id it wraps the body in `SessionEnvelopeV1::outgoing`
  (so the peer threads its reply under the *same* id); `"master"`/`"subconscious"` stay plain.
  **This is a reply only — it always targets the current session's `counterpart_agent_id`;
  it cannot initiate a new outbound thread to a different agent or mint a new session id.**
- **Human steering send** — `orchestration_send_master_message` (`schemas.rs:108-118`, handler
  `:437-539`): body + optional `recipient` + optional `sessionId`. Recipient resolution
  (`schemas.rs:457-472`): explicit wins; else the session's contact (`store::session_agent_id`,
  `store.rs:368`); else the latest master peer (`store::latest_master_peer`, `store.rs:356`).
  With `sessionId` it wraps a v1 envelope (`session_envelope_plaintext`, `schemas.rs:425-435`)
  and mirrors into that session window; without it, plain into `"master"`. It then
  `notify_orchestration_message` (`schemas.rs:535`) — which, via the wake subscriber, **wakes
  the local graph** on that window.
- **Session mint** — `orchestration_sessions_create` (`schemas.rs:82-91`, handler `:347-380`)
  mints a fresh `uuid` session id for a contact, `source="user_created"`. **Renderer-driven
  only** — no agent-side or graph-side caller.

### 5. Session persistence + read surface + frontend

- **Store reads**: `list_sessions` (`store.rs:254`), `list_messages_by_session`
  (`store.rs:279`, session-key aggregated, paged by `before`), `list_recent_messages`
  (`store.rs:441`, chronological window used by the graph), `unread_count`/`mark_chat_read`
  (`store.rs:329`/`339`).
- **RPC surface** (`schemas.rs:23-71`): `orchestration_sessions_list`, `_sessions_create`,
  `_messages_list`, `_send_master_message`, `_mark_read`, `_status`, `_self_identity`,
  `_relay_info` — internal registry (renderer-only, never advertised to agents).
- **Frontend Master-chat surface**:
  - Client `app/src/lib/orchestration/orchestrationClient.ts:166-200` (`sessionsList`,
    `messagesList`, `sendMasterMessage`, `sessionsCreate`, `markRead`, `status`, ...).
  - Hook `app/src/lib/orchestration/useOrchestrationChats.ts` — `sendMessage` (`:260-303`):
    a **session** chat sends `{ body, recipient, sessionId }`; **master/subconscious** send
    `{ body }` (steering to the latest master peer). Optimistic append + reconcile via
    `loadMessages`/`loadSessions`.
  - UI: `app/src/components/intelligence/TinyPlaceOrchestrationTab.tsx` + `TinyPlaceRoster`,
    `InstanceCard`, `InstanceStatusDot`, `HarnessGlyph`, `SelfIdentityCard`, `RelayBadge`
    (roster of instance-shaped sessions; recent commits 0500aa9fb / 05a3c97b9).

### 6. Is cross-agent Session-window history fed to the LLM today?

**No.** The graph seeds from **one** window only: `seed_state` (`ops.rs:150-166`) calls
`list_recent_messages(agent_id, session_id, message_window)` and `OrchestrationState::seed`
(`state.rs:97-134`) folds just those messages + the single `counterpart_agent_id`. The
reasoning core's only cross-session recall is indirect (memory-RAG over evicted compressed
summaries at `orchestration/<session>`, and the `tinyplace_feed`/`_status` read tools). There
is **no** node or tool that loads/validates the OpenHuman↔agentX **session transcript(s)** as
explicit context for answering a question, and none that spans *multiple* sessions/peers.

---

## WHAT'S NEEDED (gap analysis)

Target flow: **human asks in Master chat → orchestration LOADS the relevant OpenHuman↔agent
session history (validate persistence) → runs the LLM over that history + the question → if an
external agent is needed, SEND a DM under a session id (reuse the per-pair id if a thread
exists, else mint) → the external reply threads back (shared-session-id model) → update the
Master-chat answer.**

Wired-vs-missing summary of the 5 sub-flows. **The `State at scoping` column is the
pre-PR baseline** (kept as historical context); the `Landed` column is what this PR
delivers.

| # | Sub-flow | State at scoping (pre-PR) | Landed in this PR |
|---|----------|---------------------------|-------------------|
| 1 | human→OpenHuman question intake + routing into the graph | **Partial** — `send_master_message` + master wake exist, but they reply *outbound to a peer*, not *back to the human*. No "answer the asker" surface. | ✅ W2 local-ask path + human-facing `master_agent`; answers land back in the Master window. |
| 2 | graph loads + validates cross-agent session history as context | **Missing** — single-window seed only. | ✅ `orchestration_list_sessions` / `orchestration_read_session` browse tools. |
| 3 | decision + tool for OpenHuman to ask an external agent | **Missing** — reasoning core has no send/ask tool. | ✅ `orchestration_send_to_agent` (linked-peers-only guardrail). |
| 4 | choose new-vs-existing session id for the outbound ask | **Missing** — `sessions_create` mints; no reuse-lookup; no agent caller. | ✅ reuse `latest_session_for_agent`, else mint fresh. |
| 5 | thread external reply back into the Master-chat answer | **Missing** — reply wakes the *sub-session* graph and replies to the peer; never correlated back to the originating Master question. | ✅ W7 correlation (process-global beacon) → reply surfaced as OpenHuman's own message. |

Session-persistence-validity concerns to carry through: **dedupe** (already solid —
`message_exists` before decrypt, `INSERT OR IGNORE`); **ordering/#4583** (cursor keyed on
`env.message.line` can silently drop the 2nd+ inbound message on a shared `wrapper_session_id`
— **blocks reliable reply-threading**, must be fixed before sub-flow 5 is trustworthy);
**stale/dead session ids** (no liveness/expiry on session rows); **shared-id reuse rules**
(#227/#4582 give inbound threading; outbound *initiation* id-choice is unspecified).

### Status update — PR #4599 (OPEN, `Closes #4583`, follow-up to #4582)

#4599 lands the **reliable reactive reply loop** (`ingest.rs`/`ops.rs`/`store.rs`/`tools.rs`
only). It closes the P0 foundation and adds plumbing the answer path reuses — but delivers
**none** of the target-flow gaps (it makes *peer→OpenHuman→peer* replies reliable, not
*human→OpenHuman→(history|external agent)→human*):

- **W1 → DONE**: `store::next_session_seq` = `MAX(seq)+1` per `(agent,session)`, stamped in
  `persist_message` on both `last_seq` and message `seq` (replaces the warn guard); also fixes
  the `cycle_id` collision.
- **Reply-content fix → DONE (new prerequisite for a *correct* answer)**:
  `tools::with_decision_capture` task-local captures the `reply_to_channel` /
  `defer_to_orchestrator` payload; `frontend_instruct`/`frontend_compile` prefer it over
  `run_single`'s trailing narration.
- **Answer-display plumbing → DONE (reused by W7/W10)**:
  `ProductionRuntime::persist_outgoing_reply` writes the reply `role=owner` (monotonic seq) +
  `notify_orchestration_message` → the reply now renders in `orchestration_messages_list` / the
  chat window.
- **Wake resilience → DONE**: `schedule_wake` retries with 5s/15s/45s backoff,
  checkpoint-resumed, bails if superseded.

Three follow-ups #4599 explicitly defers (its "Related") are folded in below as **F1/F2/F3**.

### Design pivot — agentic tools, not static seed (branch `feat/master-chat-orchestration-tools`)

Per the owner: the master orchestration layer should have **tools** to (1) browse its
OpenHuman↔agent session chats and (2) send messages on OpenHuman's behalf — rather than the
graph pre-loading history into the prompt. This **replaces W3/W4** ("seed history into state")
with on-demand read tools the reasoning core calls, and reframes W5 as the send tool.

**Shipped in this branch (read slice):**

- ✅ `orchestration_list_sessions` + `orchestration_read_session` read tools
  (`orchestration/tools.rs`), registered in `tools/ops.rs`, added to the `reasoning_agent`
  allowlist + prompt nudge. Read-only, concurrency-safe, workspace-internal store access via
  `store::{list_sessions,count_messages,list_recent_messages,list_messages_by_session}`.
  Delivers target half (a): answer from own history.

**Shipped in this branch (send slice — W5 + W6):**

- ✅ `orchestration_send_to_agent` (`orchestration/tools.rs`) — DM a peer on OpenHuman's
  behalf. **Guardrail: linked-peers-only** (`pairing::linked_agent_ids` OR an existing session
  with the peer) — refuses cold-DMs from the un-gated background origin. **Session id:
  reuse-or-mint per peer** via new `store::latest_session_for_agent` (reuse the peer's newest
  thread's shared `wrapper_session_id` so the reply threads back; else mint a uuid). Sends a v1
  envelope via `handle_tinyplace_signal_send_message`, records the outbound `role=owner`
  message + `notify_orchestration_message` (mirrors #4599's `persist_outgoing_reply`).
  `PermissionLevel::Write`. Added to the `reasoning_agent` allowlist + prompt.
- Verified: `cargo test openhuman::orchestration` → 64/64 (6 new); loader 85/85; lib clean.

**Shipped in this branch (reply-threading — W7, core-only):**

- ✅ One-shot outbound-ask correlation. For a local-master turn the `execute` node opens a
  **process-global origin beacon** (`tools::begin_master_origin`/`end_master_origin`) around the
  agent turn; `orchestration_send_to_agent` reads it and records
  `store::set_pending_ask((peer_agent, ask_session) → origin)`. A process-global is used instead of
  a task-local because the harness dispatches tool calls past an internal `tokio::spawn`, and
  task-locals do **not** cross a spawn — the earlier `with_origin_session` task-local silently never
  reached the tool, so the correlation never armed. The key is scoped by `(peer_agent, session)` so
  a legacy shared `wrapper_session_id` across peers can't misroute a reply. When the peer's reply
  lands, `invoke_with_runtime` correlates it and **finishes the cycle without running the reply
  graph** — no ping-pong. One-shot: the pending marker is consumed **only after the reply is durably
  surfaced** (`store::{pending_ask_origin,clear_pending_ask}`), so a transient store failure retries
  on the next drain instead of dropping the answer.
- ✅ Reply surfaced as **OpenHuman's own message**, not the peer's raw words. For a master-initiated
  ask the reply is run through the **tool-free `master_reporter`** (`report_peer_reply_to_master`) —
  the peer text is untrusted, so the reporter carries no tiny.place tools/sub-agents (no
  prompt-injection surface) and emits an `assistant` message in the Master window. Peer/A2A origins
  keep the deterministic raw `thread_reply_to_origin`.
- ✅ Fire-and-forget: `orchestration_send_to_agent` returns an immediate ack and the `master_agent`
  prompt forbids polling/`read_session` for the reply, so W7 is the sole async reporter (no
  duplicate surfacing).
- Verified: `cargo test openhuman::orchestration` green (incl. `outbound_ask_reply_threads_to_
  origin_and_skips_the_reply_graph`, `pending_ask_correlation_is_one_shot`,
  `master_origin_beacon_sets_and_clears`); full loop proven live on staging.
- **Limitation (needs F3):** correlation is a pragmatic 1:1 request/response — it assumes the
  *next* inbound message on the ask session is the answer. Many-in-flight / interleaved replies
  need an explicit envelope `inReplyTo`/`fromSession` (F3, cross-repo). The process-global beacon is
  safe because local-master wakes are serialized; a concurrent A2A send during a master turn is the
  one documented edge (single-user desktop, non-fatal). Peer-session (A2A) W7 still rides the
  best-effort task-local.

**Still to do here:** RPC/UI surface for the human-facing master ask/answer (W8–W12) and
perf/robustness **F1/F2**; robust correlation **F3** (cross-repo).

### Prioritized work-item CHECKLIST (ordered Rust core → JSON-RPC → UI → tests) — remaining after #4599

**P0 — foundation / correctness**

- [x] **W1 — Fix the #4583 wake-cursor/seq decoupling. — DONE in #4599** (`store::next_session_seq`
  stamped in `persist_message`). Note: #4599 keys `last_seq`/message `seq` on the ordinal;
  `has_new_work`/`advance_cursor`/`ingest_cursor_lag` already ride `last_seq`, so they inherit
  the fix. No remaining work.

- [ ] **W2 — Master-chat "ask" intake distinct from steering.** Introduce a question record
  (role `owner`, a `pending`/answered marker) and route it into the graph as *the question to
  answer for the human*, separate from `send_master_message`'s outbound peer-steering.
  *Where:* `orchestration/{types.rs,store.rs,ops.rs,schemas.rs}` (+ maybe a `master` chat mode).
  *Size:* **M.** *Deps:* W1.

**P1 — the answer path**

- [ ] **W3 — Cross-agent session-history loader + validity check.** A store read that gathers
  the relevant OpenHuman↔agent transcript(s) (by peer and/or across peers), validates ordering
  (monotonic seq from W1), drops dead/stale sessions, and returns a bounded, LLM-ready context.
  *Where:* `orchestration/store.rs` (new `load_history_for_question`) + `ops.rs::seed_state`
  (extend state with a `history_context`). *Size:* **M/L.** *Deps:* #4599 (monotonic seq).

- [ ] **W4 — Feed the history into the reasoning/front-end prompt.** Extend `OrchestrationState`
  (`graph/state.rs`) with the loaded history and render it in `frontend_instruct` / `execute`
  prompts (`ops.rs:595-640`). *Where:* `orchestration/graph/state.rs`, `ops.rs`. *Size:* **S/M.**
  *Deps:* W3.

- [ ] **W5 — "Ask an external agent" decision + tool.** A new domain tool (e.g.
  `ask_external_agent { recipient, question, sessionId? }`) that the reasoning core can call;
  it performs the outbound session-scoped send. Add to `reasoning_agent/agent.toml` allowlist
  and gate it (approval/linked-only). *Where:* `orchestration/tools.rs` (+ re-export in
  `tools/mod.rs`), `reasoning_agent/agent.toml`, reuse `session_send_plaintext`/
  `handle_tinyplace_signal_send_message`. *Size:* **M.** *Deps:* W2.

- [ ] **W6 — New-vs-existing session-id chooser for the outbound ask.** Resolve "do I already
  have a live thread with peer X?" → reuse that `wrapper_session_id`; else mint (uuid, per
  `sessions_create`) and record the `(peer → session_id)` mapping. Encode the #227/#4582 reuse
  rule (one shared id per thread; both peers reuse on reply). *Where:* `orchestration/store.rs`
  (a `find_or_create_session_for_peer`), consumed by W5. *Size:* **M.** *Deps:* W5.

- [ ] **W7 — Thread the external reply back into the Master-chat answer.** Correlate the
  inbound session reply (arriving under the shared id, `ChatKind::Session`, waking that
  sub-session's graph) back to the originating Master question, and update/emit the Master-chat
  answer instead of (or in addition to) replying to the peer. Needs a pending-ask ↔ session-id
  correlation table and a resume/notify into the master window. Reuse #4599's
  `persist_outgoing_reply`/`notify_orchestration_message` for surfacing the answer; the
  correlation primitive is **F3** (envelope `inReplyTo`/`fromSession`). *Where:*
  `orchestration/store.rs` (correlation kv/table), `ops.rs` (wake handling for a correlated
  session), `bus.rs`. *Size:* **L.** *Deps:* W2, W5, W6, **F3**. *Risk:* highest — cross-graph
  correlation + the reactive `send_dm`-always-to-counterpart assumption in `build.rs:273-294`
  must be relaxed.

**P2 — JSON-RPC surface**

- [ ] **W8 — RPC for master-chat ask + answer polling.** e.g. `orchestration_ask_master`
  (submit question) and answer surfacing via existing `messages_list`/socket, plus a pending
  status. *Where:* `orchestration/schemas.rs` (new schema + handler; register in
  `all_*controllers`). *Size:* **M.** *Deps:* W2, W7.

- [ ] **W9 — Extend `orchestration_status`/session DTOs** with pending-ask + external-ask
  in-flight signals for the UI. *Where:* `schemas.rs`. *Size:* **S.** *Deps:* W8.

**P3 — UI**

- [ ] **W10 — Master-chat ask/answer UX.** Client method + hook wiring so a master question
  shows "OpenHuman is asking @peer…" and the threaded answer updates in place. *Where:*
  `app/src/lib/orchestration/orchestrationClient.ts`, `useOrchestrationChats.ts`,
  `TinyPlaceOrchestrationTab.tsx`. *Size:* **M.** *Deps:* W8. *Note:* i18n keys in `en.ts` +
  all locales; no dynamic imports.

**P4 — tests (each item lands with its own unit tests; these are the cross-cutting suites)**

- [ ] **W11 — Rust: threading correctness.** (a,b) seq/cursor line-0 + reuse cases are already
  covered by #4599 (`persist_stamps_monotonic_ingest_seq_so_line_zero_dms_still_wake`) — only
  (c) remains: outbound ask reuses an existing per-pair id and the reply correlates back to the
  originating master question. *Where:* `orchestration/{ops.rs,store.rs}` inline tests +
  `tests/json_rpc_e2e.rs`. *Size:* **M.** *Deps:* W6, W7.
- [ ] **W12 — JSON-RPC E2E + Vitest** for ask→answer round-trip (mock peer reply). *Where:*
  `scripts/test-rust-with-mock.sh`, `app/src/**/*.test.tsx`. *Size:* **M.** *Deps:* W8, W10.

**F — deferred by #4599 (fold into the target work)**

- [ ] **F1 — `send_dm` before `compress`/`world_diff`** to cut ~20s time-to-reply (reorder the
  graph edges so the outbound reply dispatches before the post-processing tail). *Where:*
  `orchestration/graph/build.rs` (edge wiring). *Size:* **S.** *Deps:* none. *Note:* perf, not
  blocking, but improves the human-facing answer latency the target cares about.
- [ ] **F2 — Scope the checkpoint thread id to `(agent, session)`.** Today it is
  `orchestration:<session_id>` (`build.rs:358`), so two peers sharing a session id can collide
  on checkpoints. *Where:* `orchestration/graph/build.rs::run_orchestration_graph`. *Size:* **S.**
  *Deps:* none. *Correctness* — do alongside W6/W7 (which multiply active sessions per id).
- [ ] **F3 — `SessionEnvelopeV1` `inReplyTo` / `fromSession` correlation.** The primitive W7
  needs to tie an inbound reply to the exact outbound ask (vs. inferring from session id alone),
  and to drive a `send_and_wait`-style block. *Where:* `orchestration/types.rs`
  (`SessionEnvelopeV1`), plus **tiny.place SDK / plugin #227 peer coordination**. *Size:* **M**
  (core) **+ external.** *Deps:* peer/SDK support. *Risk:* cross-repo — the blocking dependency
  for a clean W7.

---

## Open questions / risks

1. **Semantic overload of "master".** The window is currently peer-plain-DM aggregation +
   human steering; the target needs a human-question/answer channel. Decide whether to reuse
   `ChatKind::Master` or add a mode — affects W2/W7 and the wake router's terminate predicate.
2. **`send_dm` is unconditional-to-counterpart** (`build.rs:273-294`). Sub-flow 5 requires the
   graph to sometimes answer the human instead of/along with the peer — a structural change to
   the terminal node, not just a new tool.
3. **#4583 silent-drop — RESOLVED by #4599** (`next_session_seq` monotonic ordinal). Build
   reply-threading on `last_seq`/message `seq`, not the wire `line`.
4. **Outbound id-choice reuse rule is peer-side (#227) only in code.** The core has no
   "find existing thread with peer X" lookup; W6 must define freshness/liveness (ties to the
   stale-session-id concern — session rows have no expiry today).
5. **`env.message.line` is now ignored for ordering** (#4599). Its value no longer matters to
   the wake cursor; correlation (W7) must ride the store seq / an explicit `inReplyTo` (F3),
   not `line`.
6. **Approval/security:** an agent-initiated outbound DM (W5) is a new external-effect tool;
   must respect the linked-agent gate (`ingest.rs:187-198`) and the approval gate — scope its
   `CommandClass`/tier deliberately.
