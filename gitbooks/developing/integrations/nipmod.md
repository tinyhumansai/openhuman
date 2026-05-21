# Nipmod MCP Package Archive

This page documents an optional review path for connecting OpenHuman to Nipmod through OpenHuman's generic MCP client bridge.

Status: optional third-party review packet. This is not an official OpenHuman integration unless maintainers choose to accept and support it.

## Why this fits

OpenHuman can register named remote MCP servers through `mcp_client.servers`.

Nipmod exposes a hosted read-only MCP endpoint for agent package discovery:

```text
https://nipmod.com/api/mcp
```

The hosted endpoint is deliberately limited to read-only package workflows:

- `nipmod.search`
- `nipmod.view`
- `nipmod.inspect`
- `nipmod.install_plan`
- `nipmod.demo`

It does not install packages or write files. It lets an OpenHuman agent search packages, inspect trust evidence and return an install plan before any workspace write.

## Config

Add the server to OpenHuman `config.toml`:

```toml
[mcp_client]
enabled = true

[[mcp_client.servers]]
name = "nipmod"
endpoint = "https://nipmod.com/api/mcp"
description = "Nipmod shared package archive for agents. Search packages, inspect trust and create install plans before workspace writes."
enabled = true
allowed_tools = [
  "nipmod.search",
  "nipmod.view",
  "nipmod.inspect",
  "nipmod.install_plan",
  "nipmod.demo"
]
timeout_secs = 30
```

## Smoke test

Ask an OpenHuman agent:

```text
Use the registered nipmod MCP server. Run mcp_list_servers, mcp_list_tools for nipmod, then use mcp_call_tool to search for gitlawb-repo-reader, inspect trust and return an install plan. Do not install packages or write files.
```

Expected behavior:

- `nipmod` appears as a registered MCP server.
- Nipmod read-only tools are visible through the generic MCP bridge.
- The agent can search the Nipmod registry and return a plan without writing to the workspace.

## Public review packet

Nipmod publishes the matching review packet here:

```text
https://nipmod.com/openhuman
```

Config mirror:

```text
https://nipmod.com/integrations/openhuman/openhuman.mcp-client.toml
```

Claim boundary:

- Accurate: OpenHuman can review a Nipmod MCP connection packet.
- Not claimed: OpenHuman officially supports Nipmod.
- Not claimed: OpenHuman-owned packages or skills are published through Nipmod.
