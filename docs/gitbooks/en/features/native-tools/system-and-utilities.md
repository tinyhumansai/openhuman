---
description: Shell, node, Python, current time and push notifications, the small tools that round out the toolbelt.
icon: gear
---

# System and utilities

These are small tools the agent uses to finish a task.

Use the `shell` tool for Node.js, npm, and Python commands. OpenHuman uses the host's `node` and `python3` executables on `PATH`; install them with the platform package manager when needed.

## Tools in the family

| Tool | What it does |
| --- | --- |
| `shell` | Run a shell command. Output is bounded and the exit code is captured. |
| `current_time` | Get the current time in any timezone, with formatting options. |
| `schedule` | Do something once at a given time. For recurring jobs see [Cron](cron.md). |
| `pushover` | Send a push notification to your devices. |
| `lsp` | Query a language server for definitions, references and diagnostics. |
| `read_workspace_state` | Inspect the current working folder: open files, recent edits, environment. |
| `proxy_config` | Read or change proxy configuration for outbound requests. |
| `tool_stats` | Show which tools were used in this session and how often. |

## What they are good for

- The parts of a workflow that do not fit a richer tool family.
- "Just run this command and tell me what it printed."
- Time-aware behavior ("what time is it for the user right now?") without baking timezone assumptions into prompts.
- Notifying you when a long-running job is done.

## See also

- [Coder](coder.md): for file-heavy work, prefer the dedicated tools over `shell`.
- [Cron and scheduling](cron.md): for anything recurring.
