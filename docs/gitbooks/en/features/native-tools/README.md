---
description: >-
  The toolset OpenHuman's agent has out of the box: research, code, control
  your machine, schedule jobs, talk back to you, and call your connected
  services.
icon: toolbox
---

# Native tools

The agent comes with a full set of tools the moment you install it. There is no plugin marketplace, no API keys to wire up and no MCP servers to register.

This page is the index. Each subpage covers one family of tools.

## Why built in

A plugin-only model puts tools in separate processes, behind RPC, each with its own auth and packaging. That suits open-ended extensions. For the core tools every agent needs (read a file, search the web, edit code, set a reminder, drive a browser), running in-process gives you:

- Consistent error handling.
- Nothing to install.
- Output that passes through [token compression](../token-compression.md) for free.
- A predictable security boundary. File tools respect workspace scoping. Network tools use the managed OpenHuman proxy by default unless you opt into a self-hosted path such as SearXNG.

## The toolbelt

| Family | What it covers |
| --- | --- |
| [Web search](web-search.md) | Search the live web through the managed proxy (powered by Exa), backend-proxied Parallel, your own Exa, Brave, Querit or Tavily key, or self-hosted SearXNG. |
| [Web scraper](web-scraper.md) | Pull clean text out of any URL: articles, docs, READMEs. |
| [Coder](coder.md) | Read, write, edit and patch files; glob, grep, git, lint, test. |
| [Documents](documents.md) | Write `.docx` and `.pptx`, and read PDF, Word, PowerPoint and Excel files. |
| [Browser and computer control](browser-and-computer.md) | Open URLs, inspect DOM snapshots, click, type, move the mouse. |
| [Cron and scheduling](cron.md) | Recurring jobs, one-off reminders, scheduled agent runs. |
| [Voice](voice.md) | Speech-to-text in, text-to-speech out, and a live voice agent you can interrupt. |
| [Memory tools](memory-tools.md) | Recall, fetch, learn and forget through the single `memory` tool. |
| [Third-party integrations](integrations.md) | The agent's view of your [connected services](../integrations/README.md). |
| [Agent coordination](agent-coordination.md) | Spawn subagents, delegate to skills, plan, ask the user. |
| [System and utilities](system-and-utilities.md) | Shell, node, SQL, current time and push notifications. |

## See also

- [Token compression](../token-compression.md): what keeps tool output costs bounded.
- [Third-party integrations](../integrations/README.md): the OAuth flow for the managed catalog.
- [Privacy and security](../privacy-and-security.md): the boundary every tool runs inside.
- [Tools domain](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-core/src/tools/README.md): the Rust module that registers these tools.
- [Agent harness](../../developing/architecture/agent-harness.md): how tool calls run inside a turn.
