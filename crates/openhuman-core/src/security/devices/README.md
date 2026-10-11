# devices

Mobile-device pairing domain. Brokers a secure, end-to-end-encrypted tunnel between the Rust core and iOS clients over the tinyhumans backend's `tunnel:*` Socket.IO relay. It registers a pairing channel, generates an X25519 keypair, performs key agreement when the device connects, and persists the resulting paired device. Frame confidentiality and integrity use XChaCha20-Poly1305 with HKDF-SHA256-derived directional session keys from static and ephemeral X25519 exchanges. This is the Rust counterpart to the iOS `TunnelTransport` strategy described in the repo's iOS-client notes.

## Responsibilities

- Register a new pairing channel with the backend tunnel (`tunnel:register`) and return QR-bound fields (`channel_id`, `pairing_token`, `core_pubkey`, optional `rpc_url`, `expires_at`).
- Generate an X25519 static keypair per pairing; encrypt and persist the private half (via `SecretStore`) so reconnect handshakes survive restart.
- Connect as `role:"core"` on the channel (`tunnel:connect`) and listen for the device.
- On the device's first `tunnel:frame`, require the version `0x03` sealed handshake, derive session keys, and persist a `PairedDevice`.
- Track live peer-online status from `tunnel:peer-status` and overlay it onto `devices_list` results.
- List non-revoked devices; revoke a device (soft delete) and tear down its in-memory + tunnel state.
- Provide a reusable `TunnelCipher` (seal/open with replay-protection window) for tunnel frame crypto.
- Detect a best-effort LAN `rpc_url` for the direct-HTTP fast path.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/security/devices/mod.rs` | Export-only: module docstring, `pub mod` decls, re-exports of schema registry fns and public types. |
| `crates/openhuman-core/src/security/devices/types.rs` | Serde domain types: `PairedDevice`, `PairingSession`, and the three RPC response payloads. |
| `crates/openhuman-core/src/security/devices/rpc.rs` | RPC handler logic for the three methods + module-level in-memory state singletons (`PENDING_KEYPAIRS`, `PERSISTED_KEYPAIRS`, `PENDING_SESSIONS`, `PEER_STATUS`), LAN-URL detection, and `SecretStore`-backed keypair persist/restore. |
| `crates/openhuman-core/src/security/devices/schemas.rs` | Controller schemas, `all_controller_schemas`/`all_registered_controllers`, and `handle_*` bridges delegating to [`rpc.rs`](./rpc.rs). Mirrors `cron/schemas.rs`. |
| `crates/openhuman-core/src/security/devices/store.rs` | SQLite persistence (`paired_devices` table) via the per-call `with_connection` pattern. |
| `crates/openhuman-core/src/security/devices/crypto.rs` | `DeviceKeypair` (X25519 keygen, DH, byte round-trip), `TunnelCipher` (XChaCha20-Poly1305 seal/open with a `WINDOW_SIZE`=128 replay window), and base64url helpers. |
| `crates/openhuman-core/src/security/devices/tunnel_client.rs` | Emits/parses `tunnel:*` events over the shared `SocketManager`; wire types; `tunnel:register` uses Socket.IO ACK via `SocketManager::emit_with_ack`. Frame cap 64 KB. |
| `crates/openhuman-core/src/security/devices/bus.rs` | `DeviceTunnelSubscriber` event handler: drives handshake completion, persistence, and peer-status updates. |

## Public surface

Re-exported from [`mod.rs`](./mod.rs):

- `all_devices_controller_schemas` / `all_devices_registered_controllers` (alias for `schemas::all_controller_schemas` / `all_registered_controllers`).
- Types: `CreatePairingResponse`, `ListDevicesResponse`, `PairedDevice`, `PairingSession`, `RevokeDeviceResponse`.

Other notable public items (used across the crate but not re-exported at the domain root): `bus::register_device_tunnel_subscriber`, `crypto::{DeviceKeypair, TunnelCipher, base64url_encode, base64url_decode}`, `tunnel_client::{emit_register, emit_connect, emit_frame, TunnelPeerStatus, TunnelFrame, TunnelRegisterResponse}`.

## RPC / controllers

Namespace `devices` (invoked as `openhuman.devices_<function>`):

| Method | Inputs | Output | Behavior |
| --- | --- | --- | --- |
| `devices_create_pairing` | `label?: string` | `CreatePairingResponse` | Registers a channel via Socket.IO ACK, generates+persists keypair, emits tokenless core `tunnel:connect`, returns QR fields using the backend-provided pairing expiry. |
| `devices_list` | none | `ListDevicesResponse` | Lists non-revoked devices, overlaying live `peer_online` from `PEER_STATUS`. |
| `devices_revoke` | `channel_id: string` | `RevokeDeviceResponse` | Soft-deletes the device, clears all in-memory state for the channel, publishes `DeviceRevoked`. |

Wired into the controller registry in `crates/openhuman-core/src/core/all.rs` (schemas + registered controllers + the `"devices"` namespace branch).

## Agent tools

None. This domain has no `tools.rs` and owns no agent tools.

## Events

Subscriber registered at startup from `crates/openhuman-core/src/core/runtime/subscribers.rs` via `register_device_tunnel_subscriber`. `DeviceTunnelSubscriber` (`name() = "device::tunnel"`, `domains() = ["device"]`) **handles**:

- `DevicePeerOnline` / `DevicePeerOffline` → update `PEER_STATUS`.
- `DeviceTunnelFrame` → complete handshake + persist `PairedDevice`.

**Publishes**:

- `DevicePaired` (after successful handshake + persistence).
- `DeviceRevoked` (from `devices_revoke`).

Note: the `DevicePeerOnline/Offline` and `DeviceTunnelFrame` events are *originated* by `crates/openhuman-core/src/platform/socket/event_handlers.rs` (which parses the raw `tunnel:peer-status` / `tunnel:frame` / `tunnel:evicted` Socket.IO events and re-publishes them as `DomainEvent`s). This domain consumes them; it does not re-publish peer-status itself.

## Persistence

SQLite DB at `{workspace_dir}/devices/devices.db`, table `paired_devices`:

| Column | Notes |
| --- | --- |
| `channel_id` | PK; 128-bit base32 channel id. |
| `label` | Human-readable label. |
| `device_pubkey` | Base64url X25519 device public key. |
| `core_session_token_hash` | Legacy column name; currently stores a SHA-256 hash of the pairing credential because the backend no longer mints a core session token. |
| `shared_secret_encrypted` | BLOB, currently always written `NULL`. |
| `created_at` / `last_seen_at` | ISO 8601; `last_seen_at` set by `touch_device`. |
| `revoked` | Soft-delete flag; `list_devices` filters `revoked = 0`. |

DDL is created idempotently on every connection open (`with_connection`). `peer_online` is **not** persisted; it lives only in the in-memory `PEER_STATUS` map.

Separately, encrypted X25519 private keys are persisted as `enc2:` strings (via `keyring::SecretStore`, ChaCha20-Poly1305) keyed by `channel_id` in the in-memory `PERSISTED_KEYPAIRS` map, allowing keypair reconstruction for reconnect handshakes.

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
- `crate::security::secret_storage::SecretStore`: encrypt/decrypt the X25519 private key at rest.
- `crate::platform::socket::global_socket_manager`: reuse the shared backend Socket.IO connection to emit `tunnel:*` events (no second WebSocket).
- `crate::core::bus::BUS` (`.publish`, `.subscribe`) plus `crate::core::events::DomainEvent` and `tinybus::EventHandler`: pub/sub for device tunnel events.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}`: controller registry contract.
- `crate::core::Outcome`: RPC handler return type.
- External crates: `rusqlite`, `chacha20poly1305`, `x25519-dalek`, `base64`, `sha2`, `chrono`, `once_cell`, `tokio`, `async_trait`, `anyhow`.

## Used by

- `crates/openhuman-core/src/core/all.rs`: registers the `devices` controllers/schemas and namespace branch.
- `crates/openhuman-core/src/core/runtime/subscribers.rs`: calls `register_device_tunnel_subscriber()` at startup.
- `crates/openhuman-core/src/platform/socket/event_handlers.rs`: parses raw `tunnel:*` Socket.IO events into `DomainEvent`s that this domain consumes, using this domain's `tunnel_client` wire types (`TunnelPeerStatus`, `TunnelFrame`).

## Notes / gotchas

- **Handshake frame format** (`bus::handle_tunnel_frame`): `0x03 || client_handshake_eph_pub(32) || nonce(24) || ciphertext+tag`. The device seals its static public key and session ephemeral public key under a key derived with HKDF-SHA256 from the handshake ephemeral-to-core-static DH, salted by `client_handshake_eph_pub || core_static_pub`. The 33-byte marker and public-key header is authenticated as associated data. The core replies with `0x04 || nonce(24) || ciphertext+tag`; its key comes from the static DH salted by the client session ephemeral public key, and its associated data is `0x04 || client_session_eph_pub`. The reply carries the server session ephemeral public key. Raw-DH `0x01` handshakes and plaintext public keys are rejected; older clients must upgrade before reconnecting. An upgraded client can reuse its pairing profile and device key while the channel and credential remain valid.
- **`TunnelCipher` frame format** (`crypto`): `0x02 || nonce(24) || ciphertext+tag`. Session keys derive from the static and session ephemeral DH values with separate client-to-server and server-to-client HKDF labels. Each frame has a random nonce; the receiver tracks a 128-entry replay window. Wrap the cipher in a `Mutex`/`RwLock` at the call site (`open` takes `&mut self`).
- The `label` persisted on pairing currently falls back to the `channel_id` (the pending session stores no real label field; `PairingSession.channel_id` is used as the label source).
- `devices_revoke` only tears down local + in-memory state. There is **no backend revoke endpoint yet** (TODO referencing PR #709 follow-up); the backend channel is left to expire via the pairing-token TTL.
- `rpc_url` LAN detection uses the UDP "connect to 8.8.8.8" trick to read the local IPv4; port comes from `OPENHUMAN_CORE_RPC_PORT` env (default `7788`). Non-fatal if it fails.
- `tunnel:register` uses `SocketManager::emit_with_ack` and expects backend ACK shape `{channelId, pairingToken, pairingExpiresAt}` with a 10-second timeout.
- `PairingSession` and the keypair maps are in-memory only (TTL/cleanup deferred to backend semantics); they are cleared on revoke.
- Outbound `tunnel:frame` payloads are capped at 64 KB; callers are expected to stay ≤ 100 frames/s.

## Further reading

- [Parent module (`security`)](../README.md)
- [Security architecture](../../../../../gitbooks/developing/architecture/security.md)
- [Privacy and security](../../../../../gitbooks/features/privacy-and-security.md)
- [Approval gate](../../../../../gitbooks/features/approval-gate.md)
- [OS keyring and secret storage](../../../../../gitbooks/features/os-keyring-and-secret-storage.md)
