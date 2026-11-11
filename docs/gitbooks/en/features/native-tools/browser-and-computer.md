---
description: >-
  Read and drive a real browser and a real desktop: accessibility snapshots,
  element references, bounded tasks that stop at a payment page, and a small
  decision model that picks each step.
icon: display
---

# Browser and computer control

When the agent needs to use a computer the way a person would, by opening a page, reading the screen, clicking a button or typing a phrase, these tools do it. Desktop and browser control both come from one loadable module, TinyComputer. It downloads on first use.

## Two surfaces, one model

Desktop and browser produce the same kind of "screen": a compact list of things a person could interact with. Each item has a role, a name, a value, its states, the actions it supports, and the labels of the boxes it sits in. The list is capped at 254 controls. Both surfaces offer the same short list of actions (click, type, check, uncheck, expand, collapse, scroll, wait, press), so the agent learns one vocabulary.

Elements are addressed by reference, never by coordinates or CSS selectors. A desktop reference like `@s8f3k2p9:e1` is tied to the snapshot that produced it. Acting on it either reaches the element that was described or fails with a stale-reference error telling the agent to look again. It never clicks whatever has moved into that spot since. Browser references survive a redraw, because they are marks that last as long as the element is on the page.

## Browser

The browser is a real Chrome or Chromium, driven over the Chrome DevTools Protocol by a library built into the module. Chrome runs as its own process, but nothing sits between it and the module: no driver binary, no local HTTP bridge and no second OpenHuman process.

- **Open** a session. You can launch a fresh browser (headless or headed) or attach to the Chrome you already have running. Attaching matters because many sites turn away a fresh automated browser but serve a person's own. When a task ends, an attached browser is only disconnected, never closed.
- **Snapshot** the page. The default mode, `sight`, runs a small in-page script that reads the page the way a person sees it: real links, buttons and fields, plus anything with a pointer cursor, a click handler or a tab stop. It drops hidden, zero-size, transparent and disabled items. It marks items below the fold as offscreen and items behind a dialog as covered. It falls back to the accessibility tree when it can't read a page, and `perception: "tree"` forces the tree.
- **Act**: click (optionally into a new tab), fill, type, press, select, check, hover, scroll, wait, read and go back.
- **Read** the page as text, Markdown or serialized DOM, and **evaluate** JavaScript.
- **Downloads**: list the browser's download events and wait for a completed file. A page snapshot alone doesn't prove a file finished.

Screenshots are never returned inline. A reply carries a handle that the agent reads in chunks and releases when done.

Clicks, typing, key presses and other consequential actions ask for your approval in the chat first. A scheduled job has nobody to ask, so by default it can only open and read pages. An operator can let cron, background and approval-free workflow turns take specific actions with `[browser] unattended_actions`. See [TinyComputer browser on Linux and Docker](../../developing/tinycomputer-docker.md#4-unattended-actions).

Sessions are bounded. The module allows up to 8 at once. OpenHuman keeps at most 6 per conversation thread with a 30-minute idle timeout, so a long chat reuses one browser and doesn't open a new one every turn. A session can be restricted to a set of origins.

## Desktop

Desktop control reads the operating system's accessibility tree, the same interface a screen reader uses. It covers running apps, windows, displays, Notification Center entries, the clipboard and screenshots.

Desktop control works on macOS only for now. Windows desktop control isn't supported yet; use the browser on Windows. Linux isn't supported (see the table below).

Actions go through the accessibility API and not through synthesized input. By default they don't steal focus, move your cursor or touch the clipboard, so a run can continue while you use the machine. A headed mode and real mouse and key events exist for apps that need them, but they are the exception.

For dense apps, a skeleton snapshot stops at three levels and returns structure without leaf detail. That is the difference between tens of thousands of tokens and a few hundred.

## Tasks

Above these basics sits a bounded task loop. You describe the outcome, the module plans the steps, and a small decision model ([Jev](../../developing/jev.md)) picks every click. A task starts and returns at once with a status, which the agent polls and answers:

| Status                          | What it means                                                     |
| ------------------------------- | ----------------------------------------------------------------- |
| `running`                       | Working.                                                          |
| `needs_input`                   | A field it can't fill without you.                                |
| `needs_approval`                | An irreversible step. Only an explicit approval releases it.      |
| `needs_human`                   | A captcha, OTP, 2FA or login. No model can get past it.           |
| `needs_plan`                    | Plain language with no planner configured.                        |
| `checkpoint`                    | Stopped on purpose, with the page left open.                      |
| `done`, `failed`, `cancelled`   | Final states.                                                     |

Two rules hold for every task:

- **Reaching a payment page always stops the task.** It becomes a checkpoint that can't be continued, with the page left open for a person. The agent does not pay.
- **Secret values never reach a model.** Facts you mark secret are shared as `${name}` templates. The planner, the decision model and the rescue model see only the name, and the value is typed locally into the field.

Page and screen text is treated as data, never as instructions. The safety rules are plain code, not model judgment.

## Agent tools

| Tool | What it does |
| --- | --- |
| `browser` | Everything above through one `action` argument: `open`, `snapshot`, `read_page`, `click`, `fill`, `type`, `get_text`, `get_title`, `get_url`, `wait`, `press`, `hover`, `scroll`, `is_visible`, `find`, `task`, `task_continue`, `task_cancel`, `confirm_pending`, `list_downloads`, `wait_download`, `close`. |
| `browser_open` | Opens a URL and returns a session. |
| `desktop_list_apps`, `desktop_list_windows` | What is running and where. |
| `desktop_launch` | Starts an app. |
| `desktop_snapshot`, `desktop_find` | Reads the screen, or looks for one thing on it. |
| `desktop_goal`, `desktop_continue_goal` | Runs a desktop task and answers its stops. |

Irreversible steps go through OpenHuman's [approval gate](../approval-gate.md), tied to a digest of the exact action and URL. If no approval gate is installed, the action is denied.

## Platform support

| Platform | Desktop | Browser |
| --- | --- | --- |
| macOS | Yes. Needs Accessibility permission, plus Screen Recording for screenshots and Automation for the Notification Center opener. | Yes |
| Windows | Not yet | Yes |
| Linux | No | No |

A permission check runs before the first read. An accessibility API called by an unauthorized process usually returns an empty tree, not an error, and that looks the same as an app with no buttons. A missing permission produces a named refusal that says which setting to change.

Linux is not supported. The module registry publishes no Linux build, so neither desktop control nor browser automation is available there, even though the browser engine itself would run.

## Settings

**Connections > Computer Control** holds all of it: the module's status and contract version, the decision model (Jev, OpenJEV or Levanto Sage) and which route it bills through, the planner and rescue models, and the browser section (executable path, perception mode and allowed domains).

`[desktop] approvals_enabled` defaults to `false`. It governs one narrow thing: the module's own confirmation stop in the middle of a desktop goal. When it is off, the core continues automatically after the module re-observes and re-validates the exact target. When it is `true`, that stop shows as an approval card. It does not weaken the [approval gate](../approval-gate.md). An irreversible step still needs approval, and with no gate installed it is denied. Desktop goal confirmations are single-use and expire after 10 minutes.

## What it is good for

- Driving sites that have no API and no [native integration](../integrations/README.md).
- Multi-step UI flows where each snapshot shows the next element to act on.
- Automating a local app from inside a chat.

## See also

- [Web scraper](web-scraper.md): when you only need the article, not the whole page.
- [Jev](../../developing/jev.md): the decision model that picks each step.
- [Approval gate](../approval-gate.md): what gets parked, and what happens with no gate installed.
