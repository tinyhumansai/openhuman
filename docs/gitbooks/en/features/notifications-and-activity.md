---
description: >-
  The notification center, the notice tray, and where background work shows
  up: what OpenHuman tells you about, and what it does while you are away.
icon: bell
---

# Notifications and activity

OpenHuman shows two kinds of "what is happening". Notifications are things you should look at, like an important Slack message, a failed webhook or a high-priority email. Activity is a record of what the agent did on its own while you were not watching.

Both live on the Notifications page. There is no Activity hub or Routines screen. `/activity` redirects to **Settings → Account**, and `/routines` redirects to [Workflows](workflows.md). The scheduler lives under Workflows → Schedules.

## Notification center

Two independent streams feed the notification center. They show as two stacked sections on the Notifications page.

### Integration notifications

Notifications from connected accounts (Gmail, Slack, WhatsApp, Discord and others) come in through the `notification.ingest` RPC. They are saved to a per-workspace SQLite store, then triaged by a local LLM in the background. Ingest returns immediately. Triage runs in a spawned task and fills in the score a moment later, so a new item can briefly show as unscored.

Triage gives each notification an action, which maps to a fixed importance score from 0.0 to 1.0:

| Triage action | Score | What it means |
| --- | --- | --- |
| `drop` | 0.10 | Noise, not worth surfacing |
| `acknowledge` | 0.35 | Low value, informational |
| `react` | 0.65 | Worth a follow-up |
| `escalate` | 0.90 | High priority, hand to the agent |

Only `react` and `escalate` are routed. `drop` and `acknowledge` stay quiet. Each item carries a one-sentence `triage_reason` explaining the call, and a status that moves from unread to read to acted to dismissed. Duplicate content received within 60 seconds collapses into one entry.

### System notifications

The second stream turns selected internal events into short alerts and pushes them over the socket bridge as they happen. They are saved before they are sent, so anything fired while the app was closed syncs on the next open. Each has a category and an in-app deep link:

| Source event | Category | Shows when |
| --- | --- | --- |
| Cron job completed | Agents | Always (success or failure) |
| Webhook processed | System | Only on failure. Successes are silent. |
| Sub-agent finished | Agents | Always |
| Sub-agent failed | Agents | Always |
| Notification triaged | Agents | Only when routed (`escalate` or `react`) |
| API key rejected | System | Always. Links to the LLM settings tab. |

The categories are messages, agents, skills, system, meetings, reminders and important. The page shows filter chips only for categories present in the current feed, plus **Mark all read** and **Clear**. Clicking a notification marks it read and follows its deep link. Some system notifications have action buttons and are pinned to the top. The feed holds the most recent 200 items.

### Per-provider routing and thresholds

Each provider has its own settings (`notification.settings_set`), so you can tune the noise per source:

| Setting | Effect |
| --- | --- |
| `enabled` | When off, that provider's notifications are not ingested at all. |
| `importance_threshold` | Minimum score (0.0 to 1.0) to display. `0.0` shows everything. |
| `route_to_orchestrator` | When on, high-importance (`react` or `escalate`) items are forwarded to the agent. |

Auto-routing re-reads the provider's settings just before it escalates, so a change takes effect immediately. A notification goes to the agent only when its score clears the provider threshold and `route_to_orchestrator` is on.

These per-provider settings have no UI today. Set them through the RPC or by hand. They gate ingest, not just display, so keep that in mind before changing one.

## The notice tray

A quiet tray in the bottom-right corner collects things you can act on: a rejected provider key, a reached plan limit, a keyring consent prompt. Repeats of the same problem bump a count instead of stacking. The tray lives in memory, so it clears on restart.

Product announcements arrive separately, as a modal shown once per signed-in session. OpenHuman remembers the ids you have already seen.

## Where background work shows up

| Kind of background work | Where to look |
| --- | --- |
| Scheduled jobs and their run history | **Workflows → Schedules**. See [Cron and scheduling](native-tools/cron.md). |
| Workflow runs | **Workflows → Runs** |
| Memory belief builds and source syncs | **Connections → Memory → Background** |
| Detached sub-agents and async delegation | The background inbox card in the thread that started them. See [Chat](chat.md). |
| All of the above, as notifications | The Notifications page, Agents category |

## See also

- [Cron and scheduling](native-tools/cron.md): the scheduling engine and the agent tools behind the Schedules view.
- [Triggers](integrations/triggers.md): webhooks and inbound events that can raise a notification.
- [Chat](chat.md): where an approval or a sub-agent result lands when you are in the conversation.
