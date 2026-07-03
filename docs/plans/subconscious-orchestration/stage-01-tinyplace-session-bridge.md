# Stage 1 — tiny.place session bridge (SDK/CLI, `tiny.place/` repo)

## Goal command

> In the `tiny.place/` checkout (`sdk/typescript`), define a versioned **`SessionEnvelope` v1**
> message schema for harness sessions and extend the CLI wrappers so that **every semantic message
> (user and agent) from a wrapped Codex or Claude Code instance is sent as a Signal E2E DM to a
> configured tiny.place recipient** (the OpenHuman owner agent), in addition to the existing
> on-disk JSONL envelopes. Add a `tinyplace claude` wrapper mirroring `tinyplace codex`.
> **Configure-once, zero-args**: after one-time setup the owner target and forwarding preferences
> live in `~/.tinyplace/config.json`, so plain `tinyplace codex` / `tinyplace claude` (or a session
> launched from the TUI) forwards automatically — no per-invocation flags. The **TUI is the
> primary surface** for setup, pairing status, and launching sessions; `--tinyplace-*` flags exist
> only as overrides for scripting.

## Read first

- `sdk/typescript/src/cli/codex.ts` — existing wrapper: PTY proxy + session-JSONL tailing already
  produces `SemanticMessage { role: "user" | "assistant" }` and `SessionEnvelope`s written under
  `~/.tinyplace/codex-envelopes/messages/…`. This is the source of truth to forward.
- `sdk/typescript/src/agent/` — `Agent` facade (`sendMessage` does handle-resolution + Signal E2E).
- `sdk/typescript/src/messaging/`, `src/signal/` — DM send path.
- `sdk/typescript/src/cli/context.ts` — persisted CLI config at `~/.tinyplace/config.json`
  (`TINYPLACE_CONFIG` override); this is where the new keys land.
- `sdk/typescript/src/cli/tui.ts` — existing blessed TUI (`runTinyPlaceTui`), already models agent
  kinds `"claude" | "codex"` and session launch; the setup/pairing UX extends this.
- `sdk/typescript/AGENTS.md` — identifier kinds, error-code contract.

## Deliverables

1. **`SessionEnvelope` v1 wire schema** (new `sdk/typescript/src/types/session-envelope.ts`,
   exported from the root). This is the contract stages 3–7 parse; it rides inside the DM body as
   JSON (encrypted end-to-end):

   ```ts
   interface HarnessSessionEnvelope {
     v: 1;
     kind: "session_message" | "session_lifecycle";
     source: "codex" | "claude-code";
     sessionId: string;        // stable wrapper session id
     sessionLabel?: string;    // e.g. repo folder name
     workspace?: string;       // cwd of the wrapped instance
     seq: number;              // monotonic per session
     role: "user" | "assistant" | "system";
     body: string;             // the semantic message text (may be truncated, see limits)
     truncated?: boolean;
     timestamp: string;        // ISO-8601
     lifecycle?: "started" | "ended" | "error";  // kind === session_lifecycle
   }
   ```

2. **Persisted forwarding config + one-time setup**: new `orchestration` block in
   `~/.tinyplace/config.json` — `{ forwardTo: "<@handle|agentId>", forwardEnabled: true,
   pairingStatus?, scope?, bucket? }` — written once by the TUI setup flow (or
   `tinyplace setup --forward-to @owner` non-interactively). Resolution order per run:
   CLI flag (`--tinyplace-forward-to`) → env (`TINYPLACE_FORWARD_TO`) → config. Once configured,
   **bare `tinyplace codex` / `tinyplace claude` forwards with no extra args**; when nothing is
   configured, the wrappers behave exactly as today (local envelopes only, no nagging).
3. **DM forwarding in the wrappers**: each `SemanticMessage` and lifecycle event is wrapped in a
   `HarnessSessionEnvelope` and sent via the Agent facade Signal DM path to the resolved target.
   Batching: flush per message, but coalesce assistant deltas into one message per completed turn
   (the tailer already yields whole messages). Failures are logged to stderr JSON and **never
   crash the wrapped CLI**; retry with backoff on `transient`/`rate_limited`, drop after N retries
   with a lifecycle `error`.
4. **`tinyplace claude` wrapper** (`sdk/typescript/src/cli/claude.ts`): same PTY-proxy shape as
   `codex.ts`, tailing Claude Code session JSONL (`~/.claude/projects/<slug>/*.jsonl`) for
   user/assistant messages; shares the envelope writer + forwarder (extract the reusable parts of
   `codex.ts` into `src/cli/session-bridge.ts` rather than copy-pasting). Both wrappers are
   launchable from the TUI (`tui.ts` already models both agent kinds) and inherit the config.
5. **Size limits**: cap `body` at 8 KiB per DM (set `truncated: true`; full text stays in the local
   JSONL). Never forward raw terminal chunks — semantic messages only.
6. **Contact-pairing handshake in the TUI** (wrapper half of stage 2 — the relay refuses DMs
   between non-contacts, `403 not_a_contact`). The TUI setup flow owns this: it shows the CLI's
   own identity (`agentId` / `@handle`) so the user can link it from OpenHuman, displays live
   pairing status (`none / pending / accepted / blocked`), and persists `pairingStatus` to config
   so subsequent runs skip straight to forwarding. Headless runs perform the same handshake
   silently from config. State machine on `contacts/{owner}/status`:
   - `accepted` → forward; `none` → send a contact request, queue envelopes (bounded, drop-oldest
     with a lifecycle `error` note) and poll until accepted;
   - `pending incoming` **from the exact configured forward-to identity only** → accept;
   - `blocked` → terminal error, no retries.
   At send time, branch on the `not_a_contact` error code → re-enter pairing instead of retrying.
7. Docs: README section + `tinyplace describe` entries for setup, the config keys, and the
   override flags.

## Tasks

1. Extract shared session-bridge module from `codex.ts` (envelope writer, session-id logic, tailer
   interface) — no behavior change; existing tests stay green.
2. Add `HarnessSessionEnvelope` type + serializer/validator (`parseHarnessSessionEnvelope` for
   consumers) with unit tests round-tripping v1 and rejecting unknown `v`.
3. Add the `orchestration` config block to `context.ts` (read/write, flag→env→config resolution)
   + `tinyplace setup` command; config round-trip tests.
4. Implement the forwarder (queue + retry + never-crash guarantee), target resolved from config.
5. Implement `claude.ts` tailer: resolve session file for the wrapped instance (newest JSONL in the
   project slug dir after spawn; honor `--tinyplace-session-file` override like codex does).
6. Implement the pairing handshake (status check, request, owner-only auto-accept, bounded queue,
   `not_a_contact` recovery) in the shared session-bridge module; TUI panels for setup, pairing
   status, and session launch on top of it.
7. Wire both into `cli.ts`/`commands.ts`/`tui.ts`; update help/catalog output.
8. Vitest coverage: envelope schema, config resolution order, forwarder retry/drop behavior (mock
   client), pairing state machine (all four contact statuses + owner-only accept), claude JSONL
   parsing fixtures (user message, assistant message, tool-use records ignored).

## Acceptance criteria

- After one-time setup (TUI or `tinyplace setup --forward-to @owner`), a bare `tinyplace codex`
  forwards every user + assistant message of the session as Signal DMs whose plaintext bodies
  parse as `HarnessSessionEnvelope` v1, plus `started`/`ended` lifecycle envelopes — **no
  per-invocation flags**.
- `tinyplace claude` does the same for a Claude Code session; both launch from the TUI too.
- With no config and no flags, behavior is unchanged from today (no forwarding, no prompts).
- With no contact edge, the wrapper pairs (or waits, queueing) before any envelope DM is sent; it
  never auto-accepts an identity other than the configured forward-to target.
- Killing the network mid-session does not break the wrapped CLI; envelopes on disk stay complete.
- `pnpm test` green in `sdk/typescript`; new module has no `any`-typed public surface.
