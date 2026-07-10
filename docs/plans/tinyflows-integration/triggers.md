# Trigger taxonomy & the cron → workflows unification

> Companion to [README.md](README.md) (the tinyflows completion POA). This doc answers two questions: **what kinds of triggers can this platform offer**, and **how do we migrate cron so that "everything automated is a workflow"**.

## 0. Thesis

Almost every automated behavior in OpenHuman is already shaped like *event → conditions → actions*: cron jobs, Composio app events, channel-message reactions, subconscious escalations, meeting follow-ups, notification triage. Today each has bespoke plumbing. tinyflows gives us one uniform substrate (a tinyagents graph per run, durable, observable, approval-gated), and the event bus (`src/core/event_bus/events.rs`, ~130 `DomainEvent` variants) is already the spine every one of those signals travels on.

**End state**: *Workflows are the single user-facing automation concept.* A trigger is just a subscription that seeds a flow run; the trigger catalog is a curated projection of the event bus plus time and manual entry points.

## 1. What the engine already models

`tinyflows::model::TriggerKind` (declarative — the host fires them): `manual`, `schedule`, `webhook`, `app_event`, `form`, `execute_by_workflow`, `chat_message`, `evaluation`, `system`.

Host dispatch status today (`flows/bus.rs`):

| Kind | Status |
| --- | --- |
| `manual` | ✅ `flows_run` RPC / UI Run button |
| `schedule` | ✅ cron `JobType::Flow` → `FlowScheduleTick` |
| `app_event` | ✅ `ComposioTriggerReceived` matching (toolkit + trigger_slug) |
| `webhook` | ⏸ deferred (Composio is the webhook story — README Phase 1a) |
| `chat_message`, `form`, `execute_by_workflow`, `evaluation`, `system` | ❌ no dispatcher |

The gap is not engine capability — it's host dispatchers and a catalog. Crucially, `system` is a free config surface: we can build **one generic event-trigger dispatcher** instead of nine bespoke ones.

## 2. Proposed trigger catalog

Organized by source. "Backing event" = the existing `DomainEvent` variant(s) that would feed the dispatcher.

### 2.1 Time (`schedule`)

Already live via cron. Expose the full richness cron already has (`CronJob` in `cron/types.rs`):

- **Cron expression** (today's path) and **interval** ("every 15 min") sugar.
- **One-shot** ("run once at T") — cron's `delete_after_run` already models this; surface it in trigger config.
- **Windowed schedules** (weekdays only, quiet hours) — respects `scheduler_gate`.

### 2.2 Manual & parameterized (`manual`, `form`)

- **Manual**: Run button / RPC / agent-initiated (`flows_run` with input).
- **Form**: a manual run with a typed input schema. Trigger config declares fields; the UI renders a form; submission becomes the trigger payload. This is n8n's Form trigger and gives every workflow a shareable "mini-app" entry point. Cheap to ship: validation + a generated form in the canvas/run dialog.

### 2.3 External apps (`app_event`, `webhook`)

- **Composio triggers** (live): Gmail new message, Slack event, GitHub push, calendar event, … the platform delivers; we match toolkit/slug. Work remaining is subscription lifecycle + catalog surfacing (README Phase 1a).
- **Raw webhook** (deferred): non-Composio HTTP callers via `webhooks::ops` tunnels. Backlog until demanded.

### 2.4 Conversation (`chat_message`)

Backing events: `ChannelInboundMessage` / `ChannelMessageReceived` / `ChannelReactionReceived`.

- **Message received** with filters: provider/channel id, sender, regex or keyword, mention-of-me, DM vs group.
- **Reaction added** (e.g. ✅ on a message → file it as a task).
- Guardrail: these runs consume untrusted text — `prompt_injection` screening before any `agent` node, and a per-flow debounce/concurrency cap (the `app_event` dispatcher's guard generalizes).

### 2.5 Agent & task lifecycle (`system`)

Backing events: `AgentTurnCompleted`, `SubagentCompleted`/`SubagentFailed`, `ApprovalDecided`, `TaskSourceTaskIngested`, `TaskRunReclaimed`, `ThreadGoalUpdated`.

Examples: "when any subagent fails, post a summary to my ops channel"; "when a task is ingested from a task source, enrich and label it"; "when an approval is denied, notify the requester thread".

### 2.6 Memory & knowledge (`system`)

Backing events: `MemoryStored`, `MemoryIngestionCompleted`, `MemoryDiffComputed`, `DocumentCanonicalized`, `TreeSummarizerHourCompleted`.

Examples: "when a document is canonicalized, generate an abstract and store it"; "when the hourly summary lands, check for action items".

### 2.7 Meetings & voice (`system`)

Backing events: `MeetingSessionCreated`, `MeetingSummaryGenerated`, `BackendMeetTranscript`, `PttTranscriptCommitted`, `MeetingAutoJoinTriggered`.

Examples: "when a meeting summary is generated, extract action items → create tasks → email attendees" — the flagship demo workflow.

### 2.8 Notifications & devices (`system`)

Backing events: `NotificationIngested`/`NotificationTriaged`, `DevicePaired`, `DevicePeerOnline`/`Offline`.

Examples: triage automations ("when a notification from app X is ingested, decide urgent/ignore"), presence automations ("when my phone comes online, sync …").

### 2.9 Platform & health (`system`)

Backing events: `SystemStartup`, `HealthChanged`, `HealthRestarted`, `AutonomyConfigChanged`, `McpServerDisconnected`, `ProviderApiKeyRejected`, `EmbeddingModelUnhealthy`.

Examples: self-healing/maintenance workflows ("on health degradation, run doctor and report"). These should default to `require_approval` off but notification-heavy.

### 2.10 Subconscious escalations (`system`)

Backing events: `SubconsciousTriggerProcessed`, `TriggerEvaluated`, `TriggerEscalated`.

The subconscious domain is itself a mini automation engine (evaluate → escalate). Long-term convergence candidate: an escalation's *action* becomes "run flow X", making workflows the actuator layer for subconscious signals rather than a parallel system.

### 2.11 Workflow-to-workflow (`execute_by_workflow`, `evaluation`)

- `execute_by_workflow`: fired when another flow's `sub_workflow`/by-id call targets this flow (README Phase 4b) — enables composition and shared "library" flows.
- `evaluation`: reserved for eval-harness runs of a flow (regression-test a workflow against recorded inputs); pairs with `dry_run_workflow` (README Phase 5b).

## 3. Design: one generic event dispatcher, not N bespoke ones

Adding a hand-written `handle_*` per event (like `handle_app_event`) doesn't scale to §2.5–2.10. Proposal:

1. **Trigger catalog registry** (new, `flows/trigger_catalog.rs`): a curated allowlist of `DomainEvent` variants exposed as triggers — `{ key: "meeting.summary_generated", event: <variant match>, payload_schema, description, risk_class }`. Only cataloged events are subscribable; the raw bus is never exposed wholesale.
2. **Generic dispatcher** in `FlowTriggerSubscriber`: one match arm per cataloged domain that projects the event into a stable JSON payload, then matches enabled flows whose trigger is `kind: system, config: { event: "<key>", filter: "=<jq expr>" }`. The existing jq engine (`tinyflows::expr`) evaluates the optional filter against the payload — no new expression language.
3. **RPC** `flows_list_trigger_catalog()` → feeds the Phase 3 trigger-config UI and the Phase 5 builder agent (so prompts like "when a meeting ends…" ground to a real catalog key).
4. **Guardrails** (non-negotiable, enforced in the dispatcher):
   - **Loop prevention**: flow runs emit events too; runs triggered by a flow-originated event carry a provenance chain with a max depth (reuse tinyagents `root_run_id` lineage); a flow can never trigger itself.
   - **Debounce/rate limits**: per-flow trigger token bucket + the existing per-flow concurrency guard.
   - **Payload hygiene**: catalog projection strips PII-ish fields by default; `prompt_injection` screening before agent nodes.
   - **Risk classes**: catalog entries tagged (e.g. conversation events = untrusted-input class → forces screening; health events = internal class); validation surfaces the class in the UI.

## 4. Cron → workflows migration

Today `cron` runs three job types (`cron/types.rs::JobType`): `Shell`, `Agent`, and `Flow` (already just a tick-publisher for flows). The inversion: **cron stops being a product surface and becomes the schedule service for workflows.**

**M1 — Model mapping.** Every legacy job is a one-node flow:

- `JobType::Shell { command }` → flow: `trigger(schedule)` → `code` node (or a dedicated `shell` tool_call, subject to the same `classify_command` gating cron uses today).
- `JobType::Agent { prompt, agent_id, model, session_target }` → flow: `trigger(schedule)` → `agent` node (prompt/model in config; `agent_id` maps to the Phase 5+ agent-definition reference once engine sub-ports land — until then, definition prompt inlined).
- `DeliveryConfig` / `session_target` (post result to main thread vs isolated) → flow-level `delivery` setting or an explicit terminal "notify/post" node. Prefer the explicit node: it's visible on the canvas, which is the whole point.
- `delete_after_run` → one-shot schedule config (§2.1).

**M2 — Dual-write bridge.** `cron_add`/`cron_update` agent tools and RPCs keep working but create flows under the hood (a `legacy_cron` tag preserves round-tripping for `cron_list`). New UI creation always goes through flows.

**M3 — Backfill migration.** One-time migration (per-user, on upgrade): convert existing enabled shell/agent jobs into flows, preserving ids in metadata, next_run continuity, and run history linkage (`CronRun` rows stay readable; new runs are `FlowRun`s). Feature-flag + rollback window before deleting the legacy paths.

**M4 — Surface consolidation.** Settings → Automations lists flows only; cron internals (`scheduler.rs`, jobs table) remain as the tick engine for `JobType::Flow`, which becomes the *only* job type. The `cron_*` tool family shrinks to a thin alias the agent can still call, documented as "creates a scheduled workflow".

**What cron keeps**: the single tokio scheduler loop, persistence of next-fire times, catch-up semantics, `scheduler_gate`. That's infrastructure — workflows are the product on top of it.

**Sequencing**: M1/M2 fit inside README Phase 1 (they're mostly `flows`-side); M3/M4 after the canvas + run inspector are solid (users must be able to *see* what their migrated jobs became), i.e. post-Phase 3.

## 5. Rollout order for new trigger kinds

1. `form` + one-shot/interval schedule sugar (cheap, immediately useful).
2. Trigger catalog + generic `system` dispatcher, seeded with a small vetted set: `meeting.summary_generated`, `notification.triaged`, `subagent.failed`, `task.ingested`, `document.canonicalized`.
3. `chat_message` (needs the untrusted-input guardrails first).
4. Cron migration M1–M2, then M3–M4.
5. `execute_by_workflow` (with Phase 4b sub-workflow-by-id), `evaluation` last.

Each new dispatcher lands with: E2E test in `tests/json_rpc_e2e.rs`, catalog entry with payload schema, validation warning removal (the kind moves from "declared but never fires" to live), and a template (README Phase 4c) demonstrating it.
