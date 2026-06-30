---
description: >-
  Human-in-the-loop consent for side-effecting tool calls — the agent parks any
  risky action until you approve it, and fails closed if you don't.
icon: shield-check
---

# Approval Gate

The Approval Gate is the checkpoint between the agent and the outside world. Whenever the agent wants to run a tool that has a real-world effect — post to Slack, send an email, create a calendar event, run a shell command, install a package — the gate intercepts the call, shows you exactly what's about to happen, and waits for your decision before anything runs.

It's on by default. Nothing with an external effect leaves your machine in an interactive chat without you saying yes.

***

## What triggers a prompt

Every acting tool call is classified into a **command class**, and your **autonomy tier** decides whether that class runs silently, prompts, or is blocked.

| Command class | What it covers                                              |
| ------------- | ---------------------------------------------------------- |
| Read          | Provably read-only / observational (curated allowlist)     |
| Write         | State-changing; the fail-closed default for anything unrecognized |
| Network       | Reaches the network (curl, wget, ssh, scp, …)              |
| Install       | Installs an OS or global language package                  |
| Destructive   | Catastrophic / irreversible / privilege-escalating         |

The tier comes from **Settings → Agent access** (`[autonomy].level`):

| Tier           | Read  | Write  | Network / Install / Destructive |
| -------------- | ----- | ------ | ------------------------------- |
| Read-only      | Allow | Block  | Block                           |
| Supervised *(default)* | Allow | Prompt | Prompt                          |
| Full           | Allow | Allow  | Prompt                          |

Anything that lands on **Prompt** is parked at the gate. `Block` is refused outright — no in-tier approval can authorize it. Classification is fail-closed: a command that isn't provably read-only is treated as at least `Write`, and across a piped command the highest class wins (so `ls | curl …` is `Network`).

***

## The flow

```text
agent wants to act
        │
        ▼
 classify command ──► Block ──► refused
        │
     Prompt
        │
        ▼
 on "Always allow" list? ──► yes ──► run immediately
        │ no
        ▼
 park call · persist pending row · emit approval_request
        │
        ▼
 ┌──────────────┬───────────────┬────────────┐
 ▼              ▼               ▼            ▼
Approve     Always allow      Deny      10-min TTL
(once)    (+ allowlist)                     │
 │             │               │            ▼
 ▼             ▼               ▼          Deny
 run           run           refused   (fail closed)
```

When a call is parked, an **Approval Request card** appears above the chat composer. It shows the tool name, a safe one-line summary of the action, and the (redacted) command. Three choices:

* **Approve** — run this one call.
* **Always allow** — run it, and add the tool to your `auto_approve` list so it skips the prompt next time.
* **Deny** — refuse this call.

You can also just type **yes** / **no** in chat — the reply is routed back to the parked request.

***

## Always allow

Approving with **Always allow** persists the tool name onto `[autonomy].auto_approve` (config save + live-policy reload), so the gate short-circuits to *allow* for that tool on future turns. The list ships with safe read-only tools pre-approved (`file_read`, `memory_search`, `memory_list`, `get_time`, `list_dir`, `glob`, `grep`) and is editable in **Settings → Agent access**. Remove an entry there to start being prompted again.

***

## Fail-closed behavior

Every non-approve path resolves to **Deny**:

* **Timeout** — a parked request lives for 10 minutes; if undecided it is transitioned to a terminal `deny`.
* **Persist failure** or a dropped channel — denied.
* The timeout path re-reads the stored decision first, so an approval that committed in the race still wins.

Pending requests are stored in SQLite (`{workspace_dir}/approval/approval.db`) and **survive a core restart**. After an approved tool finishes, the gate records a write-once execution outcome (success / error, error text sanitized and capped) as a durable audit trail. Everything persisted or broadcast is redacted first — PII and chat content are scrubbed and home paths stripped.

***

## Background and cron bypass

The gate is **interactive-only**. Background, triage, and cron turns carry no chat context, so there's nobody to answer a prompt — these turns are pre-authorized and pass straight through (no row, no event). Approval is only enforced for live chat turns. (The Subconscious loop has its own, separate escalation-card approval for *unsolicited* writes — see below.)

***

## Configuration & RPC

* **`OPENHUMAN_APPROVAL_GATE`** — set to `0` / `false` to skip installing the gate entirely. With no gate, `Prompt`-class calls run unprompted. On by default.
* **`[autonomy].level`** and **`[autonomy].auto_approve`** — tier and allowlist, via the `config.update_autonomy_settings` RPC or Settings → Agent access.

The `approval` controller exposes three JSON-RPC methods:

| Method                              | Purpose                                                              |
| ----------------------------------- | ------------------------------------------------------------------- |
| `openhuman.approval_list_pending`   | The live queue of parked requests.                                  |
| `openhuman.approval_list_recent_decisions` | Decided/executed audit rows (`limit` 1–500, default 50). Surfaced in **Settings → Approval history**. |
| `openhuman.approval_decide`         | Apply a decision (`approve_once` / `approve_always_for_tool` / `deny`). |

`list_pending` / `list_recent_decisions` return empty (not an error) when no gate is installed; `decide` errors when the gate is absent or the request is unknown or already decided.

***

## See also

* [Privacy & Security](privacy-and-security.md) — autonomy tiers, trusted roots, and path hardening.
* [Subconscious Loop](subconscious.md) — the background loop and its separate escalation approvals.
* [Security architecture](../developing/architecture/security.md) — the command-classification and policy internals.
