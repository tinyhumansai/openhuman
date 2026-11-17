---
description: "Answers to common host, lifecycle, feature and provider choices."
---

# Embedding FAQ

## Can I create more than one runtime?

An ordinary embed process supports one `Runtime`; a second build returns `AlreadyRunning`. Add agents to the existing runtime. A multi-user host uses `ProfileRuntime` instead and needs a process dedicated to that mode. See [runtime](concepts/runtime.md) and [profiles](concepts/profiles-saas.md).

## Is an agent clone an independent agent?

No. It is another handle to the same agent instance, context and state. Register a different `AgentSpec` ID for a separate agent. Runtime credentials are still shared, so agent IDs alone do not isolate authenticated customers.

## Does lean mean fewer dependencies?

`RuntimeBuilder::lean()` selects a smaller runtime surface. Cargo still compiles enabled gates; use `--no-default-features` and explicit named gates for graph reduction. The [compiled matrix](capability-matrix.md) reports one fixed default build, and the [lean guide](guides/lean-headless.md) separates those controls.

## Why do MCP or skills methods disappear while the report says they are compiled?

The report describes core Cargo gates. Named Embed gates control facade exports too. Enable Embed's `mcp` or `skills` feature for their setters/re-exports, even when core defaults already compile the implementation. `channels` is named in Embed defaults.

## Can I supply an in-process model without HTTP?

Yes: `Provider::custom` takes the native `ChatModel<()>` contract. A provider can add an ordered fallback chain and explicit workload role pins. [Providers](concepts/providers.md) describes the rule that fallback stops after visible output.

## Do I need the backend for every agent turn?

An explicit BYOK/local/native provider can run independently of hosted inference. Hosted integrations, voice, billing or other backend surfaces need an installed backend transport and a credential. Use the [TinyHumans layer](integrations/tinyhumans-managed.md) for managed calls; Embed alone reports backend unavailable.

## Can an old handle control a replacement with the same ID?

No. Removal binds cancellation, callbacks and polling handles to the original lifecycle. It keeps the ID reserved until teardown/purge completes; later old handles cannot decide replacement approvals or send replacement turns. Dropping a handle does not purge its persisted home.

## Why did a prompt or tool change not affect a resumed thread?

A resumed session reuses recorded system messages and tool declarations. Start a new session for the changed prompt/tool catalog. Compaction preserves prior generations; it does not rewrite the old conversation. See [turns and sessions](concepts/turns-sessions.md).

## Where is the complete builder knob list?

[RuntimeBuilder setters](builder-setters.md) is generated from actual consuming public setters, with source signatures. Host/weight presets and non-setter methods are intentionally documented through [API reference](api-reference.md).
