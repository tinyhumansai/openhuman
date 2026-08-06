# Brief: Tauri live-app benchmark and profiling

Audience: the next agent/session taking on desktop-app (live Tauri + CEF)
profiling. The Rust-core library side is done and documented in
[`library-benchmarking.md`](library-benchmarking.md); this brief covers what
to build for the shipped desktop app, what already exists, and what to reuse.

## Goal

Bring the desktop app to the same standard the core now has: named,
repeatable scenarios; fresh-process repeats with median aggregation; JSON
results; pass/fail budget gates; and an escalation path for attribution. The
app is a process *family* (Tauri host + CEF helpers + utility/GPU/renderer
processes + node/python runtime children), so every measurement must be
family-wide, not single-pid.

## What already exists (reuse, do not rebuild)

- **`app/src-tauri/profiling/`** — an offline Tauri process-family profiler
  (own small crate: `src/main.rs` ~813 lines, README) added in commit
  `ce6cd2291`. Start here; extend it rather than writing a new sampler.
- **`src/openhuman/platform/proc_metrics/`** — RSS/peak/threads/CPU-ms/fds sampling
  plus `tree.rs` (descendant process-tree walker, macOS `proc_listchildpids`
  + Linux `/proc` ppid walk). The tree sampler is exactly what family-wide
  measurement needs.
- **`scripts/profile/`** — the driver-script pattern (build → N fresh runs →
  jq medians → summary.md + gate exit code). Copy `library-bench.sh`'s shape
  for an `app-bench.sh`.
- **JSON schema v2** (`library-profile` output) — reuse the field names
  (`baseline`/`settled`/`peak_rss_kib`/`checkpoints[]`/`budget`) so existing
  aggregation and future CI tooling work on both suites.
- **Env-var conventions** — `OPENHUMAN_PROFILE_*` knobs, `HOLD_SECS`-style
  inspection points.

## Prior findings to build on (2026-07-21 session)

| Finding | Number |
| --- | ---: |
| Full desktop process family | 1,207-1,440 MiB |
| CEF prewarm cost | ~86 MiB (disable/short-lived candidate) |
| spaCy cost | ~146 MiB (lazy-init candidate) |
| Rust core share of the family | ~40-50 MiB |

The gap between the ~50 MiB core and the ~1.2-1.4 GiB family is the entire
story: CEF/renderer processes, prewarm policy, spaCy, and shell-side polling.

## Suggested scenarios

1. **cold-boot** — launch to interactive UI; family RSS + wall time,
   checkpointed (host start, core ready, CEF first frame, UI route mounted).
2. **idle-drift** — 10-30 min idle; family RSS + CPU sampled continuously.
   This is where scanner polling, heartbeat, accessibility probes, and
   `osascript` probes show up (keep them event-driven per prior findings).
3. **chat-turn-e2e** — one full chat turn through the real UI (drive via CDP
   on the CEF debug port or the Appium harness in `openhuman/e2e/`); compare
   against the core-only `agent-turn` baseline to attribute shell overhead.
4. **webview-cycle** — open/close provider webviews (N cycles); CEF child
   process lifecycle, leak check on repeat.
5. **prewarm-matrix** — CEF prewarm on/off × spaCy on/off, reproducing and
   pinning the prior session's ~86/~146 MiB findings as a regression gate.
6. **overlay-surfaces** — mascot/notch/companion windows up vs down.

## Method notes

- Family enumeration: union of the Tauri host's descendant tree (use
  `proc_metrics::tree`) plus CEF helper processes, which may re-parent —
  match by bundle path/name as `app/src-tauri/profiling` already does.
- Sum-RSS double-counts shared CEF framework pages across helpers; on macOS
  record `footprint` output alongside ps-style sums (the instances driver
  in `scripts/profile/library-instances.sh` shows the pattern and caveat
  wording); on Linux use PSS.
- Drive the UI mechanically, not by hand: CDP against CEF (`:19222` per the
  e2e harness) or the WDIO/Appium specs. Every scenario must run
  unattended.
- Gate suggestion: family budget per scenario (e.g. cold-boot ≤ X MiB,
  idle-drift slope ≈ 0), same PASS/FAIL summary style as
  `library-fleet.sh`.

## Definition of done

- `scripts/profile/app-bench.sh` (or equivalent) running the scenarios
  above unattended with median aggregation and gates.
- Baselines recorded in a doc table (like `library-benchmarking.md`).
- The prewarm/spaCy findings converted from one-off observations into
  standing regression gates.
