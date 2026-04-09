# Subconscious Loop

Background task evaluation and execution system. Periodically checks user-defined and system tasks against the current workspace state, decides what to do, and either acts autonomously or escalates to the user.

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Heartbeat Engine                      │
│              (sleeps N minutes between ticks)            │
└──────────────────────┬──────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────┐
│                  Subconscious Engine                     │
│                                                         │
│  1. Load due tasks from SQLite                          │
│  2. Insert in_progress log entries                      │
│  3. Build situation report (memory + workspace state)   │
│  4. Evaluate tasks with local Ollama model              │
│  5. Execute decisions (act / noop / escalate)           │
│  6. Update log entries in place                         │
└─────────────────────────────────────────────────────────┘
                       │
           ┌───────────┼───────────┐
           ▼           ▼           ▼
         noop         act       escalate
        (skip)    (execute)   (agentic-v1)
```

### Key files

| File | Purpose |
|------|---------|
| `src/openhuman/heartbeat/engine.rs` | Periodic scheduler, delegates to subconscious engine |
| `src/openhuman/subconscious/engine.rs` | Core tick logic, state management, overlap guard |
| `src/openhuman/subconscious/executor.rs` | Task execution routing (local model vs agentic-v1) |
| `src/openhuman/subconscious/prompt.rs` | Prompt builders for evaluation and execution |
| `src/openhuman/subconscious/store.rs` | SQLite persistence (tasks, log, escalations) |
| `src/openhuman/subconscious/types.rs` | Data types and enums |
| `src/openhuman/subconscious/situation_report.rs` | Builds context from memory and workspace state |
| `src/openhuman/subconscious/global.rs` | Global singleton shared between heartbeat and RPC |
| `src/openhuman/subconscious/schemas.rs` | RPC endpoint handlers |
| `app/src/hooks/useSubconscious.ts` | Frontend hook for data fetching and actions |
| `app/src/pages/Intelligence.tsx` | UI rendering (Subconscious tab) |
| `app/src/utils/tauriCommands/subconscious.ts` | TypeScript RPC wrappers |

---

## Task Types

### System tasks

Seeded automatically on engine initialization. Cannot be deleted, only disabled.

Default system tasks:
- Check connected skills for errors or disconnections
- Review new memory updates for actionable items
- Monitor system health (Ollama, memory, connections)

Additional system tasks are imported from `HEARTBEAT.md` in the workspace directory (one task per `- ` line).

### User tasks

Added manually via the UI. Can be toggled on/off and deleted.

Examples:
- "Check urgent emails" (read-only)
- "Send daily summary to Slack" (write intent)
- "Summarize Notion updates" (read-only)

---

## Tick Lifecycle

### 1. Overlap guard

Each tick gets a monotonically increasing generation counter. If a new tick starts while the old one is still running (e.g., slow LLM call), the old tick's results are discarded and its `in_progress` log entries are marked as `cancelled`.

The heartbeat uses `tokio::time::sleep` (not `interval`) so ticks never stack up.

### 2. Load due tasks

Query: enabled, not completed, `next_run_at <= now` or never run.

### 3. Log as in_progress

Each due task gets a single log row inserted with `decision = "in_progress"`. This row is updated in place as the task progresses — no duplicate rows.

### 4. Evaluate with local model

The local Ollama model receives all due tasks + a situation report and returns a per-task decision:

| Decision | Meaning |
|----------|---------|
| `noop` | Nothing relevant right now |
| `act` | Something relevant found — execute the task |
| `escalate` | Needs deeper reasoning — hand off to agentic-v1 |

### 5. Execute

Routing depends on the decision and the task's intent:

```
Decision: noop
  → Update log to "noop", advance schedule

Decision: act
  ├─ Task has write intent (needs_tools = true)
  │   → Execute with local model
  │
  └─ Task is read-only
      → Execute with local model

Decision: escalate
  ├─ Task has write intent (needs_tools = true)
  │   → Run agentic-v1 with full permissions
  │   → No approval needed (user explicitly asked for the write action)
  │
  └─ Task is read-only
      → Run agentic-v1 in analysis-only mode
      → If response contains "RECOMMENDED ACTION:" (unsolicited write)
      │   → Create escalation for user approval
      │   → On approval → run agentic-v1 with full permissions
      └─ Otherwise → log result, done
```

### 6. Update log entry

The same row inserted in step 3 is updated to the final state:

| Decision | Dot color | Text |
|----------|-----------|------|
| `in_progress` | Blue (pulsing) | "Evaluating..." |
| `act` | Green | Result text |
| `noop` | Gray | "Nothing new" |
| `escalate` | Amber | "Waiting for approval" |
| `failed` | Coral | Error message |
| `cancelled` | Gray | "Cancelled" |
| `dismissed` | Gray | "Skipped" |

---

## Execution Models

### Local Ollama model

Used for:
- Task evaluation (all tasks, every tick)
- Text-only task execution (summarize, check, monitor, review)

No cost, no rate limits, runs on-device.

### agentic-v1 (cloud)

Used for:
- Tool-required task execution (send, post, delete, create)
- Analysis-only mode for read-only tasks escalated by the local model

Rate-limit retry: up to 3 attempts with exponential backoff (2s, 4s, 8s) on 429 errors.

---

## Approval Gate

Approval is only required when the AI wants to take a **write action that the user didn't explicitly request**.

| Task intent | AI wants to write | Approval needed? |
|-------------|-------------------|-----------------|
| "Send digest to Slack" (write) | Yes | No — user asked for it |
| "Check urgent emails" (read) | No | No — read-only result |
| "Check urgent emails" (read) | Yes (wants to forward them) | **Yes** — unsolicited write |

The approval flow:
1. agentic-v1 runs in analysis-only mode
2. Response contains `RECOMMENDED ACTION: Forward 3 urgent emails to #team-alerts`
3. Escalation card appears in UI under "Approval Needed"
4. User clicks "Go ahead" → agentic-v1 runs again with full permissions
5. Or user clicks "Skip" → nothing happens

### Skill-related escalations

Escalations related to skills (detected by keywords: skill, oauth, notion, gmail, integration, disconnect, re-auth) show a "Fix in Skills" button that navigates to the Skills page instead of "Go ahead".

---

## Failure Handling

### Consecutive failure counter

Tracked in `EngineState.consecutive_failures`. Increments when the entire LLM evaluation fails (Ollama down, network error). Resets to 0 on any successful tick. Surfaced in the UI status bar as "N failed" in coral.

Individual task execution failures do NOT increment this counter — they are logged per-task but the tick itself is considered successful.

### last_tick_at advancement

`last_tick_at` only advances on successful ticks. If the LLM evaluation fails or the tick is cancelled, `last_tick_at` stays unchanged so the next tick's situation report covers the same time range — nothing is missed.

---

## Configuration

In `config.toml` under `[heartbeat]`:

```toml
[heartbeat]
enabled = true              # Enable the heartbeat loop
interval_minutes = 5        # Tick interval (minimum 5)
inference_enabled = true    # Enable local model evaluation
context_budget_tokens = 40000  # Max tokens for situation report
```

Defaults: `enabled = true`, `interval_minutes = 5`, `inference_enabled = true`.

---

## SQLite Schema

Database: `{workspace_dir}/subconscious/subconscious.db`

### subconscious_tasks

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | UUID |
| title | TEXT | Task description |
| source | TEXT | `"system"` or `"user"` |
| recurrence | TEXT | `"pending"`, `"once"`, or `"cron:expr"` |
| enabled | INTEGER | 1 = active, 0 = paused |
| last_run_at | REAL | Unix timestamp of last evaluation |
| next_run_at | REAL | Unix timestamp of next scheduled run |
| completed | INTEGER | 1 = done (one-off tasks) |
| created_at | REAL | Unix timestamp |

### subconscious_log

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | UUID |
| task_id | TEXT | FK to tasks |
| tick_at | REAL | Unix timestamp of the tick |
| decision | TEXT | `in_progress`, `act`, `noop`, `escalate`, `failed`, `cancelled`, `dismissed` |
| result | TEXT | Result text or error message |
| duration_ms | INTEGER | Execution duration |
| created_at | REAL | Unix timestamp |

### subconscious_escalations

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | UUID |
| task_id | TEXT | FK to tasks |
| log_id | TEXT | FK to log entry |
| title | TEXT | Escalation title |
| description | TEXT | What needs approval |
| priority | TEXT | `critical`, `important`, `normal` |
| status | TEXT | `pending`, `approved`, `dismissed` |
| created_at | REAL | Unix timestamp |
| resolved_at | REAL | When approved/dismissed |

---

## RPC Endpoints

All under `openhuman.subconscious_*`:

| Method | Description |
|--------|-------------|
| `subconscious_status` | Get engine status (enabled, ticks, failures) |
| `subconscious_trigger` | Manually trigger a tick (runs in background, returns immediately) |
| `subconscious_tasks_list` | List all tasks |
| `subconscious_tasks_add` | Add a user task |
| `subconscious_tasks_update` | Update task (title, enabled, recurrence) |
| `subconscious_tasks_remove` | Remove a user task (system tasks can only be disabled) |
| `subconscious_log_list` | List activity log entries |
| `subconscious_escalations_list` | List escalations (filterable by status) |
| `subconscious_escalations_approve` | Approve and execute an escalation |
| `subconscious_escalations_dismiss` | Dismiss an escalation |

---

## UI (Intelligence Page → Subconscious Tab)

### Status bar

Shows: task count, total ticks, last tick time, consecutive failures (if > 0).

### Active Tasks

- **System tasks**: displayed as plain text with green dot and "default" badge. No controls.
- **User tasks**: toggle switch (enable/disable) + delete button on hover.
- **Add task**: text input + "Add" button at the bottom.

### Approval Needed

Amber cards for pending escalations. Each shows title, description, priority badge.
- **"Go ahead"**: approve and execute the write action.
- **"Fix in Skills"**: shown for skill-related escalations, navigates to Skills page.
- **"Skip"**: dismiss without executing.

### Activity Log

Chronological list of task evaluations. Each entry shows timestamp, colored dot, and result text. Auto-polls every 2s while any entries are `in_progress`.

### Run Now

Triggers a manual tick. The tick runs in the background — the RPC returns immediately and the UI polls for updates.
