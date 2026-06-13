---
description: >-
  The full toolset OpenHuman's agent has out of the box - research, code,
  control your machine, schedule jobs, talk back to you, and call into 118+
  third-party services.
icon: toolbox
---

# Native Tools

OpenHuman's agent doesn't ship empty. Every model behind the agent has a curated set of tools available the moment you install - no plugin marketplace, no API keys to wire up, no MCP servers to register. The whole toolbelt is in the box.

This page is the index. Each subpage covers one family of tools.

## Why ship them natively

A plugin-only model means tools live in different processes, behind RPC, with their own auth and packaging stories. That's fine for open-ended extensibility, but for the **core** tools every agent needs (read a file, search the web, edit code, set a reminder, join a meeting), shipping them in-process means:

* Consistent error handling.
* Zero install friction.
* All output passes through [Smart Token Compression](../token-compression.md) for free.
* Predictable security boundary - filesystem tools respect workspace scoping, and network tools use the managed OpenHuman proxy by default unless you opt into a self-hosted path such as SearXNG.

## The toolbelt

| Family | What it covers |
| ------ | -------------- |
| [Web Search](web-search.md) | Search the live web via the managed proxy, or opt into self-hosted SearXNG. |
| [Web Scraper](web-scraper.md) | Pull clean text out of any URL - articles, docs, READMEs. |
| [Coder](coder.md) | Read/write/edit/patch files, glob, grep, git, lint, test. |
| [Browser & Computer Control](browser-and-computer.md) | Open URLs, screenshot, click, type, move the mouse. |
| [Cron & Scheduling](cron.md) | Recurring jobs, one-off reminders, scheduled agent runs. |
| [Voice](voice.md) | Speech-to-text in, text-to-speech out, live Google Meet agent. |
| [Memory Tools](memory-tools.md) | Recall, store, forget, and search the [Memory Tree](../obsidian-wiki/memory-tree.md). |
| [Third-party Integrations](../integrations/README.md) | The agent's view of the [118+ connected services](../integrations/README.md). |
| [Agent Coordination](agent-coordination.md) | Spawn subagents, delegate to skills, plan, ask the user. |
| [System & Utilities](system-and-utilities.md) | Shell, node, SQL, current time, push notifications, LSP. |

## See also

* [Smart Token Compression](../token-compression.md) - what keeps tool output costs bounded.
* [Third-party Integrations](../integrations/README.md) - the user-facing pitch and OAuth flow for the 118+ catalog.
* [Privacy & Security](../privacy-and-security.md) - the boundary every tool runs inside.
