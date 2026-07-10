---
description: >-
  118+ third-party integrations - Gmail, Notion, GitHub, Slack, Stripe, Calendar
  and more - with one-click OAuth and zero API keys.
icon: plug
---

# Third-party Integrations (118+)

OpenHuman ships with backend-proxied access to **118+ third-party services**. Connecting any of them through the managed path is a one-click OAuth flow inside the app, there are no API keys to wire by hand, and no plugin marketplace to navigate.

Under the hood, the connector layer is powered by [Composio](https://composio.dev). In the default managed mode, OpenHuman's backend owns the Composio API key, OAuth token brokering, rate limits, and trigger webhook fan-out. If you switch to direct mode, the core talks to Composio with your own Composio API key; synchronous tool calls work, but real-time trigger webhooks must be configured on your own webhook infrastructure.

Once a service is connected, it shows up in four places at once:

1. As an **agent tool**, the model can call it directly.
2. As a **memory source**, [auto-fetch](../obsidian-wiki/auto-fetch.md) syncs it into the [Memory Tree](../obsidian-wiki/memory-tree.md) every twenty minutes.
3. As a **profile signal**, your activity across services feeds your personalization.
4. As a **trigger source**, live events (a new email, a new charge, an inbound DM) flow into the [Triggers](triggers.md) pipeline and can fire off agent actions automatically.

## Some of what's in the catalog

The catalog spans productivity, business, social, messaging and Google. A non-exhaustive sample:

| Category                | Examples                                             |
| ----------------------- | ---------------------------------------------------- |
| **Email & calendar**    | Gmail, Outlook, Google Calendar, Apple Calendar      |
| **Docs & storage**      | Google Docs, Google Drive, Notion, Dropbox, Airtable |
| **Code & dev**          | GitHub, Linear, Jira, Figma                          |
| **Comms**               | Slack, Discord, Microsoft Teams, Telegram, WhatsApp  |
| **CRM & sales**         | Salesforce, HubSpot                                  |
| **Commerce & payments** | Stripe, Shopify                                      |
| **Project management**  | Asana, Trello                                        |
| **Social**              | Twitter / X, Spotify, YouTube                        |

## Native vs proxied

Some services have **native providers**. Rust modules that know how to ingest the service into the Memory Tree directly (e.g. Gmail's native ingest path). Others are exposed as **proxied tools** only: the agent can call them, but there's no automatic ingest yet. New native providers are added as features land.

## How connections work

Click **Connect** on any integration. A browser window opens for OAuth. Once you sign in, the connection becomes active and OpenHuman starts syncing it through [auto-fetch](../obsidian-wiki/auto-fetch.md) on the next 20-minute tick.

Each integration shows its current status:

* **Not connected**. integration has not been set up.
* **Connected**. integration is active and being synced.
* **Manage**. active integration with options to reconfigure or disconnect.

You can revoke any connection at any time from the Skills tab.

## Messaging channels

Three integrations are special. OpenHuman uses them to _talk back_ to you, not just read from them:

* **Telegram**. the primary messaging channel. Two-way: send and receive messages, manage chats, search history, create groups, 80+ actions on your behalf. All actions run through your own encrypted credentials.
* **Discord**. send and receive messages via Discord. Connect your account to receive OpenHuman messages there.
* **Web**. a browser-based chat interface within the desktop app. Messages stay entirely local.

Set your default under **Settings → Automation & Channels → Messaging Channels**. The active route status shows which channel is currently in use. Telegram offers two credential modes: connect via OpenHuman (one-click, encrypted) or provide your own credentials for maximum control.

## Beyond the curated catalog: MCP & Skills

The 118+ OAuth connectors are the curated path. Beyond them, OpenHuman opens up the wider open-tooling ecosystem:

* **MCP servers**: a built-in registry browses thousands of [Model Context Protocol](https://modelcontextprotocol.io) servers (Smithery + the official registry) that install locally as new agent tools.
* **Skills**: a browsable, ~90,000-entry catalog of `SKILL.md` capability bundles aggregated from HermesHub, ClawHub, LobeHub and more. (Note: the old in-app skills runtime has been removed; Skills are now a metadata catalog you install from the Skills tab.)

See [MCP Servers & Skills](mcp-and-skills.md) for the full picture.

## Native voice and tools

Two capabilities ship native rather than as integrations because they're load-bearing for the desktop experience:

* [**Voice**](../native-tools/voice.md). STT in, TTS out, plus a live Google Meet agent that joins meetings, transcribes them into your Memory Tree, and can speak back into the call.
* [**Native tools**](../native-tools/README.md). built-in web search, web-fetch scraper, and a full filesystem/git/lint/test/grep coder toolset that the agent uses out of the box.

## Privacy boundary

OpenHuman's core never calls any third-party API directly. All requests go through the OpenHuman backend, which handles OAuth tokens and rate limiting. Your tokens never sit on disk in plaintext on your machine, and the agent only sees the _results_ of tool calls, not the credentials.

If you opt into direct Composio mode, that boundary changes: your local core uses your own Composio API key and you are responsible for the Composio account, rate limits, billing relationship, and any webhook endpoint needed for trigger delivery.

See [Privacy & Security](../privacy-and-security.md) for the full boundary.

## See also

* [Triggers](triggers.md), live events from connected integrations and how they fire agent actions.
* [Auto-fetch from Integrations](../obsidian-wiki/auto-fetch.md)
* [Memory Tree](../obsidian-wiki/memory-tree.md)
