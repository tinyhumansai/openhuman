---
description: How the agent uses your connected third-party services as tools.
icon: plug
---

# Third-party integrations

The agent can call the [services you connect](../integrations/README.md), such as Gmail, Notion, GitHub, Slack, Lark/Feishu, Stripe and Calendar, through one proxied tool surface.

## How the agent sees them

Once you connect a service with OAuth, its actions become callable tools. The agent does not need to know whether a tool talks to Gmail or a local file. It calls the tool, the proxy routes the request through the OpenHuman backend with your token, and the result comes back like any other tool output.

Some examples of what you can ask:

- "Send a message to #engineering on Slack."
- "Create an issue in the openhuman repo."
- "What's on my calendar tomorrow?"
- "Pull the last 20 Stripe charges over $1000."

## Native and proxied services

Some services have a native provider. That is a Rust module that ingests the service into [memory](../memory.md) directly, such as Gmail's native ingest path. Other services are proxied tools only. The agent can call them, but nothing is ingested automatically yet.

Lark/Feishu has two surfaces. One is a native real-time channel for sending and receiving messages. The other is a Composio-proxied workspace toolkit for chat, docs, wiki and meeting actions, available when the backend allowlist exposes it. Backfilling old Lark chat and docs into memory is not a native provider yet.

## Privacy boundary

For Composio-proxied integrations, the core never calls a third-party API directly. Requests go through the OpenHuman backend, which handles OAuth tokens and rate limiting. Your tokens never sit on your disk in plaintext, and the agent only sees the results of tool calls, never the credentials. Native channels such as Lark/Feishu use their own local configuration, so review them separately from the Composio OAuth boundary.

## See also

- [Third-party integrations catalog](../integrations/README.md): the OAuth flow and connection management.
- [Memory](../memory.md): how connected services become memory sources.
- [Privacy and security](../privacy-and-security.md): the full boundary.
