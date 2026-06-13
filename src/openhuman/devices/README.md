# devices

Mobile-device pairing domain. Brokers a secure, end-to-end-encrypted tunnel between the Rust core and iOS clients over the tinyhumans backend's `tunnel:*` Socket.IO relay. It registers a pairing channel, generates an X25519 keypair, performs key agreement when the device connects, and persists the resulting paired device. Frame confidentiality/integrity is XChaCha20-Poly1305 over an X25519-derived shared secret. This is the Rust counterpart to the iOS `TunnelTransport` strategy described in the repo's iOS-client notes.

## Responsibilities

- Register a new pairing channel with the backend tunnel (`tunnel:register`) and return QR-bound fields (`channel_id`, `pairing_token`, `core_pubkey`, optional `rpc_url`, `expires_at`).
- Generate an X25519 static keypair per pairing; encrypt and persist the private half (via `SecretStore`) so reconnect handshakes survive restart.
- Connect as `role:"core"` on the channel (`tunnel:connect`) and listen for the device.
- On the device's first `tunnel:frame`, complete the X25519 handshake (sealed-handshake or plaintext-pubkey fallback), derive the shared secret, and persist a `PairedDevice`.
- Track live peer-online status from `tunnel:peer-status` and overlay it onto `devices_list` results.
- List non-revoked devices; revoke a device (soft delete) and tear down its in-memory + tunnel state.
- Provide a reusable `TunnelCipher` (seal/open with replay-protection window) for tunnel frame crypto.
- Detect a best-effort LAN `rpc_url` for the direct-HTTP fast path.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/devices/mod.rs` | Export-only: module docstring, `pub mod` decls, re-exports of schema registry fns and public types. |
| `src/openhuman/devices/types.rs` | Serde domain types: `PairedDevice`, `PairingSession`, and the three RPC response payloads. |
| `src/openhuman/devices/rpc.rs` | RPC handler logic for the three methods + module-level in-memory state singletons (`PENDING_KEYPAIRS`, `PERSISTED_KEYPAIRS`, `PENDING_SESSIONS`, `PEER_STATUS`), LAN-URL detection, and `SecretStore`-backed keypair persist/restore. |
| `src/openhuman/devices/schemas.rs` | Controller schemas, `all_controller_schemas`/`all_registered_controllers`, and `handle_*` bridges delegating to `rpc.rs`. Mirrors `cron/schemas.rs`. |
| `src/openhuman/devices/store.rs` | SQLite persistence (`paired_devices` table) via the per-call `with_connection` pattern. |
| `src/openhuman/devices/crypto.rs` | `DeviceKeypair` (X25519 keygen, DH, byte round-trip), `TunnelCipher` (XChaCha20-Poly1305 seal/open with a `WINDOW_SIZE`=128 replay window), and base64url helpers. |
| `src/openhuman/devices/tunnel_client.rs` | Emits/parses `tunnel:*` events over the shared `SocketManager`; wire types; the one-shot ack registry (`PENDING_REGISTER`) that resolves `tunnel:register`. Frame cap 64 KB. |
| `src/openhuman/devices/bus.rs` | `DeviceTunnelSubscriber` event handler — drives handshake completion, persistence, peer-status updates, and register-ack resolution. |

## Public surface

Re-exported from `mod.rs`:

- `all_devices_controller_schemas` / `all_devices_registered_controllers` (alias for `schemas::all_controller_schemas` / `all_registered_controllers`).
- Types: `CreatePairingResponse`, `ListDevicesResponse`, `PairedDevice`, `PairingSession`, `RevokeDeviceResponse`.

Other notable public items (used across the crate but not re-exported at the domain root): `bus::register_device_tunnel_subscriber`, `crypto::{DeviceKeypair, TunnelCipher, base64url_encode, base64url_decode}`, `tunnel_client::{emit_register, emit_connect, emit_frame, resolve_register_ack, TunnelPeerStatus, TunnelFrame, TunnelRegisterResponse}`.

## RPC / controllers

Namespace `devices` (invoked as `openhuman.devices_<function>`):

| Method | Inputs | Output | Behavior |
| --- | --- | --- | --- |
| `devices_create_pairing` | `label?: string` | `CreatePairingResponse` | Registers a channel, generates+persists keypair, emits `tunnel:connect`, returns QR fields. Pairing token expires in 10 min (backend enforces real TTL). |
| `devices_list` | — | `ListDevicesResponse` | Lists non-revoked devices, overlaying live `peer_online` from `PEER_STATUS`. |
| `devices_revoke` | `channel_id: string` | `RevokeDeviceResponse` | Soft-deletes the device, clears all in-memory state for the channel, publishes `DeviceRevoked`. |

Wired into the controller registry in `src/core/all.rs` (schemas + registered controllers + the `"devices"` namespace branch).

## Agent tools

None. This domain has no `tools.rs` and owns no agent tools.

## Events

Subscriber registered at startup from `src/core/jsonrpc.rs` via `register_device_tunnel_subscriber`. `DeviceTunnelSubscriber` (`name() = "device::tunnel"`, `domains() = ["device"]`) **handles**:

- `DevicePeerOnline` / `DevicePeerOffline` → update `PEER_STATUS`.
- `DeviceTunnelFrame` → complete handshake + persist `PairedDevice`.
- `DeviceTunnelRegistered` → resolve the pending `tunnel:register` ack in `tunnel_client`.

**Publishes**:

- `DevicePaired` (after successful handshake + persistence).
- `DeviceRevoked` (from `devices_revoke`).

Note: the `DevicePeerOnline/Offline`, `DeviceTunnelFrame`, and `DeviceTunnelRegistered` events are *originated* by `src/openhuman/socket/event_handlers.rs` (which parses the raw `tunnel:peer-status` / `tunnel:frame` / `tunnel:registered` / `tunnel:evicted` Socket.IO events and re-publishes them as `DomainEvent`s). This domain consumes them; it does not re-publish peer-status itself.

## Persistence

SQLite DB at `{workspace_dir}/devices/devices.db`, table `paired_devices`:

| Column | Notes |
| --- | --- |
| `channel_id` | PK; 128-bit base32 channel id. |
| `label` | Human-readable label. |
| `device_pubkey` | Base64url X25519 device public key. |
| `core_session_token_hash` | SHA-256 of the core session token. |
| `shared_secret_encrypted` | BLOB, currently always written `NULL`. |
| `created_at` / `last_seen_at` | ISO 8601; `last_seen_at` set by `touch_device`. |
| `revoked` | Soft-delete flag; `list_devices` filters `revoked = 0`. |

DDL is created idempotently on every connection open (`with_connection`). `peer_online` is **not** persisted — it lives only in the in-memory `PEER_STATUS` map.

Separately, encrypted X25519 private keys are persisted as `enc2:` strings (via `keyring::SecretStore`, ChaCha20-Poly1305) keyed by `channel_id` in the in-memory `PERSISTED_KEYPAIRS` map, allowing keypair reconstruction (`load_keypair_from_store`) for reconnect handshakes.

## Dependencies

- `crate::openhuman::config` (`Config`, `config::rpc::load_config_with_timeout`) — workspace paths and config loading for handlers.
- `crate::openhuman::keyring::SecretStore` — encrypt/decrypt the X25519 private key at rest.
- `crate::openhuman::socket::global_socket_manager` — reuse the shared backend Socket.IO connection to emit `tunnel:*` events (no second WebSocket).
- `crate::core::event_bus` (`publish_global`, `DomainEvent`, `EventHandler`, `SubscriptionHandle`, `subscribe_global`) — pub/sub for device tunnel events.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller registry contract.
- `crate::rpc::RpcOutcome` — RPC handler return type.
- External crates: `rusqlite`, `chacha20poly1305`, `x25519-dalek`, `base64`, `sha2`, `chrono`, `once_cell`, `tokio`, `async_trait`, `anyhow`.

## Used by

- `src/core/all.rs` — registers the `devices` controllers/schemas and namespace branch.
- `src/core/jsonrpc.rs` — calls `register_device_tunnel_subscriber()` at startup.
- `src/openhuman/socket/event_handlers.rs` — parses raw `tunnel:*` Socket.IO events into `DomainEvent`s that this domain consumes, using this domain's `tunnel_client` wire types (`TunnelPeerStatus`, `TunnelFrame`).

## Notes / gotchas

- **Handshake frame format** (`bus::handle_tunnel_frame`): version `0x01` = sealed-handshake (`eph_pub(32) || nonce(24) || ciphertext+tag`; device seals its static pubkey under an ephemeral DH); a non-`0x01`/`0x02` leading byte falls back to treating the whole payload as a plaintext base64url device pubkey (pre-Layer-2 compat). The sealed-handshake decrypt path does **not** reuse `TunnelCipher::open`; it calls `XChaCha20Poly1305` directly on `nonce||ct` after stripping the `eph_pub` prefix.
- **`TunnelCipher` frame format** (`crypto`): `version(1)=0x01 || nonce(24) || ciphertext+tag`, random nonce per frame, replay protection via a 128-entry sliding window of seen nonces. Wrap in a `Mutex`/`RwLock` at the call site (it is `&mut self` on `open`).
- The `label` persisted on pairing currently falls back to the `channel_id` (the pending session stores no real label field; `PairingSession.channel_id` is used as the label source).
- `devices_revoke` only tears down local + in-memory state. There is **no backend revoke endpoint yet** (TODO referencing PR #709 follow-up); the backend channel is left to expire via the pairing-token TTL.
- `rpc_url` LAN detection uses the UDP "connect to 8.8.8.8" trick to read the local IPv4; port comes from `OPENHUMAN_CORE_RPC_PORT` env (default `7788`). Non-fatal if it fails.
- `tunnel:register` has no native Socket.IO ack support, so `tunnel_client` uses a global one-shot (`PENDING_REGISTER`) resolved by the bus on `tunnel:registered`, with a 10-second timeout.
- `PairingSession` and the keypair maps are in-memory only (TTL/cleanup deferred to backend semantics); they are cleared on revoke.
- Outbound `tunnel:frame` payloads are capped at 64 KB; callers are expected to stay ≤ 100 frames/s.
