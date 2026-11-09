---
description: >-
  Durable, visual automations. The agent proposes a workflow in chat, you
  review it on a canvas and save it, and it runs on schedules or live app
  events, pausing for your approval when it matters.
icon: diagram-project
---

# Workflows

<figure><img src="../.gitbook/assets/workflows.png" alt=""><figcaption><p>A workflow on the visual canvas. The agent proposes the graph; you review each step and save.</p></figcaption></figure>

Chat is good for one-off asks. Workflows are for things you want done every time: triage every new support email, file every Linear ticket that mentions your team, post a digest every Monday at 9.

A workflow is a saved, typed graph of steps. You can see it on a canvas, and it runs without you. It is inspired by [n8n](https://n8n.io) and [Zapier](https://zapier.com), and it runs on the open-source [tinyflows](https://github.com/tinyhumansai/tinyflows) engine with the same trust and approval machinery as the rest of OpenHuman. The difference from n8n and Zapier is that you do not build the graph. The agent does.

## The agent builds it, you approve it

You do not drag boxes to get started. Describe the automation in chat, for example "whenever a new email arrives from a customer, summarize it and post to my Slack". The agent uses its `propose_workflow` tool to draft a full workflow graph. The proposal appears in chat as a Workflow Proposal Card with a plain-English summary of every step.

Three guarantees keep this safe:

- `propose_workflow` only validates and describes a candidate graph. It can never save or enable a flow.
- Every authoring turn is propose-only by default. Normally you click **Save & enable** on the card, which calls the `flows_create` RPC from the app, not from the agent.
- Nothing the agent can do turns a flow on. `flows_set_enabled` is not one of its tools.

The agent can save in two explicit cases. `save_workflow` writes the built graph onto an existing flow. That is the case when you started the build: the Workflows page prompt bar creates the flow first and opens the copilot on it. When there is no flow yet and you explicitly ask for one ("create this and save it"), the agent may also use `create_workflow`, or `duplicate_flow` to clone a saved flow so you can edit the copy. Both are on the `workflow_builder` agent's tool list (`crates/openhuman-core/src/agent/registry/agents/workflow_builder/agent.toml`), and both force the flow off at creation. `CreateWorkflowTool` calls `flows_set_enabled(.., false)` on anything `flows_create` left enabled. If that second write fails, it reports the flow's real state, not the intended one. So a flow the agent creates starts off and stays off until you enable it. A real test run of a saved flow always needs your confirmation first.

## What a workflow is made of

A workflow graph uses the engine's 22 node kinds: exactly one `trigger`, plus any mix of `agent` (a full agent turn with tools), `tool_call`, `http_request`, `code` (JavaScript or Python), `shell`, `condition`, `switch`, `transform`, `split_out`, `merge`, `output_parser`, `sub_workflow`, `memory`, `dedup`, `loop`, `spawn`, `gate`, `scatter`, `gather`, `approval` and `void`.

The canvas palette offers fifteen of them today: `trigger`, `agent`, `tool_call`, `http_request`, `code`, `condition`, `switch`, `merge`, `split_out`, `transform`, `output_parser`, `sub_workflow`, `memory`, `dedup` and `loop`. You can reach the other seven by importing a graph. Two also have no host adapter wired yet. A `shell` node has no runner, and an `approval` node pauses the run for `flows_resume` instead of raising its own card.

A graph is usually a straight line or a fan-out. It can also contain a bounded loop. A `loop` node emits on its `body` port until its `max_iterations` cap (or an optional `condition`) says stop, then emits on `done`. You close the loop by wiring the body's last node back to the `loop` node. The cap is always finite, and `on_exceeded` decides what reaching it means. `error` fails the run and names the loop. `continue` stops looping and carries the last pass's items out through `done`.

These triggers are live today:

- **Schedule:** cron-backed. The flow fires on its schedule and re-registers itself on every app boot.
- **App event:** a live [trigger](integrations/triggers.md) from a connected integration (a new Gmail thread, a Notion change, a Linear ticket), matched by toolkit and trigger slug.
- **Manual:** a Run button on the Workflows page, or the `flows_run` RPC.
- **Resume:** continuing a run that paused at an approval gate.

A per-flow dispatch lock means a schedule burst can never run the same flow twice at once.

## Trust, approvals and human-in-the-loop

Every flow run executes under a dedicated trust origin (`TrustedAutomation → Workflow`). The flow's actions, meaning which tools it calls and which URLs it hits, are static graph configuration you approved at save time. The runtime trigger payload (a webhook body, an inbound event) stays untrusted. It can feed arguments into those pre-declared actions, but it can never add a new action.

Each flow also has a **Require approval for outbound actions** switch. With it on, every external-effect tool or HTTP call in the run waits at the [approval gate](approval-gate.md) for a real decision. The run's trust root does not auto-allow anything.

When a run pauses, a Flow Approval Card appears in your notifications naming the flow and the pending steps. Approving resumes the run (through `flows_resume`) exactly where it stopped. Runs are durable and checkpointed, so "later today" is fine.

## Watching it run

- `/flows` is the Workflows hub. It shows every flow with its enabled toggle, last-run status (`completed`, `pending approval` or `failed`) and a Run button.
- `/flows/:id` is a read-only canvas view of the workflow graph, drawn as nodes and edges, so you can see exactly what you approved.
- The Run Inspector is a drawer that shows each run step by step (node label, emitted output and final status). It polls every 2 seconds until the run finishes.
- Full run history is saved per flow: status, start and finish times, pending approvals, errors and rebuilt per-step output.

## RPC surface (for developers)

The `flows` domain (`crates/openhuman-core/src/flows/`) exposes 36 controllers under `openhuman.flows_*`. They cover definitions (`create`, `get`, `list`, `update`, `delete`, `set_enabled`, `duplicate`, `validate`, `import`), runs (`run`, `resume`, `cancel`, `list_runs`, `get_run`, history), drafts and the authoring copilot. See the [agent harness](../developing/architecture/agent-harness.md) page for how flow runs share the tinyagents execution stack.

## See also

- [Triggers](integrations/triggers.md): the live app events that fire `app_event` workflows.
- [Approval gate](approval-gate.md): how pending approvals are surfaced and expire.
- [Cron and scheduling](native-tools/cron.md): one-shot and recurring agent jobs. Workflows are the structured, multi-step upgrade.
