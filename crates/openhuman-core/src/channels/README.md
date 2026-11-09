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

Production code uses the portable TinyChannels bus contracts from
`tinychannels-bus` and the compiled module client. Provider algorithms,
transports, and concrete adapters stay in TinyChannels; OpenHuman does not
link the provider implementation library or expose a host-side provider trait.
The module prepares inbound identity and typed content before the host applies
its account authorization and session policy. Host callbacks handle only
capabilities the module cannot own, such as scoped media access, voice, and
event publication.

The TinyChannels v2 bus is also the source of the typed channel and web-event
wire contracts. OpenHuman passes those values across the module boundary and
keeps only host-specific routing, authorization, and lifecycle data around
them. It does not maintain a parallel provider API or re-map canonical web
output events in the host.

## Hosted and relay paths

Hosted-channel messages arrive through the backend socket and are handled by
[`bus/subscriber.rs`](./bus/subscriber.rs). The host prepares the caller identity and opens a
private module runtime for progressive delivery. The module owns drafts,
typing, retries, and idempotent final delivery; the host callback provides the
captured managed REST transport.

Agent execution uses the shared TinyAgents turn runner. OpenHuman selects the
agent, applies account and approval policy, binds the session, and supplies
host capabilities. TinyAgents owns generic admission, queueing, and turn-run
mechanics; TinyChannels owns provider transport and delivery behavior.

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

| Path                                                                                                         | Purpose                                                                        |
| ------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------ |
| [`mod.rs`](./mod.rs)                                                                                         | Feature gates, TinyBus module setup, and host-facing channel contracts.        |
| [`tinychannels-bus channel contracts`](../../../../vendor/tinychannels/crates/tinychannels-bus/src/channel/) | Portable message and typed send contracts.                                     |
| [`controllers/`](./controllers/)                                                                             | OpenHuman `channels.*` operations and host-side adaptation of the registered channel callback. |
| [`host/`](./host/)                                                                                           | Typed module callbacks, event subscribers, and remote-control host operations. |
| [`runtime/`](./runtime/README.md)                                                                            | Module lease startup, account lifetime, and the OpenHuman dispatch pipeline.   |
| [`bus/`](./bus/)                                                                                             | Hosted-channel inbound and durable progressive delivery integration.           |
| [`providers/relay/`](./providers/relay/README.md)                                                            | Gateway relay RPC surface and host-side relay orchestration.                   |
