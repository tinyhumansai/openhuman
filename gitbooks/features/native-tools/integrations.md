---
description: The agent's view of the 119+ connected third-party services.
icon: plug
---

# Third-party Integrations

OpenHuman's agent can call into [119+ third-party services](../integrations/README.md) - Gmail, Notion, GitHub, Slack, Lark / Feishu, Stripe, Calendar, and the long tail - through a single proxied tool surface.

## How it shows up to the agent

Once you've connected a service via OAuth, its actions become callable tools. The agent doesn't need to know whether a tool talks to Gmail or to a local file - it just calls the tool, the proxy routes the request through the OpenHuman backend with your token, and the result comes back like any other tool output.

A few examples of what becomes available:

- "Send a message to #engineering on Slack."
- "Create an issue in the openhuman repo."
- "What's on my calendar tomorrow?"
- "Pull the last 20 Stripe charges over $1000."

## Native vs proxied

Some services have **native providers** - Rust modules that know how to ingest the service into the [Memory Tree](../obsidian-wiki/memory-tree.md) directly (e.g. Gmail's native ingest path). Others are exposed as **proxied tools** only: the agent can call them, but there's no automatic ingest yet. New native providers are added as features land.

Lark / Feishu currently has two surfaces: a native real-time channel for message send/receive, and a Composio-proxied workspace toolkit entry for chat, docs, wiki, and meeting actions when the backend allowlist exposes it. Historical chat/doc backfill into the Memory Tree is not yet a native provider; track that separately from the live channel connector.

## Privacy boundary

For Composio-proxied integrations, OpenHuman's core never calls any third-party API directly. Requests go through the OpenHuman backend, which handles OAuth tokens and rate limiting. Your tokens never sit on disk in plaintext on your machine, and the agent only sees the _results_ of tool calls, not the credentials. Native channels such as Lark / Feishu use their own local configuration and should be reviewed separately from the Composio OAuth boundary.

## See also

- [Third-party Integrations (catalog)](../integrations/README.md) - the user-facing pitch, OAuth flow, and connection management.
- [Auto-fetch](../obsidian-wiki/auto-fetch.md) - how connected services flow into the Memory Tree.
- [Privacy & Security](../privacy-and-security.md) - the full boundary.
