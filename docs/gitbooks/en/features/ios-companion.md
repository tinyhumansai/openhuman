---
description: >-
  Pair an iOS companion app with your desktop OpenHuman over an end-to-end
  encrypted tunnel, set up by scanning a QR code.
icon: mobile-screen
---

# iOS companion

The iOS companion lets you reach your desktop OpenHuman from your phone. You scan a QR code shown on the desktop, the two devices agree on a shared key, and from then on the phone talks to the desktop core over an encrypted channel.

{% hint style="warning" %}
Experimental and not shipping. The iOS client is in progress and is not part of the shipped desktop product. APIs, wire formats and the pairing flow can change without notice, and an upgrade may force you to pair again. Treat everything below as a developer preview.
{% endhint %}

The desktop core is always the source of truth. The phone is a thin client. It does not run its own agent. It relays requests to the core and shows the results.

The experimental part is the client, not the core. The core's pairing code is complete: it registers the channel, derives the keys, saves the device and tracks whether the peer is online. Two gaps remain. There is no desktop screen for managing paired devices, and revoking a device only takes effect locally.

## How pairing works

The core's `devices` domain handles pairing. The core registers a pairing channel with the backend's `tunnel:*` Socket.IO relay, generates a fresh X25519 keypair and shows a QR code. The phone scans it, generates its own X25519 keypair and connects back over the same relay. The backend only forwards frames. It relays opaque data and never sees plaintext.

Pairing and revocation are core RPCs, not a desktop screen. The old **Settings > Devices** page was removed, and its address now redirects to **Settings > Account**. Nothing in the desktop frontend calls the `devices_*` methods yet, so listing or revoking a paired device means calling the core directly.

## Pairing with a QR code

```text
Desktop core                         Backend relay              iOS app
     |                                     |                        |
     |-- devices_create_pairing RPC        |                        |
     |-- tunnel:register ----------------->|                        |
     |<-- channel_id, expires_at ----------|                        |
     |-- generate X25519 keypair           |                        |
     |-- tunnel:connect (role: core) ----->|                        |
     |                                     |                        |
     |   shows QR:                         |                        |
     |   cid, pt, cpk, rpc?, exp           |                        |
     |.................. scan QR ......................>            |
     |                                     |   generate device      |
     |                                     |   X25519 keypair        |
     |                                     |<-- tunnel:connect ------|
     |                                     |    (role: client)       |
     |<------ tunnel:frame (handshake) ----|------------------------|
     |-- X25519 DH + derive session keys   |                        |
     |-- persist PairedDevice              |                        |
     |-- publish DevicePaired event        |                        |
     |   devices_list now returns it       |                        |
```

The QR code carries an `openhuman://pair?...` deep link with these fields:

- `cid`: the channel id.
- `pt`: a single-use pairing token.
- `cpk`: the core's public key.
- `rpc`: an optional LAN URL.
- `exp`: the expiry.

The phone rejects the QR once `exp` has passed. The core does not choose the expiry. It stores and republishes whatever `pairingExpiresAt` the backend returns when the channel is registered, so the real lifetime is the backend's.

## The end-to-end tunnel

Confidentiality and integrity are handled entirely by the two endpoints. The primitives are:

- **Key agreement:** X25519 Diffie-Hellman. Each side has a long-term static keypair (the core's is in the QR code, and the device's is created at scan time) plus an ephemeral keypair for each session, for forward secrecy.
- **Session keys:** HKDF-SHA256 over the static and ephemeral shared secrets, salted with both ephemeral public keys. It expands two directional 32-byte keys with different labels (`openhuman-tunnel/v1/c2s` and `openhuman-tunnel/v1/s2c`). A frame one side seals can therefore never be opened by the same side, which blocks reflection attacks.
- **Frame cipher:** XChaCha20-Poly1305 with a 192-bit nonce. A frame is `version(0x02) || nonce(24) || ciphertext+tag`, with a random nonce each time.
- **Replay protection:** a sliding window over the last 128 nonces seen by each receiver.

The static exchange authenticates the peer through the QR code. The ephemeral exchange means a later leak of a static key cannot decrypt past traffic. The old single-key `version=0x01` frame is rejected with a "re-pair required" error, so peers must pair again after an upgrade. Outbound frames are capped at 64 KB.

## Transport strategies

The phone can reach the core in three ways. `TransportManager` picks one based on the saved connection profile. For a paired device it races the LAN against the tunnel (with a 2 second LAN timeout) and uses whichever answers `openhuman.ping` first.

| Strategy | What it does | When it is used | Trade-offs |
| --- | --- | --- | --- |
| LAN HTTP (`LanHttpTransport`) | Direct HTTP to the core's LAN `rpc_url` | Phone and desktop on the same network | Fastest. Needs the same LAN, and this layer does not encrypt it, so it relies on trust in the local network. |
| Tunnel (`TunnelTransport`) | Encrypted frames over the backend Socket.IO relay | Anywhere with internet. The default fallback. | Works across networks and is encrypted end to end. Slower because it is relayed, and it depends on the backend being up. |
| Cloud HTTP (`CloudHttpTransport`) | HTTP to a cloud-hosted core endpoint | Profile `kind: "cloud"`, when LAN and tunnel are unreachable | Reachable from anywhere. Needs a hosted core and its own auth. |

## Device management and revocation

The core saves paired devices in SQLite (`{workspace_dir}/devices/devices.db`, table `paired_devices`). Each record has the channel id, a label, the device's public key, a SHA-256 hash of the core session token, and timestamps. The core's X25519 private key is encrypted at rest through the OS keyring, so handshakes survive a restart.

- **Create:** `devices_create_pairing` registers the channel, creates and saves the keypair, and returns the QR fields.
- **List:** `devices_list` returns devices that are not revoked, with a live `peer_online` flag from `tunnel:peer-status`. Online status is never saved.
- **Revoke:** `devices_revoke` soft-deletes the device, clears all in-memory and tunnel state for the channel, and publishes a `DeviceRevoked` event. Revocation is local only. The backend channel is left to expire with its pairing-token lifetime.

All three are available over JSON-RPC as `openhuman.devices_create_pairing`, `openhuman.devices_list` and `openhuman.devices_revoke`. On the CLI, use `openhuman-core devices create_pairing`, `openhuman-core devices list` and `openhuman-core devices revoke --channel_id <id>`.

## See also

- [Privacy and security](privacy-and-security.md): how OpenHuman handles your data and keys.
- [Voice](native-tools/voice.md): use the iOS hold-to-talk control, dictation, or a live voice session from the phone companion.
- [Architecture](../developing/architecture.md): where the iOS client sits relative to the core.
- [OS keyring and secret storage](os-keyring-and-secret-storage.md): where the core's X25519 private key is kept.
