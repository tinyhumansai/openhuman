---
description: >-
  Live events from connected apps (a new Gmail message, a Notion edit, a Stripe
  charge) arrive as triggers, get sorted by a triage agent, and can start agent
  actions automatically.
icon: bolt
---

# Triggers

A connected integration is more than a place the agent can read from on demand. It is also a source of live events. When someone emails you, edits a Notion page, opens a GitHub issue on your repo, charges a card on Stripe or DMs you on Slack, OpenHuman gets the event within moments and decides whether to act on it.

This page covers how triggers arrive, how they are sorted, and how one can turn into a full agent action without you typing anything.

## What a trigger is

A trigger is an external event published by an integration you have connected. Some examples:

| Integration | Example trigger                                                   |
| ----------- | ----------------------------------------------------------------- |
| Gmail       | `GMAIL_NEW_GMAIL_MESSAGE`, new mail in your inbox                 |
| Slack       | `SLACK_NEW_MESSAGE`, a channel or DM message that mentions you    |
| Notion      | `NOTION_PAGE_UPDATED`, a tracked page changed                     |
| GitHub      | `GITHUB_ISSUE_OPENED`, `GITHUB_PULL_REQUEST_OPENED` on your repos |
| Stripe      | `STRIPE_CHARGE_SUCCEEDED`, a successful charge on your account    |
| Calendar    | `GOOGLE_CALENDAR_EVENT_CREATED`, a new event on your calendar     |

The full set comes from the [Composio](https://composio.dev) connector layer behind [third-party integrations](README.md). When a connection is active, the matching trigger subscriptions are set up for you.

### Gmail OAuth scopes

Gmail triggers need message-read access on the connected Google account. New Gmail authorizations request `https://www.googleapis.com/auth/gmail.readonly`, so `GMAIL_NEW_GMAIL_MESSAGE` can be enabled and the native Gmail sync can read new message metadata. If you connected Gmail before this scope was requested, reconnect it from Settings before enabling Gmail triggers.

## How a trigger reaches the agent

```text
┌────────────────────┐
│ third-party API    │ Gmail / Slack / Notion / GitHub / ...
└─────────┬──────────┘
          │ webhook
          ▼
┌────────────────────┐
│ OpenHuman backend  │ verifies the webhook signature, cleans up the payload
└─────────┬──────────┘
          │ Socket.IO event ("composio:trigger")
          ▼
┌────────────────────┐
│ Rust core          │ publishes a trigger-received event
│ (your computer)    │ on the in-process event bus
└─────────┬──────────┘
          │
          ▼
┌────────────────────┐
│ Trigger triage     │ drop / acknowledge / react / escalate
└─────────┬──────────┘
          │
          ▼
┌────────────────────┐
│ One of:            │
│ - nothing          │ ← drop
│ - memory note      │ ← acknowledge
│ - Trigger reactor  │ ← react (1-2 tool calls)
│ - Orchestrator     │ ← escalate (full multi-step planning)
└────────────────────┘
```

The raw webhook never reaches your machine. The backend holds the OAuth token and receives the webhook from the third party. It verifies the HMAC signature, cleans up the payload and forwards it to your core over the existing authenticated socket. Your computer only sees a validated event.

## The triage step

Before anything runs, the `trigger_triage` agent looks at every trigger. Its only job is to decide what happens next. It picks exactly one of four actions:

| Action        | What happens                                                                                      | When it is used                                                                                                                                    |
| ------------- | ------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `drop`        | Nothing. The trigger is logged and discarded.                                                     | Spam, duplicates and noise. This is the default for things you don't care about.                                                                   |
| `acknowledge` | A short memory note is saved. No agent runs.                                                      | Passive notices worth remembering, such as "a new page was created in archive".                                                                    |
| `react`       | The `trigger_reactor` agent runs with one or two tool calls.                                      | A small one-step effect: store a memory entry, post a quick acknowledgement, mark a thread read.                                                   |
| `escalate`    | The full orchestrator agent takes over and plans.                                                 | Anything that needs reasoning or several steps: drafting a reply, updating several Notion pages, deciding how to handle an inbound issue.          |

The triage agent has the same memory and workspace context as the rest of the agent. It can tell whether a trigger relates to what you are working on, who is involved, and whether you have asked OpenHuman to act on this kind of thing before.

## When a trigger becomes an agent action

This is what separates "OpenHuman has a Gmail integration" from "OpenHuman is on call for your inbox".

**React** is the cheap path. The trigger reactor is a narrow specialist with a hard budget of a couple of tool calls. It suits writing a one-line memory note ("saw a new Stripe charge for $84, customer X"), marking a Slack message handled because it is the same automated alert you have already triaged twice this week, or storing a structured record of an event you might look up later.

**Escalate** is the heavy path. When triage decides a trigger needs real work, it hands the orchestrator a self-contained task description. The orchestrator has your full set of skills, tools and memory. It might:

- Draft a reply to an important email and queue it for your approval.
- Pull the relevant Notion, Linear or Drive context for an inbound issue and write a structured comment.
- Update three connected systems from one event, such as a customer's plan changing in Stripe, which updates HubSpot, posts in #revenue and adds a note to their Notion file.

Either way, the action runs on your machine, against your configured memory engine, with the same model routing and tools as the rest of the agent.

## Why triage exists

You could pipe every trigger straight into the orchestrator. That would be a mistake for two reasons.

1. Most triggers are noise. A connected Gmail account fires dozens of triggers an hour, and you care about few of them. Running the orchestrator on each would burn budget and create a constant stream of background activity.
2. Triggers deserve different budgets. An automated Stripe receipt and a personal Slack DM should not cost the same number of tokens. Triage keeps the cheap path cheap and saves the orchestrator for events that earn it.

Triage runs on the fast model tier (see [Model routing](../model-routing/README.md)), so classification takes well under a second.

## Configuration and opt-out

- **On by default.** Once an integration is connected, its triggers feed the pipeline automatically.
- **Opt out.** Set the `OPENHUMAN_TRIGGER_TRIAGE_DISABLED` environment variable to `1`, `true` or `yes`. Agent classification turns off and triggers are only logged. The integration stays connected. Only the automatic actions stop.
- **Per-trigger settings.** Choose which integrations and event types are evaluated under **Settings**. The RPC methods are `get_composio_trigger_settings` and `update_composio_trigger_settings`.
- **Audit log.** Every trigger is written to the trigger history, whatever the decision, so you can see what arrived, what triage decided and what ran. Decisions and escalations are also published as `TriggerEvaluated` and `TriggerEscalated` events on the in-process bus, so anything inside the core can subscribe.

## Privacy boundary

Triggers follow the same boundary as the rest of the product (see [Privacy and security](../privacy-and-security.md)):

- The third-party token lives on the backend and never on your computer.
- The backend verifies the webhook signature before anything reaches your machine.
- Your local core processes the payload. Classification and any reaction run on your machine, against your configured memory engine.
- Notes written by the acknowledge, react and escalate paths are stored as learnings in your memory engine when memory is on.

## See also

- [Third-party integrations](README.md): the catalog of services triggers come from.
- [Memory sources](../memory.md): the polling counterpart, which pulls source data into memory on a schedule.
