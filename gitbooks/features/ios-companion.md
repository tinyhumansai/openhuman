---
description: >-
  Pair an iOS companion app to your desktop OpenHuman over an end-to-end
  encrypted tunnel, scanned from a QR code.
icon: smartphone
---

# iOS Companion

The iOS Companion lets you reach your desktop OpenHuman from your phone: you scan a QR code shown on the desktop, the two devices agree on a shared key, and from then on the phone talks to the desktop core over an encrypted channel.

{% hint style="warning" %}
**Experimental / non-shipping.** The iOS client is in-progress and is **not** part of the shipped desktop product. APIs, wire formats, and the pairing flow can change without notice, and an upgrade may force you to re-pair. Treat everything below as a developer preview.
{% endhint %}

The desktop core is always the source of truth. The phone is a thin client — it does not run its own agent, it relays requests to the core and renders the results.

***

## What it is

Pairing is brokered by the Rust `devices` domain in the core. The core registers a pairing channel with the tinyhumans backend's `tunnel:*` Socket.IO relay, generates a fresh X25519 keypair, and renders a QR code. The phone scans it, generates **its own** X25519 keypair, and connects back over the same relay. The backend is a **blind forwarder** — it relays opaque frames and never sees plaintext.

Once paired, the device shows up in **Settings → Devices** on the desktop with an online/offline dot, and can be revoked at any time.

***

## Pairing via QR code

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
     |   device appears in Devices list    |                        |
```

The QR payload (carried as an `openhuman://pair?...` deep link) contains the channel id (`cid`), a single-use pairing token (`pt`), the core's public key (`cpk`), an optional LAN URL (`rpc`), and an expiry (`exp`). The pairing token is single-use, hashed at rest on the backend, and the QR is rejected client-side once `exp` has passed (the backend enforces the real ~10 minute TTL).

***

## The end-to-end tunnel

Confidentiality and integrity live entirely on the two endpoints. The exact primitives, from `src/openhuman/devices/crypto.rs`:

* **Key agreement:** X25519 Diffie-Hellman. Each side has a long-term static keypair (the core's is in the QR; the device's is minted at scan time) plus an ephemeral keypair minted per session for forward secrecy.
* **Session-key derivation:** HKDF-SHA256 over `ikm = static_dh || eph_dh`, salted with `client_eph_pub || server_eph_pub`. Two **directional** 32-byte subkeys are expanded with distinct info tags — `openhuman-tunnel/v1/c2s` and `openhuman-tunnel/v1/s2c` — so a frame one side seals can never decrypt under its own opener (closes the cross-direction reflection attack class).
* **Frame cipher:** XChaCha20-Poly1305 (AEAD, 192-bit nonce). Wire format is `version(0x02) || nonce(24) || ciphertext+tag`, with a random nonce per frame.
* **Replay protection:** a sliding window over the last 128 nonces seen per opener.

Static DH authenticates the peer via the QR-code provenance; ephemeral DH means a later static-key leak cannot decrypt past traffic. The legacy single-key `version=0x01` frame shape is rejected with an explicit "re-pair required" error — peers must re-pair after an upgrade. Outbound frames are capped at 64 KB.

***

## Transport strategies

The phone may reach the core three ways. `TransportManager` (`app/src/services/transport/`) picks one from the saved `ConnectionProfile`; for a paired device it **races LAN against the tunnel** (2 s LAN timeout) and uses whichever answers `openhuman.ping` first.

| Strategy | Class | When it's used | Trade-offs |
| --- | --- | --- | --- |
| **LAN HTTP** (`LanHttpTransport`) | Direct HTTP to the core's LAN `rpc_url` | Phone and desktop on the same network | Fastest, lowest latency. Requires same LAN; not encrypted by this layer (relies on local network trust). |
| **Tunnel** (`TunnelTransport`) | E2E encrypted frames over the backend Socket.IO relay | Anywhere with internet; default fallback | Works across networks; X25519 + XChaCha20-Poly1305 end to end. Higher latency (relayed); depends on backend availability. |
| **Cloud HTTP** (`CloudHttpTransport`) | HTTP to a cloud-hosted core endpoint | Profile `kind: "cloud"`, when LAN and tunnel are unreachable | Reachable from anywhere; depends on a hosted core and its own auth. |

***

## Device management & revocation

Paired devices are persisted by the core in SQLite (`{workspace_dir}/devices/devices.db`, table `paired_devices`): channel id, label, the device's public key, a SHA-256 hash of the core session token, and timestamps. The core's X25519 private key is stored encrypted at rest (via the OS keyring `SecretStore`) so handshakes survive a restart.

* **List** — `devices_list` returns non-revoked devices, overlaying a live `peer_online` flag sourced from `tunnel:peer-status` (online status is never persisted).
* **Revoke** — `devices_revoke` soft-deletes the device, tears down all in-memory and tunnel state for the channel, and publishes a `DeviceRevoked` event. Today revocation is local-side: the backend channel is left to expire via its pairing-token TTL (a backend revoke endpoint is a follow-up).

***

## See also

* [Privacy & Security](privacy-and-security.md) — how OpenHuman handles your data and keys.
* [Voice](native-tools/voice.md) — push-to-talk and dictation, the headline use case for a phone companion.
