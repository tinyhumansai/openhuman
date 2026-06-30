---
description: >-
  The notification center, the Activity transparency hub, and the Routines
  scheduler — everything OpenHuman tells you about, and everything it does in
  the background.
icon: bell
---

# Notifications & Activity

OpenHuman surfaces two kinds of "what's happening" in one place: **notifications** (things you should look at — an important Slack message, a failed webhook, a high-priority email) and **activity** (a transparent ledger of what the agent did on its own while you weren't watching). This page covers the notification center, the Activity hub that fronts it, and the Routines screen for managing scheduled automations.

***

## Notification Center

The notification center is fed by two independent streams that render side by side under **Activity → Alerts**.

### Integration notifications

Notifications captured from connected accounts (Gmail, Slack, WhatsApp, Discord, …) are ingested through the `notification.ingest` RPC, persisted to a per-workspace SQLite store, and then **triaged by a local LLM in the background**. Ingest returns immediately; triage runs in a spawned task and back-fills the score a moment later, so a freshly arrived item can briefly show as unscored.

Triage assigns each notification an **action**, which maps to a fixed 0.0–1.0 importance score:

| Triage action | Score | What it means                                    |
| ------------- | ----- | ------------------------------------------------ |
| `drop`        | 0.10  | Noise — not worth surfacing                      |
| `acknowledge` | 0.35  | Low value, informational                         |
| `react`       | 0.65  | Worth a follow-up                                |
| `escalate`    | 0.90  | High priority — hand to the agent                |

Only `react` and `escalate` are considered "routed" actions; `drop` and `acknowledge` stay quiet. Each ingested item carries a one-sentence `triage_reason` justifying the classifier's call, plus a lifecycle status: **unread → read → acted → dismissed**. Duplicate content received within a 60-second window collapses to a single entry.

### System (core-bridge) notifications

The second stream translates selected internal events into compact, user-facing alerts and pushes them over the socket bridge as they happen. These are persisted before broadcast, so anything fired while the app was closed syncs down on the next open. Each carries a **category** and an in-app deep link:

| Source event           | Category   | Surfaces when                                   |
| ---------------------- | ---------- | ----------------------------------------------- |
| Cron job completed     | Agents     | Always (success or failure)                     |
| Webhook processed      | System     | **Only on failure** — successes are silent      |
| Sub-agent finished     | Agents     | Always                                           |
| Sub-agent failed       | Agents     | Always                                           |
| Notification triaged   | Agents     | Only when routed (`escalate`/`react`)           |
| API key rejected       | System     | Always — links to the LLM settings tab          |

The category set the notification center understands is **messages, agents, skills, system, meetings, reminders, important**. The Alerts view shows a filter chip row, but only for categories that actually appear in the current feed, plus **Mark all read** and **Clear**. Clicking a notification marks it read and follows its deep link. Some core notifications carry **action buttons** (e.g. a meeting auto-join prompt) and are pinned to the top of the center.

### Per-provider routing & thresholds

Every provider has its own settings (`notification.settings_set`), letting you tune the noise per source:

| Setting                | Effect                                                                       |
| ---------------------- | --------------------------------------------------------------------------- |
| `enabled`              | When off, that provider's notifications are not ingested at all             |
| `importance_threshold` | Minimum score (0.0–1.0) to display; `0.0` shows everything                  |
| `route_to_orchestrator`| When on, high-importance (`react`/`escalate`) items are forwarded to the agent |

Auto-routing re-reads the provider's settings the moment before escalating, so toggling a setting mid-flight takes effect immediately. A notification is only routed to the agent when its score clears the provider threshold **and** `route_to_orchestrator` is enabled.

***

## Activity hub

The Activity surface (`/activity`) is the transparency layer over everything the agent does without you in the loop. It has three tabs:

| Tab                     | What it shows                                                                                  |
| ----------------------- | --------------------------------------------------------------------------------------------- |
| **Automations**         | Workflows the agent runs on your behalf (the workflows panel)                                  |
| **Background Activity** | The subconscious engine: status bar, active tasks, approval cards, and the evaluation ledger   |
| **Alerts**              | The notification center described above (integration + system streams)                        |

The **Background Activity** tab embeds the subconscious loop's controls and activity log — its tick interval, mode, a manual **Run Now** trigger, and a chronological feed of every background task evaluation with a colored status dot. That loop is documented in full on the [Subconscious Loop](subconscious.md) page; the Activity hub is just its front door.

Older deep links (`?tab=memory`, `?tab=agents`, `?tab=tasks`, …) now live under Settings → Developer & Diagnostics and fall back to the Automations tab.

***

## Routines

Routines (`/routines`) is the user-facing management UI for scheduled automations — the desktop face of the cron system. Jobs are sorted by next-run time, each rendered as a card showing:

* The schedule, rendered human-readable (e.g. "every day at 9am") from its cron expression.
* The job **type** badge — *agent* (runs a prompt through the agent) or *command*.
* The **next run** time (when enabled) and the **last run status** dot — sage for success, coral for failure, neutral when it has not run yet.
* A toggle to **enable/disable** the routine, a **Run Now** button for manual triggering (it polls until the run lands), and an expandable **run history**.

Routines surface and manage the scheduled jobs; the underlying scheduling engine, cron syntax, and the agent tools for creating jobs programmatically are covered on the [Cron / scheduled tasks](native-tools/cron.md) page. Completed and failed runs also emit Agents-category notifications into the center described above.

***

## See also

* [Subconscious Loop](subconscious.md) — the background engine behind the Background Activity tab.
* [Cron / scheduled tasks](native-tools/cron.md) — the scheduling engine and agent tools behind Routines.
* [Triggers](integrations/triggers.md) — webhooks and inbound events that can raise notifications.
