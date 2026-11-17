---
description: >-
  Managed OAuth into Gmail, Notion, GitHub, Slack, Stripe, Calendar and more,
  with one-click sign-in and no API keys.
icon: plug
---

# Third-party integrations

OpenHuman gives you managed access to 119 toolkits. Connecting one is a one-click OAuth flow inside the app. You don't wire up API keys or browse a plugin marketplace.

The 119 is the catalog the app ships. What you can connect is whatever the backend's allowlist returns when the page loads. The actions for each toolkit are fetched live, so no fixed number bounds the total.

The connector layer is powered by [Composio](https://composio.dev). In the default managed mode, OpenHuman's backend owns the Composio API key, OAuth token handling, rate limits and trigger webhooks. In direct mode, the core talks to Composio with your own API key. Synchronous tool calls work in that mode, but you must set up your own webhook infrastructure for real-time triggers.

Once a service is connected, it shows up in three places:

1. As an agent tool, which the model can call directly.
2. As a profile signal. Your activity across services feeds your personalization.
3. As a trigger source. Live events (a new email, a new charge, an inbound DM) flow into the [triggers](triggers.md) pipeline and can fire agent actions automatically.

## What is in the catalog

The catalog spans productivity, business, social, messaging and Google. Here is a sample:

| Category                | Examples                                             |
| ----------------------- | ---------------------------------------------------- |
| Email and calendar      | Gmail, Outlook, Google Calendar, Apple Calendar      |
| Docs and storage        | Google Docs, Google Drive, Notion, Dropbox, Airtable |
| Code and dev            | GitHub, Linear, Jira, Figma                          |
| Comms                   | Slack, Discord, Microsoft Teams, Telegram, WhatsApp  |
| CRM and sales           | Salesforce, HubSpot                                  |
| Commerce and payments   | Stripe, Shopify                                      |
| Project management      | Asana, Trello                                        |
| Social                  | Twitter / X, Spotify, YouTube                        |

## Native and proxied

Some services have native providers. These are Rust modules that load the service straight into memory, such as Gmail's native ingest. Others are proxied tools only: the agent can call them, but nothing is ingested automatically. Native providers are added over time.

## How connections work

Click **Connect** on any integration. A browser window opens for OAuth. Once you sign in, the connection is active, and you can add it as a [memory source](../memory.md) to sync on a schedule.

Each integration shows its status:

- **Not connected**: it has not been set up.
- **Connected**: it is active and syncing.
- **Manage**: it is active, with options to reconfigure or disconnect.

You can revoke any connection at any time from the **Connections** page.

## Messaging channels

Some integrations are not just something to read from. OpenHuman also uses them to talk back to you. Eight channels have a setup flow in the app, and more can be enabled by hand in `config.toml`. The three to start with:

- **Telegram** is the usual first choice. It is two-way, with a managed one-click connection or your own bot token.
- **Discord** is two-way, through OAuth or your own bot token, with a server and channel picker.
- **Web** is the chat inside the desktop app. Messages stay entirely local.

Set your default under **Connections > Channels**. [Messaging channels](../channels.md) lists them all and what each can do.

## Beyond the curated catalog

The managed OAuth connectors are the curated path. Two more routes open up the wider ecosystem:

- **MCP servers.** A built-in registry browses thousands of [Model Context Protocol](https://modelcontextprotocol.io) servers (Smithery and the official registry) and installs them locally as new agent tools.
- **Skills.** A browsable catalog of `SKILL.md` capability bundles from several public registries, installed from the **Connections > Skills** tab. A skill runs as its own agent session with the interpreters it declares. It is not code inside the app.

See [MCP servers and skills](mcp-and-skills.md) for more.

## Native voice and tools

Two capabilities are built in rather than offered as integrations, because the desktop experience depends on them:

- [Voice](../native-tools/voice.md): speech in, speech out, and a live voice agent you can interrupt mid-sentence.
- [Native tools](../native-tools/README.md): built-in web search, a web-fetch scraper, and a full coder toolset (filesystem, git, lint, test and grep).

## Privacy boundary

In managed mode, OpenHuman's core never calls a third-party API directly. Requests go through the OpenHuman backend, which handles OAuth tokens and rate limits. Your tokens are never stored in plaintext on your machine, and the agent sees only the results of tool calls, not the credentials.

In direct mode that boundary changes. Your local core uses your own Composio API key, and you are responsible for the Composio account, its rate limits and billing, and any webhook endpoint needed for triggers.

See [Privacy and security](../privacy-and-security.md) for the full boundary.

## See also

- [Triggers](triggers.md): live events from connected integrations, and how they fire agent actions.
- [Memory](../memory.md)
- [MCP servers and skills](mcp-and-skills.md): the open tooling path beyond the curated catalog.
- [Integrations domain](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-core/src/integrations/README.md): the Rust side of this page.
- [Architecture](../../developing/architecture/README.md): how the backend proxy fits the core.
