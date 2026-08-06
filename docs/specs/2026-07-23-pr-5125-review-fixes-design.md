# PR #5125 Review Fixes Design

## Objective

Address every actionable unresolved review thread on PR #5125 without broadening
the already-large core-slimming change. Preserve the intended removal and
relocation boundaries while restoring behavior that the migration accidentally
dropped.

## Approach

Work in small review clusters and commit each validated cluster separately:

1. Companion lifecycle and transport:
   - Centralize terminal cleanup so manual stop and TTL expiry cancel the active
     turn, stop microphone capture, remove the session, and emit an Idle state.
   - Let cancelled and empty-transcript turns return Listening to Idle without
     clobbering a newer capture.
   - Register the configured global hotkey when the user starts a companion
     session.
   - Resolve companion chat requests against the OpenHuman backend base, never a
     user-configured inference URL carrying different credentials.
2. Persistence concurrency:
   - Clear goal continuation suppression through the tinyagents atomic mutation
     path, retaining the current goal identity and concurrent usage/status
     changes.
   - Serialize every thread-board read/modify/write mutation with the existing
     per-board async lock.
   - Verify that the already-updated boot migration path completes before
     runtime writers become available and does not block a Tokio worker.
3. Product surfaces:
   - Restore the always-on listening switch in the Voice settings panel using
     the existing voice settings RPC.
   - Restore companion data-movement disclosure in the capability catalog.
   - Remove the stale launch-app catalog entry if it remains in current HEAD.
4. Test and E2E alignment:
   - Replace or remove scenarios that target deleted autocomplete routes or the
     removed WhatsApp core RPC. Prefer the supported Tauri command path when the
     behavior still ships.
   - Add focused regression tests for every behavior-changing fix.

Outdated review threads will be checked against current HEAD. If a later commit
already fixed the reported behavior, no duplicate code change will be made; the
thread will be resolved with the validating evidence.

## Error Handling and Safety

- Cleanup is idempotent: repeated stop, expiry, or cancellation calls must not
  panic or affect a newer session or turn.
- Session credentials are sent only to the canonical backend origin.
- Goal and board mutations must not reintroduce stale snapshot writes.
- Removed product surfaces stay removed; fixes restore only behavior that still
  exists in the shipped desktop application.

## Validation

Each cluster receives its narrowest meaningful tests before its atomic commit.
Final validation covers:

- Rust formatting and targeted core tests.
- Targeted Tauri companion tests and Tauri `cargo check`.
- Targeted frontend tests, typecheck, lint, and formatting.
- E2E inventory/script checks for retired scenarios.
- Relevant default and disabled-feature core checks where touched code crosses
  compile-time gates.

After all local checks pass, push the branch and poll PR checks plus unresolved
review threads until required checks are successful and no actionable feedback
remains.
