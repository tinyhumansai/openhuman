# Cursor observer prototype — empirical findings (spike-tests A & B)

> Date: 2026-07-11
> Companion to `2026-07-10-cursor-windsurf-adapter-spike.md`, which flagged two
> empirical spike-tests (A: what env reaches the MCP subprocess; B: can an
> inbound DM be surfaced into a live turn) as the gates before a firm feasibility
> yes. This doc records what a throwaway prototype actually demonstrated against
> **staging** (`https://staging-api.tiny.place`).

## What was built (and then removed)

A throwaway **Cursor hooks observer** — not the real adapter, just enough to
prove the observe→stream half end-to-end:

- `~/.cursor/hooks.json` wired `beforeSubmitPrompt` (user turn) and
  `afterAgentResponse` (assistant turn) to a small Node script.
- The script normalized each turn into a tinyplace `SessionEnvelopeV1`
  (`harness.provider = "cursor"`, `scope.harness_session_id = conversation_id`)
  and sent it as a Signal-E2E DM to the running OpenHuman identity.
- OpenHuman's orchestration ingest decrypted it, classified it as `cursor`
  (`harness_type_for`, the gate widened in the recognition slice), and rendered
  it under a **Runtime · cursor** session.

All prototype artifacts (the hooks file, the observer scripts, the throwaway
`cursorbridge`/`cursoragent` wallets, the `~/.tinyplace-cursorbridge` store) were
removed after the demo. The shippable output is the recognition slice (PR #4775).

> **Correction (2026-07-13).** A first attempt to "fix" a bundle 404 by minting
> a slash-free tiny.place identity (PR #4779) was **closed as wrong** on review.
> The `/keys/:cryptoId/*` relay routes are keyed on the **base58** cryptoId (no
> `/`) and the backend already resolves both encodings, so the 404 only occurs
> when a *client* puts the base64 `identityKey` in the URL path. Every current
> send path already fetches by base58 (TS SDK ≥2.0.2 `deriveCryptoId`; OpenHuman
> `resolve_recipient_to_agent_id` → `crypto_id`). The observer's 404 was a stale
> `@tinyhumansai/tinyplace@1.0.1` build in the prototype, not a product defect —
> and remapping the identity would have broken the payment/wallet coupling
> (`signer.agent_id()` is the x402 `from` / registry `crypto_id`). Identity must
> stay `== funded Solana wallet`; the real follow-up is bumping the plugin's SDK
> dependency to ≥2.0.2 (see `tiny.place#213`).

## Result: observe → render works ✅

A **real, live Cursor chat** (a Cursor agent window open on `~/work/k8s`,
`conversation_id = 4c17e405…`) streamed both turns into OpenHuman and rendered as
a Cursor runtime session — with no polling, driven purely by Cursor's own hooks
firing on each turn. This confirms the observe path the adapter needs is real:
hooks give us `{prompt}` / `{text}` plus `conversation_id` / `workspace_roots`,
which is enough to build the envelope the ingest already understands.

### Spike-test A (env to MCP subprocess) — partially answered

The observer used **hooks**, not the MCP subprocess, so it did not need the
sanitized-env workaround. But the same run confirmed the constraint the spike
called out: Cursor hands MCP/hook subprocesses a **sanitized environment** (only
`HOME`/`PATH`/`SHELL`/…). The observer therefore had to inject
`TINYPLACE_CURSOR_HOME` / `OPENHUMAN_ADDR` / `TINYPLACE_API_URL` explicitly via a
wrapper `.sh`. **Implication for the real adapter:** its `launch.prepare()` must
bake every env var the MCP server needs into the written `mcp.json` `env` block —
it cannot rely on inheriting the parent shell's environment. (This matches what
the committed `adapters/cursor.mjs` already does with its `env` sentinel.)

## Gaps found (both are the *respond/inbound* half, not observe)

### Gap 1 — reply address is base58↔base64 mismatched

When OpenHuman tries to **reply** into the runtime session, the send fails with
`No agent found for <id>`. This is the same base58/base64 encoding family as the
correction above: the peer must be addressed and resolved by its **base58**
`crypto_id`, not the **base64** `identityKey`. No new crypto is needed — the fix
is base58 discipline on the reply/resolve boundary, and it is likely already
closed once the prototype/plugin is on SDK ≥2.0.2 (whose send path fetches by
`deriveCryptoId`). **Next step: re-test the reply path against a current-SDK
plugin build before assuming any OpenHuman-side change is required.**

### Gap 2 — no Cursor GUI-chat injection API

Even with Gap 1 fixed, an orchestrator reply can be *delivered* to the runtime,
but there is **no supported way to inject text back into Cursor's live GUI chat
window**. The headless `cursor-agent -p` path can *answer* a prompt (that's the
adapter's `responder`), but it is a separate, headless turn — it does not appear
in the human's open Cursor chat. Surfacing an inbound DM into the *live GUI* turn
(spike-test B) remains **unsolved** and is a research item, not a quick fix. The
observe→render→headless-respond loop works; observe→render→*GUI-inject* does not.

### First-contact ratchet desync (SDK-level, recoverable)

On the very first DM between two fresh identities, the Double Ratchet can desync
("No session for <peer>" → the recipient drops the decrypt). Recovery is
`reset_session(<peer>, rehandshake: true)` on the emitter followed by a resend;
after that the session is stable. This is a known SDK first-contact edge, not
specific to the harness work, but adapter onboarding flows should expect it and
either pre-warm the session or retry once on the first send.

## Net feasibility read

- **Observe → normalize → render as a Cursor runtime: proven live.** ✅
- **Headless respond (`cursor-agent -p`): proven separately** in the adapter
  live E2E (whoami, session label `cursor:1`, staging-verified).
- **Orchestrator → live GUI reply: blocked** on Gap 1 (encoding, fixable) and
  Gap 2 (no injection API, open research).

So the recognition slice + the adapter's observe and headless-respond surfaces
are sound; the remaining work is the reply-address encoding fix and the (harder)
question of GUI injection.
