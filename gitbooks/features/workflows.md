---
description: >-
  Durable, visual automations built on the open-source tinyflows engine. The
  agent proposes a workflow in chat; you review it on a canvas, save it, and it
  runs on schedules or live app events, pausing for your approval when it
  matters.
icon: diagram-project
---

# Workflows

<figure><img src="../.gitbook/assets/workflows.png" alt=""><figcaption><p>A workflow on the visual canvas. The agent proposes the graph; you review each step and save.</p></figcaption></figure>

Chat is great for one-off asks. **Workflows** are for the things you want done *every time*: triage every new support email, file every Linear ticket that mentions your team, post a digest every Monday at 9. Heavily inspired by [n8n](https://n8n.io) and [Zapier](https://zapier.com), a workflow is a saved, typed graph of steps you can see on a canvas and that runs without you. It is backed by the open-source [tinyflows](https://github.com/tinyhumansai/tinyflows) engine and the same trust and approval machinery as the rest of OpenHuman. The difference from n8n and Zapier: you don't build the graph, the agent does.

## The agent builds it, you approve it

You don't drag boxes to get started. Describe the automation in chat, for example *"whenever a new email arrives from a customer, summarize it and post to my Slack"*, and the agent uses its `propose_workflow` tool to draft a complete workflow graph. The proposal shows up in chat as a **Workflow Proposal Card** with a plain-English summary of every step.

Two design guarantees make this safe:

* The `propose_workflow` tool **only validates and describes** a candidate graph. It can never create or enable a flow by itself.
* The **only** path from a proposal to a *new* saved workflow is you clicking **Save & enable** on the card. That calls the `flows_create` RPC directly from the app, not from the agent.

One deliberate carve-out: when *you* start the build (the Workflows page prompt bar creates the flow first and opens the copilot on it), the builder agent may finish the job with its `save_workflow` tool — it writes the built graph onto that **already-existing** flow after a sandbox dry run. It still cannot create a flow of its own, enable or disable one, or change the approval gate, and a real test run always requires your explicit confirmation first.

## What a workflow is made of

A workflow graph is composed of **12 node kinds**: exactly one `trigger`, plus any mix of `agent` (a full agent turn with tools), `tool_call`, `http_request`, `code` (JavaScript or Python), `condition`, `switch`, `transform`, `split_out`, `merge`, `output_parser`, and `sub_workflow`.

Triggers come in several kinds. The ones live today:

* **Schedule**: cron-backed; the flow fires on its schedule and re-registers itself on every app boot.
* **App event**: a live [trigger](integrations/triggers.md) from a connected integration (a new Gmail thread, a Notion change, a Linear ticket) matched by toolkit + trigger slug.
* **Manual**: a Run button on the Workflows page or the `flows_run` RPC.
* **Resume**: continuing a run that paused at an approval gate.

A per-flow dispatch lock means a schedule burst can never run the same flow twice concurrently.

## Trust, approvals, and human-in-the-loop

Every flow run executes under a dedicated trust origin (`TrustedAutomation → Workflow`). The reasoning: the flow's *actions* (which tools it calls, which URLs it hits) are static graph configuration you approved at save time. The runtime trigger payload (a webhook body, an inbound event) stays **untrusted**: it can feed arguments into those pre-declared actions, but it can never introduce a new action.

On top of that, each flow has a **"Require approval for outbound actions"** switch. With it on, every external-effect tool or HTTP call in the run parks at the [approval gate](approval-gate.md) and waits for a real decision. The run's trust root does not auto-allow anything.

When a run pauses, you get a **Flow Approval Card** in your notifications naming the flow and the pending steps. Approve resumes the run (via `flows_resume`) exactly where it stopped. Runs are durable and checkpointed, so "later today" is fine.

## Watching it run

* **`/flows`**: the Workflows hub, showing every flow with its enabled toggle, last-run status (`completed` / `pending approval` / `failed`), and a Run button.
* **`/flows/:id`**: a read-only **canvas view** of the workflow graph, rendered as nodes and edges so you can see exactly what you approved.
* **Run Inspector**: a drawer showing each run step by step (node label, emitted output, and final status), live-polling every 2 seconds until the run finishes.
* Full **run history** is persisted per flow: status, start/finish times, pending approvals, errors, and reconstructed per-step output.

## RPC surface (for developers)

The `flows` domain (`src/openhuman/flows/`) exposes ten controllers under `openhuman.flows_*`: `create`, `get`, `list`, `update`, `delete`, `set_enabled`, `run`, `resume`, `list_runs`, `get_run`. See the [Agent Harness](../developing/architecture/agent-harness.md) page for how flow runs share the tinyagents execution stack.

## See also

* [Triggers](integrations/triggers.md): the live app events that fire `app_event` workflows.
* [Approval Gate](approval-gate.md): how pending approvals are surfaced and expire.
* [Cron & Scheduling](native-tools/cron.md): one-shot and recurring agent jobs (workflows are the structured, multi-step upgrade).
* [Subconscious Loop](subconscious.md): the background awareness layer that complements event-driven workflows.
