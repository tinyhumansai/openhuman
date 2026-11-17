---
description: "Run the OpenHuman core inside a Rust application with one Runtime and independently configured Agents."
---

# Embedding OpenHuman

`openhuman-embed` runs the OpenHuman core inside your Rust program. Your host owns the user interface, server transport and authentication flow; the core owns agent turns, memory, tools, approvals and execution policy. Calling its typed facades does not require a JSON-RPC server or the desktop shell.

Build one `Runtime` per process and register the agents your product needs. Each agent can choose its own provider, prompt, working folder, tool scope, skills and MCP servers. Agents share runtime credentials; use `ProfileRuntime` when separate customers need separate credentials and workspaces.

## Start here

- [Installation](installation.md): dependency, Cargo gates, runtime setup and configuration.
- [Quickstart](quickstart.md): run a verified offline turn, then choose a live provider explicitly.
- [Architecture](architecture.md): ownership, lifecycle and the library chain.
- [Concepts](concepts/README.md): the 18 public API concepts and their runtime/agent scope.
- [Guides](guides/README.md): chatbots, multiple agents, SaaS, server deployment and tests.
- [Integrations](integrations/README.md): inference, MCP, connectors, storage and channels.
- [Cookbook](cookbook.md): generated inventory of executable examples.
- [API reference](api-reference.md): local rustdoc, source links and generated builder knobs.

## Choose the right surface

| Requirement | Start with |
| --- | --- |
| One agent with a small host | `Harness` |
| Several independently configured agents | `Runtime` and `AgentSpec` |
| Stateless model generation | `Completer` |
| Authenticated users with isolated credentials and state | `ProfileRuntime` |
| Public JSON-RPC server or shipped desktop host | The `openhuman-rpc` host layer |

The snippets in this section are injected from compiled, offline-tested Rust examples. Their full source files include the surrounding setup. `pnpm docs:check` checks source drift, local links, the generated API inventory and the published English mirror.

See [FAQ](faq.md) for design choices and [troubleshooting](troubleshooting.md) for setup failures.
