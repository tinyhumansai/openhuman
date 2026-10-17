---
description: >-
  Go beyond the curated OAuth connectors: browse thousands of MCP servers and
  a catalog of about 90,000 skills, and let OpenHuman act as an MCP server.
icon: blocks
---

# MCP servers and skills

The [one-click OAuth integrations](README.md) are the curated path. Beyond them, OpenHuman opens up the wider tooling ecosystem in two ways: the Model Context Protocol (MCP) registry and the skills catalog. OpenHuman can also act as an MCP server for other clients.

## MCP servers

A built-in registry browses the open MCP ecosystem and installs servers locally as new tools for the agent.

- **Two registries, merged.** Search queries [Smithery.ai](https://smithery.ai) and the official [MCP registry](https://registry.modelcontextprotocol.io) in parallel and merges the results. Together they hold thousands of servers.
- **Search, install, connect.** Search the catalog, open a server's details and install it. A local install runs the server as a stdio subprocess. Deployed servers connect over HTTP.
- **Supervised connections.** Installed servers are saved in a local SQLite store (`mcp_clients/mcp_clients.db`) with their command, arguments and transport. A supervisor keeps enabled servers connected. It probes about every 60 seconds and backs off per server after failures. Unusual probe results (a slow answer, a dropped transport, a reconnect that worked or failed) appear in **Settings > Developer > Event Log** under the `mcp` badge. A healthy probe is silent. You get a notification when a server stays down, comes back after failing, or cannot start because its launcher runtime is missing.

Once connected, an MCP server's tools work for the agent exactly like built-in tools. The catalogs grow on their own, so there is no fixed list baked into OpenHuman.

### OpenHuman as an MCP server

It also works the other way. `openhuman-core mcp` exposes OpenHuman over stdio as an MCP server for clients such as Claude Desktop. It offers memory tools (recall with citations, fetch, list, learn and forget) and optional web search. See [MCP server](../../developing/mcp-server.md) for setup.

## Skills

Skills are a large, browsable catalog of `SKILL.md` capability bundles, gathered from several public sources such as HermesHub, ClawHub and LobeHub.

- **One catalog.** It comes from HermesHub, and you can point to another source with `OPENHUMAN_SKILL_REGISTRY_CATALOG_URL`. It holds roughly 90,000 entries. Each entry has an id, name, description, source, author, version, tags, platforms, a download URL and a license.
- **Paged and cached.** The `tinyskills` skill registry serves the catalog. The app asks for one page at a time (25 rows, filtered and ranked by the core), and opening an entry loads its full detail. The catalog loads in the background at startup without blocking it, and is saved at `~/.openhuman/skill-registry/hermes.json` (`OPENHUMAN_SKILL_REGISTRY_CACHE_DIR` moves it). While a refresh runs, or when the registry cannot be reached, the tab keeps showing the saved catalog, says so, and offers Retry. After a failed refresh the registry waits 45 seconds (longer if the upstream asks) before trying again, so a Retry inside that window shows the saved catalog without a new download.
- **Guarded downloads.** Installs fetch `SKILL.md` over HTTPS only, refuse non-public addresses and re-check every redirect. Entries without a `SKILL.md` (LobeHub agents, portal pages) show "Not installable" with a link to their source page. `OPENHUMAN_SKILL_REGISTRY_DOWNLOAD_BASE_URL` serves every `SKILL.md` from a mirror, and `OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP=1` allows a loopback `http` mirror for local fixtures.
- **Security scan before install.** Every downloaded `SKILL.md`, from the catalog or a pasted URL, is scanned for supply-chain red flags such as invisible characters and hard-coded credentials. If the scan blocks it, or the download fails, OpenHuman fetches and scans it once more. If it still blocks, nothing is installed and the app shows the findings with two choices: **Block install** (the default) or **Install anyway**. Install anyway installs exactly the document you reviewed: if it changed in the meantime, it is scanned again and you are asked again. Only you can choose Install anyway. An agent that tries to install a blocked skill is told to ask you to do it from the Skills page.
- **Metadata first.** Skills are a catalog you browse and install from the **Connections > Skills** tab. They are not code running inside the app. Availability varies by entry: some offer a direct `SKILL.md` download, and others point to external hosting.

## See also

- [Third-party integrations](README.md): the curated OAuth connectors.
- [MCP server](../../developing/mcp-server.md): running OpenHuman as an MCP server.
- [Available tools](../native-tools/README.md): the built-in tools.
