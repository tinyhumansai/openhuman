---
description: >-
  The agent asks before it does anything risky. When the autonomy policy is
  on, risky actions wait for your approval and are denied if you don't answer.
icon: shield-check
---

# Approval gate

The approval gate sits between the agent and the outside world. When the agent wants to do something your autonomy tier says needs a check, the gate stops the call, shows you what is about to happen, and waits for your decision. Actions your tier already allows run without a prompt. On the `full` tier, for example, a write is never prompted.

{% hint style="warning" %}
The autonomy policy is off by default, and the gate is part of it. With `[autonomy] enabled = false`, command classification, the tier table, the allowlist, the hourly action budget, `workspace_only` and `forbidden_paths` do nothing, and acting tool calls run without a prompt. The exceptions are the forced approvals below.

This is deliberate. Agents are expected to run inside a container, a platform jail or a Docker sandbox that already provides isolation, and a shell that refuses ordinary shell syntax is not a usable shell. To turn the policy on, set `[autonomy] enabled = true` in `config.toml`.
{% endhint %}

## What still holds with the policy off

Three protections do not depend on `[autonomy] enabled`:

- Credential stores and system roots stay unreachable. `~/.ssh`, `~/.gnupg`, `~/.aws` and system roots are always refused, as are `..` traversal and null bytes in a path.
- Hard blocks inside the tools themselves, including the tool-policy middleware, always run.
- Path checks that gate file access are answered by the TinySecurity module and fail closed if it is unavailable. The approval prompts, allow-list and optional Jev auto-approve judge described here still run in the core.
- Sandboxing is separate. The platform jail and the Docker backend are chosen by `[runtime]` and the session's origin, not by the autonomy tier. See [Privacy and security](privacy-and-security.md).

Everything below describes the policy with `[autonomy] enabled = true`.

## What triggers a prompt

The agent classifies every acting tool call into a command class. Your autonomy tier then decides whether that class runs, prompts or is blocked.

| Command class | What it covers                                                    |
| ------------- | ----------------------------------------------------------------- |
| Read          | Provably read-only or observational (a curated allowlist)         |
| Write         | Changes state. This is the default for anything unrecognized      |
| Network       | Reaches the network (curl, wget, ssh, scp, and so on)             |
| Install       | Installs an OS or global language package                         |
| Destructive   | Catastrophic, irreversible or privilege-escalating                |

You pick the tier in **Settings > Permissions** (`[autonomy].level`):

| Tier                                           | Read  | Write  | Network, Install, Destructive |
| ---------------------------------------------- | ----- | ------ | ----------------------------- |
| `readonly`                                     | Allow | Block  | Block                         |
| `supervised` (default when the policy is on)   | Allow | Prompt | Prompt                        |
| `full`                                         | Allow | Allow  | Prompt                        |

A call that lands on Prompt waits at the gate. A call that lands on Block is refused, and no approval can override it.

Classification fails closed. A command that is not provably read-only counts as at least a write. In a piped command the highest class wins, so `ls | curl ...` is Network. A quoted heredoc body (`<< 'EOF' ... EOF`) is treated as data and not scanned. An unquoted one (`<< EOF`) is expanded and scanned.

## How a request flows

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

A parked call shows an approval card above the chat composer. The card has the tool name, a one-line summary of the action and the redacted command. You have three choices:

- **Approve** runs this one call.
- **Always allow** runs it and adds the tool to your `auto_approve` list, so it skips the prompt next time.
- **Deny** refuses the call.

You can also type "yes" or "no" in chat, and the reply goes to the parked request.

## Always allow

Choosing **Always allow** saves the tool name to `[autonomy].auto_approve` and reloads the policy. The gate then lets that tool through on later turns. To be prompted again, remove the entry in **Settings > Agent access**.

The default list is `file_read`, `memory_search`, `memory_list`, `get_time`, `list_dir`, `glob` and `grep`. Four of those names no longer match a registered tool. Memory is now one tool called `memory`, the clock is `current_time`, and directory listing is `list`. So only `file_read`, `glob` and `grep` have any effect.

## auto_approve_all

`[autonomy].auto_approve_all` approves every call without asking. Before you enable it, know three things:

- A call from an unlabelled call site is still denied, and the hard blocks inside the tools still apply.
- It skips parking, not just the prompt. A triage dispatch from a connector or webhook payload normally parks and writes an audit row. With this flag it runs at once and writes no audit row, so those dispatches leave no approval trail.
- It does not override a forced approval.

## Forced approvals

Browser page interactions cannot be pre-authorized. Every `click`, `double_click`, `fill`, `type`, `press`, `select` and `check` goes through a forced approval, which works differently:

- Every shortcut is skipped. `auto_approve_all`, the `auto_approve` list and per-flow tool trust are ignored. The call parks even with `[autonomy] enabled = false`.
- The decision is one-time. **Always allow** is refused, so you can only approve once or deny.
- It needs a live chat. A turn that is not a web-chat turn is denied instead of parked, so a cron job, channel message or flow cannot drive your browser.

The card names the action, the page origin and a SHA-256 digest that ties the action to that URL. If the page navigates while you decide, the approved action is refused and not replayed on the new page.

## Fail-closed behavior

Every path that is not an approval ends in a denial:

- A parked request lives for 10 minutes. If you have not decided by then, it is denied.
- If saving the request fails or the channel drops, the call is denied.
- On timeout the gate re-reads the stored decision first, so an approval that landed at the last moment still wins.

Pending requests are stored in SQLite (`{workspace_dir}/approval/approval.db`) and survive a core restart. After an approved tool finishes, the gate records the outcome (success or error) as an audit trail. Error text is sanitized and capped. Everything stored or broadcast is redacted first: personal data and chat content are scrubbed and home paths are stripped.

## What the turn's origin decides

The gate is interactive, so a parked call needs a surface that can answer it. The turn's origin decides what gets parked and what passes straight through:

| Turn origin | At the gate |
| --- | --- |
| Cron and internal background jobs | Allowed, no row, no event |
| A saved flow's pre-declared action | Allowed, no row, no event |
| CLI, a delegated sub-agent, and a triage dispatch started on your own machine | Allowed, no row, no event |
| Web chat | Parks for your decision |
| A triage dispatch from a connector or webhook payload | Parks and writes a pending-approval row |
| An external channel turn (Telegram, Slack and so on) | Parks and writes a row |
| An unlabelled call site | Denied |

A triage dispatch that your own machine started keeps the authority its caller already had. One steered by an outside payload parks and leaves an audit row. No surface can decide a background park yet, so it expires at the timeout. You get the audit trail, not a working escalation, and `auto_approve_all` gives up even that.

The browser's consequential actions (click, fill, type, key press, select, check, and a task's `needs_approval` step) are the exception. They use the **forced** gate, which ignores auto-approval and denies every turn that is not a routable WebChat chat, cron included. An operator opts specific action kinds back in for cron, background and approval-free workflow turns with `[browser] unattended_actions`. See [TinyComputer browser on Linux and Docker](../developing/tinycomputer-docker.md#4-unattended-actions).

An external channel turn parks because remote input is untrusted. The row is written, and you can still decide on the thread card before the timeout.

## Configuration and RPC

- `[autonomy].enabled` is the master switch for the policy. It is `false` by default. Forced approvals and the always-forbidden floor work without it.
- `OPENHUMAN_APPROVAL_GATE` set to `0` or `false` skips installing the gate even with the policy on. With no gate, Prompt-class calls run unprompted.
- `[autonomy].level` and `[autonomy].auto_approve` set the tier and the allowlist. Change them with the `config.update_autonomy_settings` RPC or in the settings panels.

The `approval` controller exposes three JSON-RPC methods:

| Method                                     | Purpose                                                                                                      |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------ |
| `openhuman.approval_list_pending`          | The live queue of parked requests.                                                                           |
| `openhuman.approval_list_recent_decisions` | Decided and executed audit rows (`limit` 1 to 500, default 50), shown in **Settings > Approval history**.    |
| `openhuman.approval_decide`                | Applies a decision (`approve_once`, `approve_always_for_tool` or `deny`).                                    |

The two list methods return an empty list, not an error, when no gate is installed. `decide` returns an error when the gate is absent or the request is unknown or already decided.

## See also

- [Privacy and security](privacy-and-security.md): what leaves the machine, and the layers underneath this one.
- [Security architecture](../developing/architecture/security.md): command classification and policy internals.
- [Agent access settings](settings.md): where the tier, the trusted roots and the allowlist are edited.
