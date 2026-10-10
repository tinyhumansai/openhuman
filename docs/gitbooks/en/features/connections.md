---
description: >-
  One page for everything the agent plugs into: apps, channels, MCP servers,
  skills, memory, tools, the computer, the wallet, and every API key.
icon: plug
---

# Connections

Connections is the hub. Almost every setting that changes what the agent can do, as opposed to how the app looks, lives here. The left rail splits it into two groups. Old addresses such as `/skills` and `/channels` redirect here, so bookmarks still work.

## Integrations

| Tab | What it holds |
| --- | --- |
| OAuth | The OAuth connectors. Pick a toolkit, authorize it in a browser window, and choose its scope (read, write or admin) and which triggers may fire. See [Third-party integrations](integrations/README.md). |
| Channels | The channels the agent talks to you on: Telegram, Discord, the in-app web chat, Lark/Feishu, DingTalk, iMessage, email over IMAP and SMTP, and 元宝. See [Messaging channels](channels.md). |
| MCP Servers | Model Context Protocol servers, in four views: your servers, client config, the raw `mcp.json` (the only place a server is actually added), and a browsable registry. It also has a playground for calling one tool by hand. |
| Skills | Installed skill bundles, a registry to browse and install from, and a runner. See [MCP servers and skills](integrations/mcp-and-skills.md). |
| Brain | [Memory](memory.md): the engine, ask, explorer, learnings, conversations, the shared brain, background jobs and recall settings. |
| Agent tools | Which tools the agent may use, and the policy for each. |
| Browser Control | Desktop and browser control, the decision model and the planner. See [Browser and computer control](native-tools/browser-and-computer.md). |
| Wallet | Balances, send and receive, and the recovery phrase. See [Wallet](wallet.md). |

## API keys

The second group has mostly one tab per engine family. Each works the same way: use the managed TinyHumans route, or paste your own key. The exception is Usage, which reports spending and background activity and configures nothing.

| Tab | Covers |
| --- | --- |
| LLM | Model providers: the managed route, your own keys for 27 built-in providers, and local servers (Ollama, LM Studio, MLX, any OpenAI-compatible endpoint). See [Model routing](model-routing/README.md). |
| Composio | Your own connector-platform key, if you prefer it to the managed route. |
| Voice agents | Speech-to-text and text-to-speech providers, routing per workload, and the dictation hotkey. |
| Embeddings | The embedding provider behind memory and tool search. |
| Search | Web search providers. Two have a managed route. The rest need your own key. See [Web search](native-tools/web-search.md). |
| Usage | What you have spent, by day and by model, plus the background activity log. See [Billing, cost and usage](billing-and-usage.md). |

## Why one page

Connecting Gmail is an integration, a memory source, a trigger source and a set of agent tools all at once. GitHub is the same. Keeping them together means the thing you are setting up stays in front of you while you configure the parts that live in other systems.

Settings that change how the app behaves, such as appearance, personality, security, keychain and autonomy, stay in [Settings](settings.md).

## See also

- [Getting started](../overview/getting-started.md): the first-run path through this page.
- [Memory](memory.md), [Messaging channels](channels.md), [Third-party integrations](integrations/README.md): the three tabs with the most behind them.
- [Privacy and security](privacy-and-security.md): what each connection means for what leaves your machine.
