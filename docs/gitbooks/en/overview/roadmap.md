---
description: >-
  What is being worked on next, and the known gaps, tracked in public issues.
icon: map
---

# Roadmap

This page is a reading of the open issue tracker, not a commitment. It has no dates. Each item links to the issue that owns it, so the issue stays the source of truth if this page goes stale.

## Taking off the early-beta badge

[This checklist](https://github.com/tinyhumansai/openhuman/issues/6047) defines done. The badge comes off when its boxes are closed, the memory smoke test passes on macOS, Windows and Linux, and the release workflow can publish without hand-holding. A ticked box is not always a passing test, so read each box against the text beside it.

| Area | State |
| --- | --- |
| Memory end to end (store, restart, recall, and the agent actually uses it) | Two of three items ticked ([one](https://github.com/tinyhumansai/openhuman/issues/6041), [two](https://github.com/tinyhumansai/openhuman/issues/6040)). The third is the end-to-end smoke test, which the checklist records as not passing yet. |
| Reply persistence: a completed reply never vanishes | Done |
| Clean-install sign-in on all three platforms | [Open](https://github.com/tinyhumansai/openhuman/issues/6020) |
| Windows: native modules are refused when `%TEMP%` grants Modify to a non-owner group, which leaves memory and connectors unavailable | [Open](https://github.com/tinyhumansai/openhuman/issues/6008) |

The release-pipeline and Windows path-length blockers on the checklist have since been closed, though their boxes are still unticked.

## Larger efforts

- **Agent to agent.** [This umbrella issue](https://github.com/tinyhumansai/openhuman/issues/3463) covers five phases: an agent-card endpoint and discovery, accepting tasks, delegating to an external agent as a tool, streaming and push notifications, and multi-instance coordination.
- **Pluggable memory adapters.** [This issue](https://github.com/tinyhumansai/openhuman/issues/5390) adds an adapters directory and the dependency rule. Supermemory, mem0, agentmemory and cognee would follow. Today [memory](../features/memory.md) has two engines and no adapter directory.
- **External agent runtimes.** [This issue](https://github.com/tinyhumansai/openhuman/issues/4731) adds a layer for driving Cursor, Windsurf and Codex, with a shared ACP transport for more and per-session model and effort selection. Session import from one of them is the active part.
- **Local and self-hosted.** [One issue](https://github.com/tinyhumansai/openhuman/issues/6130) would bring back a managed local runtime with model downloads and a compute-backend selector. Today local inference means an endpoint you run yourself. [Another](https://github.com/tinyhumansai/openhuman/issues/4844) is a no-UI Linux build for a server or a Raspberry Pi.
- **Observability.** [This issue](https://github.com/tinyhumansai/openhuman/issues/4496) adds scores to traces, so user feedback and automated quality signals sit next to the run they describe.

## Built but not working yet

Two surfaces exist in the build and do nothing:

- Follow-up suggestion chips are built but nothing produces them, so the row is always empty ([issue](https://github.com/tinyhumansai/openhuman/issues/6465)).
- SearXNG is configurable and reachable over the MCP server, but `searxng_search` is not registered as an agent tool, so the agent in chat cannot use it ([issue](https://github.com/tinyhumansai/openhuman/issues/6127)).

The app also lists its own maturity. **Settings → About** shows every capability as stable, beta or coming soon. It is a better answer than this page to "is X finished".

## Recently removed

These features were once documented and are gone:

- The live Google Meet agent. It joined a call through the old embedded Chromium webview and spoke back as a camera stream. See [Voice](../features/native-tools/voice.md).
- The embedded Chromium (CEF) runtime. The shell now uses Tauri's native webview, and browser control drives a real Chrome over the DevTools protocol. See [Browser and computer control](../features/native-tools/browser-and-computer.md) and the historical [CEF notes](../developing/cef.md).
- In-app model downloads and runtime management for local models. Point OpenHuman at a server you run.
- The skills sandbox. Skills are now a catalogue you browse and install, plus a run surface, not code running in a JavaScript sandbox in the app.
- The memory tree, graph view, people list and `MEMORY.md`, replaced by the per-turn memory pack. [Memory](../features/memory.md) has the full list.

## See also

- [Release policy](../developing/release-policy.md): how a release is cut and what the version gate does.
- [Platform and availability](../features/platform.md): what runs where today.
