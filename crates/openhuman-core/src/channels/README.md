# Channels

This domain owns OpenHuman's channel configuration, RPC surface, account
policy, and agent-turn integration. Provider transports and construction live
in the TinyChannels module, which OpenHuman loads through TinyBus. The host
does not build provider implementations or fall back to local listeners when
the module is unavailable.

## Native runtime

[`runtime/`](./runtime/README.md) opens an identity-scoped module lease for
configured native listeners and an optional gateway relay. Before dispatch,
the host asks the module to prepare effective timeout and concurrency limits.
The host supplies typed callbacks for host-owned operations such as voice,
allowlist persistence, event publication, and remote-control access. The
module owns provider-specific startup, supervision, transport, and interaction
behavior; OpenHuman owns routing, approvals, agent execution, and the session
cancellation lifetime.

The normal provider type and provider re-exports are available only to tests
that retain legacy compatibility fixtures. Production code uses the portable
TinyChannels bus contracts from `tinychannels-bus` and the module client.

## Hosted and relay paths

Hosted-channel messages arrive through the backend socket and are handled by
[`bus/subscriber.rs`](./bus/subscriber.rs). The host prepares the caller identity and opens a
private module runtime for progressive delivery. The module owns drafts,
typing, retries, and idempotent final delivery; the host callback provides the
captured managed REST transport.

Gateway-relayed inbound messages use [`providers/relay/`](./providers/relay/README.md).
The host preserves authenticated relay provenance outside serialized message
metadata, then runs the message through the same routing and agent policy as a
native listener. Channel operations and relay delivery use typed module
requests rather than a local provider implementation.

## Host policy

OpenHuman remains responsible for selecting the target agent, enforcing
channel permissions, handling chat approvals, and applying the captured
account/session lifetime. A missing module or malformed module response fails
the affected operation with a sanitized error. There is no host-side provider
or shell-based substitute.

## Layout

| Path | Purpose |
| --- | --- |
| [`mod.rs`](./mod.rs) | Feature gates and the public channel contract exports. Concrete provider exports are test-only. |
| [`traits.rs`](./traits.rs) | Portable message and send contracts re-exported from `tinychannels-bus`. |
| [`controllers/`](./controllers/) | OpenHuman `channels.*` operations and host-side adaptation of the registered channel callback. |
| [`host/`](./host/) | Typed module callbacks, event subscribers, and remote-control host operations. |
| [`runtime/`](./runtime/README.md) | Module lease startup, account lifetime, and the OpenHuman dispatch pipeline. |
| [`bus/`](./bus/) | Hosted-channel inbound and durable progressive delivery integration. |
| [`providers/relay/`](./providers/relay/README.md) | Gateway relay RPC surface and host-side relay orchestration. |
