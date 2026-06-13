---
description: >-
  Live events from connected integrations (a new Gmail message, a Notion edit,
  a Stripe charge) arrive as triggers, get classified by a triage agent, and
  can fire agent actions automatically.
icon: bolt
---

# Triggers

A connected integration is not just a place the agent can read from on demand. It is also a **source of live events**. When someone sends you an email, edits a Notion page, opens a GitHub issue on one of your repos, charges a card on Stripe, or DMs you on Slack, OpenHuman receives that event in near-real-time and can decide whether to do something about it.

This page is about that pipeline: how triggers arrive, how they get classified, and how a trigger can turn into a full agent action without you typing a thing.

## What a trigger is

A trigger is an external event published by an integration you've connected. Common shapes:

| Integration | Example trigger |
| --- | --- |
| **Gmail** | `GMAIL_NEW_GMAIL_MESSAGE`, new mail in inbox |
| **Slack** | `SLACK_NEW_MESSAGE`, channel/DM message you were mentioned in |
| **Notion** | `NOTION_PAGE_UPDATED`, a tracked page changed |
| **GitHub** | `GITHUB_ISSUE_OPENED`, `GITHUB_PULL_REQUEST_OPENED` on your repos |
| **Stripe** | `STRIPE_CHARGE_SUCCEEDED`, a successful charge on your account |
| **Calendar** | `GOOGLE_CALENDAR_EVENT_CREATED`, a new event on your calendar |

The full set comes from the [Composio](https://composio.dev) connector layer that powers [third-party integrations](README.md). When a connection is active, the relevant trigger subscriptions are wired up automatically.

### Gmail OAuth scopes

Gmail trigger subscriptions require message-read access on the connected Google account. Fresh OpenHuman Gmail authorizations request `https://www.googleapis.com/auth/gmail.readonly` so `GMAIL_NEW_GMAIL_MESSAGE` can be enabled and the native Gmail sync path can read the new message metadata.

If an older Gmail connection was created before this scope was requested, reconnect Gmail from Settings before enabling Gmail triggers.

## Where triggers come from, end to end

```
┌────────────────────┐
│ third-party API │ Gmail / Slack / Notion / GitHub / ...
└─────────┬──────────┘
 │ webhook
 ▼
┌────────────────────┐
│ OpenHuman backend │ HMAC-verifies the webhook, normalises the payload
└─────────┬──────────┘
 │ Socket.IO event ("composio:trigger")
 ▼
┌────────────────────┐
│ Rust core │ publishes DomainEvent::ComposioTriggerReceived
│ (your laptop) │ on the in-process event bus
└─────────┬──────────┘
 │
 ▼
┌────────────────────┐
│ Trigger Triage │ classifies: drop / acknowledge / react / escalate
└─────────┬──────────┘
 │
 ▼
┌────────────────────┐
│ One of: │
│ - nothing │ ← drop
│ - memory note │ ← acknowledge
│ - Trigger Reactor │ ← react (1-2 tool calls)
│ - Orchestrator │ ← escalate (full multi-step planning)
└────────────────────┘
```

The webhook never reaches your machine raw. The backend is what holds the OAuth token and what receives the webhook directly from the third-party. It does HMAC verification, normalises the payload, and forwards it to your Rust core over the existing authenticated socket. Your laptop sees a clean, validated `ComposioTriggerReceived` event on the bus, nothing else.

## The triage step

Before any action runs, every trigger goes through the [`trigger_triage`](https://github.com/tinyhumansai/openhuman/tree/main/src/openhuman/agent/agents/trigger_triage) agent. Its only job is to decide what the rest of the system should do.

It picks exactly one of four actions:

| Action | What happens | When to use |
| --- | --- | --- |
| **`drop`** | Nothing. Trigger is silently logged and discarded. | Spam, duplicates, irrelevant noise. The default for things you don't care about. |
| **`acknowledge`** | A short memory note is persisted, no agent runs. | Passive notifications worth remembering ("a new page was created in archive"). |
| **`react`** | The [`trigger_reactor`](https://github.com/tinyhumansai/openhuman/tree/main/src/openhuman/agent/agents/trigger_reactor) agent runs with one or two tool calls. | A small, single-step side effect: store a memory entry, post a quick acknowledgement, mark a thread read. |
| **`escalate`** | The full **orchestrator** agent takes over with planning capability. | Anything that needs reasoning, multiple steps, or multiple skills: drafting a reply, updating several Notion pages, deciding how to triage an inbound issue. |

The triage agent has the same memory and workspace context the rest of the agent has. It can tell whether a trigger is relevant to something you're currently working on, who the people involved are, and whether it's the kind of thing you've asked OpenHuman to act on before.

## When a trigger turns into an agent action

This is the part that distinguishes "OpenHuman has a Gmail integration" from "OpenHuman is on call for your inbox":

- **`react`** is the cheap path. The Trigger Reactor is a narrow specialist with a hard budget of a couple of tool calls. It's perfect for: writing a one-line memory note that says "saw a new charge from Stripe for $84, customer X, merchant Y", silently marking a Slack message as handled because it's the same automated alert you've already triaged twice this week, or storing a structured record of an event the user might want to look up later.

- **`escalate`** is the heavy path. When the Triage agent decides the trigger needs real work, it hands off to the Orchestrator with a self-contained task description. The orchestrator has access to your full skill surface, tools, memory, and the [Subconscious Loop](../subconscious.md) outputs. From there it might:
  - Draft a reply to an important email and queue it for your approval.
  - Pull up the relevant Notion / Linear / Drive context for an inbound issue and write a structured comment.
  - Update three connected systems based on a single inbound event ("this customer's plan changed in Stripe, update HubSpot, post in #revenue, and add a note to their Notion file").
  - Decide the trigger means a meeting just got scheduled and pre-load the [Meeting Agent](../mascot/meeting-agents.md) for that call.

In both cases the action runs on your machine, against your local Memory Tree, with the same model-routing and tool surface the rest of the agent uses.

## Why a triage step at all

It's tempting to skip the classifier and just pipe every trigger straight into the orchestrator. That's a bad idea for two reasons:

1. **Most triggers are noise.** A connected Gmail account fires dozens of triggers an hour, the vast majority of which the user doesn't care about. Running the orchestrator on each would burn budget and produce a constant stream of background activity.
2. **Different triggers deserve different ceilings.** An automated Stripe receipt and a personal Slack DM should not cost the same number of tokens to handle. Triage lets the cheap path be cheap and reserves the orchestrator for things that earn it.

Triage runs on the fast model tier (see [Automatic Model Routing](../model-routing/README.md)) so the classification itself is sub-second.

## Configuration and opt-out

- **On by default.** Once an integration is connected, its triggers feed into the pipeline automatically.
- **Opt-out.** The triage path is gated on the `OPENHUMAN_TRIGGER_TRIAGE_DISABLED` environment variable. Setting it to `1` / `true` / `yes` turns off agent classification and falls back to passive logging only. The integration itself stays connected; only the auto-action behaviour is suppressed.
- **Per-trigger settings.** Trigger settings (which integrations and event types should be evaluated) are managed under **Settings**; the underlying RPC methods are `update_composio_trigger_settings` / `get_composio_trigger_settings`.
- **Audit log.** Every trigger, regardless of decision, is written to the trigger history so you can see what arrived, what the classifier decided, and what (if anything) ran. Decisions and escalations are also published as `TriggerEvaluated` / `TriggerEscalated` events on the in-process bus, which means anything inside the core can subscribe to them.

## Privacy boundary

Triggers follow the same boundary as the rest of the product (see [Privacy & Security](../privacy-and-security.md)):

- The third-party token lives on the backend, never on your laptop.
- The webhook is HMAC-verified by the backend before it reaches your machine.
- The trigger payload is processed by your local core; classification and any reaction run on your machine, against your local Memory Tree.
- Memory notes written by `acknowledge` / `react` / `escalate` paths are stored in your local SQLite memory tree and Markdown vault, the same as any other source.

## Implementation pointers (for developers)

- Triage agent: `src/openhuman/agent/agents/trigger_triage/`
- Reactor agent: `src/openhuman/agent/agents/trigger_reactor/`
- Composio bus subscriber: `src/openhuman/composio/bus.rs` (`ComposioTriggerSubscriber`)
- Trigger history persistence: `src/openhuman/composio/trigger_history.rs`
- Domain events: `DomainEvent::ComposioTriggerReceived`, `DomainEvent::TriggerEscalated` in `src/core/event_bus/events.rs`
- Trigger settings RPC: `update_composio_trigger_settings` / `get_composio_trigger_settings` in `src/openhuman/config/`

## See also

* [Third-party Integrations](README.md), the catalog of services triggers come from.
* [Auto-fetch from Integrations](../obsidian-wiki/auto-fetch.md), the polling counterpart, periodic ingest of source data into the Memory Tree.
* [Subconscious Loop](../subconscious.md), the background loop that uses trigger context and memory to plan ahead.
* [Meeting Agents](../mascot/meeting-agents.md), one place an escalated trigger can land (a calendar event with a Meet link).
