---
description: "The ownership and lifecycle rules behind the Embed public API."
---

# Concepts

Start with Runtime and agents to understand which settings are shared. Then follow the lifecycle of a request through provider selection, tools, approvals and its durable session. The scope tables distinguish runtime composition from the state a host must isolate per agent or profile.

- [Runtime](runtime.md): A Runtime owns one initialized core and the process-wide resources that its agents share.
- [Runtime defaults](runtime-defaults.md): Runtime defaults form the starting point for newly registered agents; explicit agent settings override them.
- [Agents](agents.md): An Agent combines one identity, derived context and independently configured tool/model behavior on a shared Runtime.
- [Access and approvals](access-approvals.md): Access configures turn identity and permission policy; the host chooses how parked requests receive decisions.
- [Providers](providers.md): A provider supplies a route or native ChatModel implementation, with agent overrides and optional ordered fallback.
- [Turns and sessions](turns-sessions.md): A turn is one native model/tool loop; a session is the durable identity that makes subsequent turns a conversation.
- [Tools](tools.md): Tool catalogs expose only the allowed execution surface; host tools use the same native Tool contract as built-in tools.
- [MCP](mcp.md): MCP servers add remote tool discovery and execution to one agent, with explicit server configuration and tool allowlists.
- [Skills](skills.md): Skills are copied bundles of instructions and resources discovered inside an agent’s own home.
- [Memory](memory.md): The typed memory facade binds a namespace before learning, retrieving or forgetting records.
- [Cron](cron.md): Cron provides durable named jobs, explicit run-now execution and job history addressed to registered agents.
- [Channels](channels.md): A channel listener binds external messages to one registered agent and keeps transport lifecycle under host control.
- [Profiles and SaaS](profiles-saas.md): ProfileRuntime isolates authenticated users by workspace, credentials, conversations and event streams.
- [Completer](completer.md): Completer sends stateless model requests without booting a Runtime or entering the agent tool loop.
- [Hooks and seams](hooks-seams.md): Host seams replace infrastructure ports; hooks observe or control selected turn and tool boundaries.
- [Observability](observability.md): Runtime metadata events track lifecycle and approvals; turn streams carry per-turn progress and the final result.
- [Testing](testing.md): Deterministic local providers and explicit observations prove behavior without contacting real inference or backend services.
- [Errors](errors.md): Typed errors distinguish build availability, expected domain states, host setup failures and removed agent instances.
