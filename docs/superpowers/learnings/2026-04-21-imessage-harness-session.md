# iMessage live-harness session: learnings

Date: 2026-04-21
Branch: `feature/imessage-live-harness`

## What we set out to do

Extract a pure, testable `run_single_tick` from the iMessage scanner so it
could be exercised against real `chat.db` without a Tauri AppHandle — first
template for five more Apple-native sources.

## What actually broke, and why

### 1. Scanner never started in production (pre-existing)

`ScannerRegistry::ensure_scanner` called `tokio::spawn` from the Tauri
`setup` hook. That hook runs **before** a Tokio reactor is active, so the
call panicked with `"there is no reactor running, must be called from the
context of a Tokio 1.x runtime"` on the main thread — taking the UI with it
while leaving the core sidecar (a separate process) running.

This shipped in the original PR and explains the baseline finding that the
scanner "has never ingested". The prior session concluded the gate was
never connected; the real cause is the scanner task never even spawns.

**Fix:** swap `tokio::spawn` → `tauri::async_runtime::spawn`, and store a
`tauri::async_runtime::JoinHandle` in the registry. The Tauri runtime owns
its own reactor and is active by the time `setup` runs.

**Root-cause generalisation:** the PR's tests were all pure-function (ns
conversions, allowlist match, attributedBody extraction). Nothing
exercised startup. The app was never launched end-to-end before merge.

### 2. Worktree missing untracked artifacts

`git worktree add` checks out the tracked tree only. These did not come
along and the app refused to build / run until they did:

- **Submodule** `app/src-tauri/vendor/tauri-cef` — not initialised in the
  new worktree; `cargo check` failed with "Unable to update … tauri-cef".
- **`.env`** at repo root — `yarn dev:app` sources it; missing = exit 1.
- **`app/src-tauri/binaries/openhuman-core-*`** — the Tauri build script
  demands the staged sidecar exist even for `cargo check`.

**Fix:** one-shot `scripts/worktree-bootstrap.sh` that runs
`git submodule update --init --recursive`, symlinks `.env` from main,
builds+stages the core sidecar, and runs `yarn tauri:ensure`. Run once
after `git worktree add`.

### 3. CLAUDE.md drift

CLAUDE.md said `cargo build --bin openhuman` and `yarn tauri dev`. The
real names are `cargo build --bin openhuman-core` and `yarn dev:app`
(from `app/`, which chains `tauri:ensure`, `core:stage`,
`setup-chromium-safe-storage`, dotenv, then `cargo tauri dev`).

**Fix:** treat `package.json` scripts as source of truth; either strip
command recipes from CLAUDE.md or add a CI check that greps CLAUDE.md
against `package.json` / `Cargo.toml` and flags stale names.

### 4. Tauri CLI not in PATH in a fresh worktree

`yarn tauri dev` fails 127 ("tauri: command not found") unless the
vendored `cargo-tauri` has been installed to `~/.cargo/bin`. `yarn dev:app`
auto-runs `yarn tauri:ensure` which installs it, so the surface symptom
is "the docs said `yarn tauri dev` but only `yarn dev:app` works".

## What we did

- Extracted `TickInput` / `TickOutcome` / `TickDeps` trait into
  `app/src-tauri/src/imessage_scanner/tick.rs`.
- `run_single_tick` — pure: fetch gate → read new rows → rebuild each day
  → delegate ingest to `TickDeps`. No sleep, no AppHandle, no cursor I/O.
- `HttpDeps` prod impl delegates to existing `fetch_imessage_gate` /
  `ingest_group` functions.
- `run_scanner` loop shrunk to ~20 lines: cursor load + repeat
  `run_single_tick` + persist.
- Added `FakeDeps` tests: `skips_when_gate_disconnected` (unit,
  always-on), `run_single_tick_ingests_groups_from_real_chatdb` (ignored,
  real chat.db), `run_single_tick_keeps_cursor_on_group_failure`
  (ignored, forced failure invariant).
- Fixed the `tokio::spawn` panic.

## Verification state

| Level | Status |
|-------|--------|
| `cargo check` clean | ✅ |
| 10 unit tests | ✅ passing (9 existing + 1 new) |
| 4 ignored tests against real chat.db | ✅ passing |
| App launches without panic | ✅ (after panic fix) |
| Scanner task spawns + ticks fire | ✅ logs: `scanner up`, tick loop running |
| Gate connected → memory_doc_ingest rows | ⏳ pending click-through (Layer 3) |

## What still belongs on this branch before merge

- [ ] Commit refactor + panic fix + bootstrap script + this learnings doc.
- [ ] Click-through: connect iMessage in Settings, wait a tick, assert
      `memory_docs` grows (the Layer 3 step that the prior session
      identified).
- [ ] Optional: sidecar integration test that spins a real core on an
      ephemeral port and drives `run_single_tick` against it with real
      `HttpDeps`, to lock the wire path in CI instead of by inspection.

## Recommendations for next features

1. **Debuggability ladder.** Define for any PR touching startup: (1)
   compiles, (2) pure tests pass, (3) app launches without panic, (4)
   feature fires end-to-end. Steps 3 and 4 are different — skip (3) and
   you ship a scanner that crashes silently.
2. **Launch-smoke CI.** Minimum: boot the app headless, tail logs for
   `panic` / `FATAL` for 10 seconds, fail the job if found. Would have
   caught issue #1 at merge time.
3. **Worktree friction is recurring.** Every time someone branches off,
   they'll rediscover the submodule / .env / binaries problem. Script it.
