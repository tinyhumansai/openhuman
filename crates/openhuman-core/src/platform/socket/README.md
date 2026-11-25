# Socket port

The core owns the stable Socket.IO RPC contract, state DTOs, credential
provider seam, and inbound domain-event sink. The hosted transport lives in
`crates/openhuman-tinyhumans/src/socket/` and uses the SDK's `SocketConnection`
supervisor. The core has no dependency on `tinyhumans-sdk`.

## Files

- `port.rs` defines `SocketPort` and the process registration slot.
- `models.rs` defines the serialized connection status and state DTOs.
- `token_provider.rs` supplies a fresh session credential to each connection
  attempt.
- `event_sink.rs` routes generic backend events into core domains. TinyHumans
  parses hosted device-tunnel wire events and publishes only core-owned
  semantic events through the device tunnel port.
- `ops.rs` and `schemas.rs` preserve the `socket.*` RPC methods.

The TinyHumans adapter owns connection lifecycle, endpoint validation, queued
ACK emits, and reconnection. It installs the port when `openhuman-tinyhumans`
is installed. Core-only hosts keep the socket surface unavailable.

The RPC namespace and function names remain stable: `socket.connect`,
`socket.disconnect`, `socket.state`, `socket.emit`, and
`socket.connect_with_session`. `ConnectionStatus` JSON remains lowercase.

Protocol tests for the SDK supervisor and reconnect behavior live in the SDK
repository. Core port, event-sink, schema, and credential-contract tests remain
beside these modules.
