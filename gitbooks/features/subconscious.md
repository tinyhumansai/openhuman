---
description: >-
  Background loop that evaluates user / system tasks against the workspace and
  decides what to do.
icon: loader
---

# Subconscious Loop

A background task evaluation and execution system. On a periodic tick, it loads a list of user-defined and system tasks, reads the current state of your workspace, decides what to do about each one, and either acts autonomously or escalates to you for approval.

Think of it as the agent's idle thread: the part that keeps thinking after you've stopped typing.

***

## How a tick works

```
┌─────────────────────────────────────────────────────────┐
│                    Heartbeat                            │
│           (sleeps a few minutes between ticks)          │
└──────────────────────┬──────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────┐
│                  Subconscious Engine                    │
│                                                         │
│  1. Load due tasks                                      │
│  2. Mark each one in-progress                           │
│  3. Build a situation report (memory + workspace)       │
│  4. Evaluate every task with the local model            │
│  5. Execute the decision (act / noop / escalate)        │
│  6. Write the outcome back to the activity log          │
└─────────────────────────────────────────────────────────┘
                       │
           ┌───────────┼───────────┐
           ▼           ▼           ▼
         noop         act       escalate
        (skip)    (execute)   (deeper agent)
```

Each tick is independent. If a tick is still running when the next one starts (slow model call, network blip), the new tick takes over and the old one's in-progress entries are marked cancelled. Ticks never stack.

***

## Task types

### System tasks

Seeded automatically when the engine starts. Cannot be deleted, only disabled. The defaults cover things you'd want any assistant watching for:

* Check connected skills for errors or disconnections
* Review new memory updates for actionable items
* Monitor system health (local model, memory, connections)

You can extend the system task set by listing additional ones in a `HEARTBEAT.md` file in your workspace, one task per line.

### User tasks

Anything you add manually from the UI. Toggle on/off, edit, delete. Examples:

* "Check urgent emails" (read-only)
* "Send daily summary to Slack" (write intent)
* "Summarize Notion updates" (read-only)

***

## Decisions

For every due task, the local model returns one of three decisions:

| Decision | Meaning                                             |
| -------- | --------------------------------------------------- |
| Skip     | Nothing relevant right now                          |
| Act      | Something relevant found, execute the task          |
| Escalate | Needs deeper reasoning, hand off to the cloud agent |

How that decision gets executed depends on whether the task has **write intent** (it asks the agent to take an action) or is **read-only** (it asks the agent to look and report):

```
Decision: Skip
  → Log "nothing new", schedule the next run

Decision: Act
  → Execute on the local model (read or write)

Decision: Escalate
  ├─ Write-intent task
  │   → Run the cloud agent with full permissions
  │   → No approval needed (you explicitly asked for the action)
  │
  └─ Read-only task
      → Run the cloud agent in analysis-only mode
      → If the agent surfaces an unsolicited recommended action
      │   → Create an escalation card for your approval
      │   → On approval → re-run with full permissions
      └─ Otherwise → log result, done
```

Every task evaluation lands in the activity log with a colored dot and a short status:

| State             | Color          | Text                   |
| ----------------- | -------------- | ---------------------- |
| In progress       | Blue (pulsing) | "Evaluating…"          |
| Acted             | Green          | Result text            |
| Skipped           | Gray           | "Nothing new"          |
| Awaiting approval | Amber          | "Waiting for approval" |
| Failed            | Coral          | Error message          |
| Cancelled         | Gray           | "Cancelled"            |
| Dismissed         | Gray           | "Skipped"              |

***

## Two models, one loop

| Stage                                  | Where it runs           | Why                                          |
| -------------------------------------- | ----------------------- | -------------------------------------------- |
| Per-task evaluation (every tick)       | Local model (Ollama)    | Free, no rate limit, fine on-device          |
| Text-only execution (summarize, check) | Local model             | Same                                         |
| Tool-using execution (send, post, …)   | Cloud agent             | Tools, larger context, retries on rate-limit |
| Analysis mode for escalated reads      | Cloud agent (read-only) | Deeper reasoning when the local model defers |

The split keeps the loop cheap: you only pay for cloud calls when a task actually needs them.

***

## Approval gate

Approval is only required when the agent wants to take a **write action that you didn't explicitly ask for**.

| Task intent                    | Agent wants to write | Approval needed?           |
| ------------------------------ | -------------------- | -------------------------- |
| "Send digest to Slack" (write) | Yes                  | No, you asked for it       |
| "Check urgent emails" (read)   | No                   | No, read-only result       |
| "Check urgent emails" (read)   | Yes (forward them)   | **Yes**, unsolicited write |

The approval flow:

1. The cloud agent runs in analysis-only mode.
2. It surfaces a recommendation, e.g. _"forward 3 urgent emails to #team-alerts."_
3. An escalation card appears in the UI under **Approval Needed**.
4. **Go ahead** re-runs with full permissions.
5. **Skip** does nothing.

Skill-related escalations (broken integration, expired OAuth, missing scope) show a **Fix in Skills** button that takes you straight to the Skills page instead.

***

## Failure handling

A failure counter tracks consecutive ticks where the whole evaluation step failed (local model down, network out). It resets to zero on any successful tick and shows up in the UI status bar in coral when non-zero.

Per-task failures don't trip this counter, the tick itself is still considered successful.

If a tick fails or is cancelled, the engine doesn't advance its "last seen" timestamp, so the next successful tick covers the same window. Nothing in your workspace gets skipped.

***

## Configuration

The loop is configurable in the desktop app:

* **Enable / disable.** Turn the entire background loop on or off.
* **Tick interval.** How often a tick fires. Defaults to 5 minutes; that's also the minimum.
* **Inference.** Whether the local model evaluates tasks each tick. Disable this if you'd rather only run things via the manual **Run Now** button.
* **Context budget.** How much of the workspace situation report can be passed in at once. The default is sane; raise it for richer context, lower it for tighter cost.

***

## In the UI

Lives under **Intelligence → Subconscious**.

* **Status bar.** Task count, total ticks, last tick time, failure counter (if any).
* **Active Tasks.** System tasks (read-only, with a "default" badge) and your own tasks (toggle + delete).
* **Approval Needed.** Amber cards for pending escalations. Each has a title, description, and priority. Buttons: **Go ahead**, **Fix in Skills** (when relevant), or **Skip**.
* **Activity Log.** Chronological feed of every task evaluation, colored dot + result. Auto-refreshes while anything is in progress.
* **Run Now.** Manually trigger a tick. Returns immediately; the UI polls for the result.

***

## See also

* [Memory Tree](obsidian-wiki/memory-tree.md), what the situation report reads from.
* [Auto-fetch from Integrations](obsidian-wiki/auto-fetch.md), how the workspace stays fresh between ticks.
* [Local AI (optional)](model-routing/local-ai.md), the on-device model that powers evaluation.

***

## The split-brain layer

The subconscious does more than housekeep. It **steers**. When your agent participates in [tiny.place orchestration sessions](tinyplace.md) (agent-to-agent collaboration), inbound traffic runs through a split-brain wake graph:

* A **fast reflex agent** triages every message in seconds: reply immediately, or hand the deep core a concise brief (a *what*, never a *how*).
* A **deep reasoning core** does the real multi-step work, delegating to parallel sub-agent workers, with a hard superstep cap guaranteeing termination.
* Long sessions stay bounded by **20:1 history compression** plus a rolling world-state diff with utilization-based eviction.

On its periodic tick, the subconscious reviews that compressed history and world diff and injects a short, dense **steering directive** (capped at ~900 characters, expiring after ~20 reasoning cycles) into the reasoning core's system prompt. This keeps the always-on layer aligned with *your* goals. The subconscious itself is strictly offline: it never contacts anyone and never takes external actions; ticks that reacted to external changes run **tainted**, so the approval gate refuses external-effect tools.

***

## The morning briefing

The subconscious's most visible output is the **personalized morning briefing**: a daily digest that greets you by name, matches your actual local hour (no "good morning" at 4 PM), and sorts the last 24 hours plus what's ahead into four fixed buckets: **Highlights**, **Action items**, **Mentions**, and **FYI**. It reads your recent Memory Tree window and live calendar/email/task data, never fabricates, keeps senders-and-subjects privacy discipline, and collapses to a one-line "nothing pressing" on quiet days. Enable it under **Automation → Morning Briefing**.
