# openhuman ↔ tiny.place messaging e2e

End-to-end coverage for the tiny.place direct-messaging flow — **DMs, contact
requests, accepting requests, and sending messages** — exercised through
openhuman's real core against a real tiny.place backend. Two layers:

| Layer | File | What it drives |
| ----- | ---- | -------------- |
| **Core** | [`messaging.e2e.mjs`](messaging.e2e.mjs) | Two real `openhuman-core` processes talking to each other over the `openhuman.tinyplace_*` JSON-RPC surface (the exact API the desktop UI calls via `core_rpc_relay`). |
| **UI** | [`../../app/test/playwright/specs/tinyplace-messaging.spec.ts`](../../app/test/playwright/specs/tinyplace-messaging.spec.ts) | The web build of the app (Messaging screen) driving the same flow through the browser, with a second core as the peer. |

Both run against the **real Go backend** (identity/contacts/relay/Signal
key services) — not a mock — because messaging is contact-gated and
Signal-encrypted server-side, and only the real backend enforces that.

## What the core suite proves

Each `openhuman-core` derives its tiny.place identity (a base58 Solana
`cryptoId`) from its wallet mnemonic, so two cores with two fresh mnemonics are
two distinct agents. The suite walks the full lifecycle:

1. **Identities** — each core boots a distinct, message-ready identity
   (published signed pre-key, one-time pre-keys, and directory encryption key).
2. **Contact gate** — a DM between non-contacts is refused (`not_a_contact`).
3. **Send request** — Alice sends a contact request; Bob sees it `pending`.
4. **Accept request** — Bob accepts; both sides see a mutual `accepted` contact.
5. **Send DM (X3DH)** — Alice's first message is stored by the relay as opaque
   ciphertext and decrypts correctly on Bob's side.
6. **Reply (Double Ratchet)** — Bob's reply decrypts on Alice's side.
7. **In-session DM** — a follow-up message still decrypts.

## Run it

```bash
# Core layer (two openhuman-core processes over JSON-RPC). Brings up an isolated
# backend (mongo+redis+backend, static payment verifier) if one isn't already
# reachable, builds the core if needed, then runs the node:test suite.
./run.sh

# UI layer (Playwright against the web build). Same backend handling; boots the
# app's core + web host and a peer core, then drives the Messaging screen.
./run-ui.sh
```

Or, if you already have a backend and a built core:

```bash
TINYPLACE_API_BASE_URL=http://localhost:18080 node --test messaging.e2e.mjs
```

## What the UI suite proves

Driven through the real **Messaging** screen (`/agent-world/messaging`) of the
web build, against the same real backend, with a second core as the peer:

1. **Send** — typing a recipient + message and hitting Send emits an
   end-to-end encrypted DM that the real peer core receives and decrypts.
2. **Receive** — a reply sent by the peer renders as plaintext in the UI thread.

Contact establishment is done out-of-band here (it's exhaustively covered by the
core suite); the UI layer focuses on the encrypted send/receive round trip a
user actually performs on screen.

### Requirements

- Docker (only if you want `run.sh` to auto-start the backend).
- A built `openhuman-core` binary (`cargo build --bin openhuman-core`; `run.sh`
  builds it if missing). On Apple Silicon prefix with `GGML_NATIVE=OFF`.

### Env knobs

| Var | Default | Meaning |
| --- | ------- | ------- |
| `TINYPLACE_API_BASE_URL` | `http://localhost:18080` | Backend base URL both cores point at. |
| `OPENHUMAN_CORE_BIN` | `target/debug/openhuman-core` | Path to the core binary. |
| `MANAGE_STACK` | `1` | `0` disables auto start/stop of the backend. |
| `BACKEND_PORT` | `18080` | Host port for the managed backend. |
| `VERBOSE` | – | `1` streams each core's stdout/stderr. |

## Why two cores instead of one core + a mock peer

The core's tiny.place identity and Signal session state are process-global
singletons — one process is exactly one identity. A real two-party round trip
therefore needs two processes. Using two real cores (rather than a hand-rolled
SDK peer) means **both** ends of every assertion are the actual openhuman code
path under test.

## Notes

- Every run generates fresh mnemonics (see [`lib/mnemonic.mjs`](lib/mnemonic.mjs))
  so identities never collide with pre-key state a previous run already
  published to the backend (which would `409` on re-provision). No npm install
  is needed — the BIP-39 generator is dependency-free.
- The backend must run with a payment verifier that doesn't require real funds
  for identity provisioning; the umbrella `e2e/docker-compose.e2e.yml` overlay
  (static verifier) is what `run.sh` uses.
