# devices

Mobile-device pairing domain. Brokers a secure, end-to-end-encrypted tunnel between the Rust core and iOS clients over the TinyHumans backend's `tunnel:*` Socket.IO relay. TinySecurity owns the private device identity, handshake, session ciphers, frame encryption, and replay protection. OpenHuman retains channel ownership, controller policy, host callbacks, persistence adapters, and tunnel dispatch.

## Responsibilities

- Register a new pairing channel with the backend tunnel (`tunnel:register`) and return QR-bound fields (`channel_id`, `pairing_token`, `core_pubkey`, optional `rpc_url`, `expires_at`).
- Resolve the HumanOnly config master and unlock the TinySecurity pairing scope from a direct authenticated-user action.
- Connect as `role:"core"` on the channel (`tunnel:connect`) and listen for the device.
- On a fresh backend-attested socket, Renew the persisted module pairing and require the same device key to complete the existing `0x03` handshake and Confirm proof.
- Track live peer-online status from `tunnel:peer-status` and overlay it onto `devices_list` results.
- List non-revoked devices; revoke a device (soft delete) and tear down its in-memory + tunnel state.
- Hold only opaque TinySecurity session handles in memory. Restarted hosts resume stored channels after direct-user unlock and accept a same-key Renew handshake.
- Detect a best-effort LAN `rpc_url` for the direct-HTTP fast path.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/security/devices/mod.rs` | Export-only: module docstring, `pub mod` decls, re-exports of schema registry fns and public types. |
| `crates/openhuman-core/src/security/devices/types.rs` | Serde domain types: `PairedDevice`, `PairingSession`, and the three RPC response payloads. |
| `crates/openhuman-core/src/security/devices/rpc.rs` | RPC handler logic and in-memory module receipt/session handles, peer status, and LAN-URL detection. |
| `crates/openhuman-core/src/security/devices/schemas.rs` | Controller schemas, `all_controller_schemas`/`all_registered_controllers`, and `handle_*` bridges delegating to [`rpc.rs`](./rpc.rs). Mirrors `cron/schemas.rs`. |
| `crates/openhuman-core/src/security/devices/store.rs` | SQLite persistence (`paired_devices` table) via the per-call `with_connection` pattern. |
| `crates/openhuman-core/src/security/devices/tunnel_client.rs` | Core-owned typed `TunnelClientPort`, semantic registration DTO, and publishing functions for parsed peer/frame events. |
| `crates/openhuman-tinyhumans/src/devices/tunnel_client.rs` | TinyHumans Socket.IO tunnel wire DTOs, ACK and payload parsing, frame cap, and implementation of the core tunnel port. |
| `crates/openhuman-core/src/security/devices/bus.rs` | `DeviceTunnelSubscriber` event handler: validates pairing context, calls TinySecurity for handshake and tunnel crypto, persists new pairing rows, and updates peer status. |

## Public surface

Re-exported from [`mod.rs`](./mod.rs):

- `all_devices_controller_schemas` / `all_devices_registered_controllers` (alias for `schemas::all_controller_schemas` / `all_registered_controllers`).
- Types: `CreatePairingResponse`, `ListDevicesResponse`, `PairedDevice`, `PairingSession`, `RevokeDeviceResponse`.

Other notable public items (used across the crate but not re-exported at the domain root): `bus::register_device_tunnel_subscriber` and the semantic `tunnel_client::{TunnelClientPort, TunnelRegistration}` seam. Device key generation, handshake processing, and frame crypto run inside TinySecurity.

## RPC / controllers

Namespace `devices` (invoked as `openhuman.devices_<function>`):

| Method | Inputs | Output | Behavior |
| --- | --- | --- | --- |
| `devices_create_pairing` | `label?: string` | `CreatePairingResponse` | Registers a channel via Socket.IO ACK, generates+persists keypair, emits tokenless core `tunnel:connect`, returns QR fields using the backend-provided pairing expiry. |
| `devices_list` | none | `ListDevicesResponse` | A direct user call resolves the HumanOnly master, unlocks TinySecurity, and rejoins persisted channels before listing non-revoked devices. Other callers only list host metadata. |
| `devices_revoke` | `channel_id: string` | `RevokeDeviceResponse` | Soft-deletes the device, clears all in-memory state for the channel, publishes `DeviceRevoked`. |

Wired into the controller registry in `crates/openhuman-core/src/core/all.rs` (schemas + registered controllers + the `"devices"` namespace branch).

## Agent tools

None. This domain has no `tools.rs` and owns no agent tools.

## Events

Subscriber registered at startup from `crates/openhuman-core/src/core/runtime/subscribers.rs` via `register_device_tunnel_subscriber`. `DeviceTunnelSubscriber` (`name() = "device::tunnel"`, `domains() = ["device", "auth"]`) **handles**:

- `DevicePeerOnline` / `DevicePeerOffline` → update `PEER_STATUS`.
- `DeviceTunnelFrame` → complete handshake + persist `PairedDevice`.
- global `CredentialChanged { kind: "cleared" }` → clear host session capabilities; credential teardown Locks TinySecurity before the user scope is deactivated.
- graceful core shutdown → clear host session capabilities and Lock TinySecurity while the admitted scope remains available.

**Publishes**:

- `DevicePaired` (after successful handshake + persistence).
- `DeviceRevoked` (from `devices_revoke`).

Note: TinyHumans' `devices::tunnel_client` adapter parses the raw `tunnel:peer-status`, `tunnel:frame`, and `tunnel:evicted` Socket.IO payloads, then calls the core's semantic publishing functions. The device domain consumes those `DomainEvent`s; the core socket event sink does not know the hosted wire DTOs.

## Persistence

The host stores one `paired_devices` document per channel through the `tinystoragedrivers` document port:

| Column | Notes |
| --- | --- |
| `channel_id` | PK; 128-bit base32 channel id. |
| `label` | Human-readable label. |
| `device_pubkey` | Base64url X25519 device public key. |
| `core_session_token_hash` | Legacy column name; currently stores a SHA-256 hash of the pairing credential because the backend no longer mints a core session token. |
| `pairing_id` | Opaque TinySecurity owner id used for renewal, recovery, and revoke; hidden from RPC JSON. |
| `core_private_key_encrypted` | Legacy import source only; removed after successful TinySecurity recovery. New keys never enter host persistence. |
| `created_at` / `last_seen_at` | ISO 8601; `last_seen_at` set by `touch_device`. |
| `revoked` | Soft-delete flag; `list_devices` filters `revoked = 0`. |

DDL is created idempotently on every connection open (`with_connection`). `peer_online` is **not** persisted; it lives only in the in-memory `PEER_STATUS` map.

On restart, volatile receipts and frame handles are gone. A direct authenticated user action resolves the HumanOnly master, calls TinySecurity `Unlock`, and reconnects stored channels. A fresh backend socket token, nonce, channel, expiry, scope, user, and module generation bind `Renew`, `Handshake`, and `Confirm`; the old QR token is not reused. TinySecurity persists private identity and pairing state as ciphertext. The host stores only the module pairing id and public device metadata.

The backend's current peer-status event carries only `channelId` and `online`, without a connection id. The host therefore updates presence but does not mint a `SocketClosed` capability from an ambiguous offline event; the module session remains inaccessible after a nonce change and expires under TinySecurity's bound lifetime.

### On a storage backend

When the host configured a storage backend (`OPENHUMAN_STORAGE_URL` /
`[storage] url`, see `crate::storage`), every `store` function uses
`store_documents.rs` instead of `devices.db`: the same operations on the
`tinystoragedrivers` document port, under the current call's storage scope
(the acting agent; `local` on a single-user host; refused in SaaS mode with
no acting agent). One `paired_devices` document per `channel_id`. Pairing replaces the
document; touching and revoking are compare-and-swap, so a touch never
revives a device another process revoked. With no backend configured (the desktop default)
`devices.db` is used as described above.

## Dependencies

- `crate::config` (`Config`, `config::rpc::load_config_with_timeout`): workspace paths and config loading for handlers.
- `crate::modules::security`: confidential TinySecurity pairing operations and host-attested events.
- `crate::security::devices::tunnel_client::TunnelClientPort`: typed seam for the host-owned tunnel transport; no TinyHumans SDK dependency in core.
- `crate::core::bus::BUS` (`.publish`, `.subscribe`) plus `crate::core::events::DomainEvent` and `tinybus::EventHandler`: pub/sub for device tunnel events.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}`: controller registry contract.
- `crate::core::Outcome`: RPC handler return type.
- External crates: `tinysecurity-bus`, `base64`, `sha2`, `chrono`, `once_cell`, `tokio`, `async_trait`, `anyhow`.

## Used by

- `crates/openhuman-core/src/core/all.rs`: registers the `devices` controllers/schemas and namespace branch.
- `crates/openhuman-core/src/core/runtime/subscribers.rs`: calls `register_device_tunnel_subscriber()` at startup.
- `crates/openhuman-tinyhumans/src/devices/tunnel_client.rs`: parses backend `tunnel:*` wire events and implements the hosted tunnel client port.

## Notes / gotchas

- **Handshake framing** and `0x02` frame crypto run only through TinySecurity's confidential pairing and frame operations. OpenHuman does not handle private keys, derive tunnel keys, or encrypt frames.
- The `label` persisted on pairing currently falls back to the `channel_id` (the pending session stores no real label field; `PairingSession.channel_id` is used as the label source).
- `devices_revoke` only tears down local + in-memory state. There is **no backend revoke endpoint yet** (TODO referencing PR #709 follow-up); the backend channel is left to expire via the pairing-token TTL.
- `rpc_url` LAN detection uses the UDP "connect to 8.8.8.8" trick to read the local IPv4; port comes from `OPENHUMAN_CORE_RPC_PORT` env (default `7788`). Non-fatal if it fails.
- TinyHumans `tunnel:register` uses Socket.IO ACK with a 10-second timeout and converts the backend `{channelId, pairingToken, pairingExpiresAt}` response to the core's semantic `TunnelRegistration` shape.
- `PairingSession`, connection token/nonce, peer status, and active session handles are in-memory only; TinySecurity holds durable pairing identity ciphertext. The token used for a Renew is fresh per backend connection and is not the old QR token.
- Outbound `tunnel:frame` payloads are capped at 64 KB; callers are expected to stay ≤ 100 frames/s.

## Further reading

- [Parent module (`security`)](../README.md)
- [Security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md)
- [Privacy and security](../../../../../docs/gitbooks/en/features/privacy-and-security.md)
- [Approval gate](../../../../../docs/gitbooks/en/features/approval-gate.md)
- [OS keyring and secret storage](../../../../../docs/gitbooks/en/features/os-keyring-and-secret-storage.md)
