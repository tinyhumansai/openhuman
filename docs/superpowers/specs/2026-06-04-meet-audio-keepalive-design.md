# Meet Bot Audio Keepalive — Design

**Date:** 2026-06-04
**Issue:** [#2945 — Google Meet agent join flow is unreliable](https://github.com/tinyhumansai/openhuman/issues/2945)
**Slice:** Part of slice A (root cause of the ~5 s drop). This is **PR-A** of a two-PR split; **PR-B** (post-admission watchdog) will be filed after PR #3321 (slice B — diagnostics) merges.

## Problem

After the Meet bot is admitted into a call, users report it drops after ~5 seconds and Meet's UI shows "Trying to reconnect…". OpenHuman has no rejoin logic, so the bot never comes back.

### What investigation found

| Finding | File:line |
|---|---|
| The audio path has **no keepalive**. When the brain isn't speaking, `speak_pump` pushes nothing — the WebRTC track stays `readyState: live` but produces zero audio. | `app/src-tauri/src/meet_audio/speak_pump.rs:270–282` |
| The **camera path has an explicit keepalive**, and the comment says exactly why: *"A 1px synthetic bob keeps the WebRTC encoder from dropping the stream as 'frozen' while we're holding a stale frame."* | `app/src-tauri/src/meet_video/camera_bridge.js:179–181` |
| After admission, **nothing watches the bot**. `meet_scanner::run` clicks the captions toggle and exits the spawned task. No reconnect or watchdog. | `app/src-tauri/src/meet_scanner/mod.rs:353–399` |
| The `speak_pump` dies silently after **30 consecutive CDP errors** (~3 s). No event surfaced anywhere. | `app/src-tauri/src/meet_audio/speak_pump.rs:111–117` |
| Production Sentry shows **4 065 events** of `[meet-agent] no session for request_id=…` from a single 107-minute call. Shell-side pumps hammered core long after core's session ended. | Sentry issue `TAURI-RUST-8TM`; error origin `src/openhuman/meet_agent/session.rs:865` |
| No "5-second" timer anywhere in the codebase. The 5 s mark is Meet's choice, not ours. | (grep for `5` / `Duration::from_secs(5)` in meet modules) |

### Hypothesis

Meet's WebRTC media engine drops the bot because its audio track produces no energy. The camera-bridge comment is an explicit, documented precedent for the same failure mode on the video side. The audio side never got the equivalent fix.

This PR is the cheapest probe of that hypothesis: ~20 lines of JS that mirror the camera-bridge pattern. If it works, the bug is fixed. If it doesn't, PR-B (watchdog) will surface the actual drop in production telemetry and we'll escalate to a comfort-noise variant.

## Approach

**JS-side silent `AudioBufferSourceNode` in `audio_bridge.js`.** A permanently-looping source of zero samples connects to the existing `MediaStreamDestinationNode` at install time. The brain's PCM mixes in via the existing `__openhumanFeedPcm` path — zero + real = real, so live speech is unaffected.

Alternatives considered and rejected:
- **A. Rust-side push from `speak_pump`** — Rust generates zero-PCM bytes locally on idle ticks and pushes via CDP. Surgical (~10 Rust lines) but adds continuous CDP traffic for the whole call.
- **C. JS `OscillatorNode → GainNode(0)`** — 5 lines but vulnerable to encoder DTX suppression of true silence; the `AudioBuffer` form is identical on the wire and more transparent to extend if we later need comfort noise.

## Architecture

### Signal chain after the change

```text
Brain PCM (when speaking) ──┐
                            ├──→ MediaStreamDestinationNode ──→ MediaStream
silenceLoop (always)     ───┘                                    │
                                                                 ▼
                                                  freshAudioStream().clone()
                                                                 │
                                                                 ▼
                                                  getUserMedia({audio:true})
                                                                 │
                                                                 ▼
                                                            Meet WebRTC
```

The keepalive source is **not** tracked in `activeSources`, so `__openhumanFlushAudio()` (barge-in) doesn't stop it. It lives for the lifetime of the AudioContext — i.e., the whole call.

### Why this site

`ensureContext()` is called both lazily by `__openhumanFeedPcm` AND eagerly inside `freshAudioStream()` (which Meet's pre-join calls when it hits `getUserMedia`). That means the keepalive starts the same moment Meet first asks for the bot's microphone — strictly before admission, so the audio track is energetic from the first WebRTC packet onward.

No Rust changes. `speak_pump.rs` continues to push only brain PCM; the silence floor is owned entirely by the page-side bridge.

## Implementation

Inside `ensureContext()` in `app/src-tauri/src/meet_audio/audio_bridge.js`, immediately after the existing `dest = ctx.createMediaStreamDestination();` plus its console.log, insert:

```js
    // Keepalive: a permanently-running source of zero samples connected
    // to `dest`. Without this, Meet sees an audio track that is `live`
    // but produces zero PCM during silence — some Meet builds drop the
    // bot for "no audio activity" after a few seconds. Mirrors the
    // camera-bridge 1px-bob keepalive in `meet_video/camera_bridge.js`.
    //
    // Brain PCM mixes in via `__openhumanFeedPcm` over the top —
    // zero + real = real, so this does NOT mute the bot. NOT pushed into
    // `activeSources`: barge-in (`__openhumanFlushAudio`) must not stop
    // the keepalive.
    try {
      var silenceSamples = Math.max(1, Math.floor(ctx.sampleRate / 10)); // 100 ms
      var silenceBuffer = ctx.createBuffer(1, silenceSamples, ctx.sampleRate);
      // createBuffer initializes to zeros — no fill needed.
      var silenceSource = ctx.createBufferSource();
      silenceSource.buffer = silenceBuffer;
      silenceSource.loop = true;
      silenceSource.connect(dest);
      silenceSource.start(0);
      console.log("[openhuman-audio-bridge] keepalive silence source started buffer_samples=" + silenceSamples);
    } catch (e) {
      // Keepalive failure is non-fatal — the bridge still functions for
      // active speech. Log so support can spot it in user reports.
      console.warn("[openhuman-audio-bridge] keepalive setup failed err=" + e);
    }
```

Total new code: ~20 lines including comments.

## Edge Cases

| Case | Behavior |
|---|---|
| Brain speaks | Brain `BufferSource` + silence loop both connect to `dest`; sums to brain PCM (zeros don't subtract). Mascot speaking-state detector in `speak_pump.rs` is unaffected (it gates on `had_pcm` from core, not on bridge activity). |
| Barge-in (`__openhumanFlushAudio`) | Walks `activeSources`; the silence source is not in that list, so it keeps running. |
| AudioContext suspended (Chromium autoplay policy) | The bridge already handles `ctx.resume()` elsewhere on first interaction; silence source produces once resumed. Same behavior as brain PCM. |
| Meet calls `track.stop()` on the destination clone | The clone dies (Meet's track is gone), but `dest.stream` is untouched — next `getUserMedia` returns a fresh clone with both sources still attached. |
| Sample rate ≠ 16 kHz (default Chromium is 48 kHz) | `ctx.sampleRate / 10` adapts to whatever rate Chromium gave us. |
| Setup throws (e.g., AudioContext closed mid-construction) | `try/catch` swallows + warns. Bridge still functions for speech. |

## Risk & Rollback

**If the hypothesis is wrong** — bot still drops at ~5 s. The fix is invisible; we've spent ~20 lines of code to learn DTX is in play. Next step: ship PR-B (watchdog) so the drop is surfaced in production telemetry, then escalate to a comfort-noise variant (-60 dB low-amplitude noise) which bypasses DTX.

**If the hypothesis is right** — bot stays joined. Closes 2 more acceptance criteria of #2945 ("Agent stays joined", "Rejoin state resolves *successfully*"). PR-B (watchdog) still useful to detect drops from OTHER causes.

**Rollback:** revert the single audio_bridge.js commit. Zero state, zero migration, behavior returns to prior immediately. No data loss.

**Risk of making things worse:** very low.
- The keepalive only adds energy; it doesn't change track lifecycle, doesn't intercept getUserMedia differently, doesn't affect the speak_pump, the caption listener, the camera bridge, or any RPC.
- If WebAudio is broken in some Chromium build, the `try/catch` keeps the bridge functional for active speech.
- No new permissions, no new dependencies.

## Testing

### Automated

**Rust static regex test.** A new test in `app/src-tauri/src/meet_audio/audio_bridge_tests.rs` (sibling-test pattern) that loads `audio_bridge.js` as `include_str!` and asserts the keepalive code block is present (looks for the literal substrings `keepalive silence source started` and `silenceSource.start(0)`). Guards against accidental deletion in future refactors. ~10 lines.

Note: this is a **presence test**, not a behavior test. The keepalive's correctness lives in the Chromium WebAudio runtime; even a Vitest test with mocked AudioContext would just verify the mock returns what we expect. The presence regex is honest about what it's testing.

### Manual (recorded in PR body)

1. Run `pnpm dev:app`, open Skills → Meeting Bots, join a real Google Meet call.
2. Confirm the bot stays in the call for ≥60 s without dropping.
3. Open Meet's `chrome://webrtc-internals/`, find the outbound audio track for our bot, confirm `bytesSent` is monotonically increasing during silence (not stuck at zero).
4. Trigger a brain reply (say the wake word + a question); confirm speech still plays correctly — the keepalive doesn't mute live PCM.
5. Trigger barge-in (start a new question mid-reply); confirm in-flight brain audio still cuts.

### E2E

Real Meet failure modes can't be reproduced in CI. PR body explicitly defers, same justification as PR #3321.

### Coverage gate

The diff is ~20 JS lines (uncovered by `cargo-llvm-cov`) + ~10 Rust regex test lines (fully covered by itself). Effect on the 80% diff-coverage gate: positive (small denominator, fully covered).

## Out of Scope (PR-B and beyond)

- **Post-admission watchdog** — poll for the "Leave call" affordance every 5 s; on disappearance, emit a new `dropped_post_admission` ReasonCode via PR #3321's `meet-call:failed` event channel. Requires #3321 to merge first (because the ReasonCode enum lives there). Will be a separate PR with its own brainstorm.
- **Comfort-noise escalation** — if PR-A's pure-zero keepalive doesn't fix the drop (DTX suppression), swap the zero buffer for a -60 dB pseudo-random noise buffer. Trivial follow-up.
- **Stop-session race fix** — the Sentry `TAURI-RUST-8TM` "no session" cascade (4 065 events) is a separate bug where shell-side pumps keep pushing after core stops the session. Different cause, different fix. Filed separately if PR-A doesn't already address it as a side-effect.
- **Real Meet E2E coverage** — needs a mock-Meet harness; no PR can sensibly add it today.

## Acceptance Criteria (this PR)

- `audio_bridge.js` `ensureContext()` installs a permanent looping zero-source connected to `dest`, not tracked in `activeSources`.
- Setup is wrapped in `try/catch` and degrades gracefully.
- Rust static regex test in `app/src-tauri/src/meet_audio/` asserts the keepalive code is present in the shipped JS.
- Manual verification steps recorded in PR body confirm: bot stays joined ≥60 s, `bytesSent` monotonic during silence, brain speech still plays, barge-in still works.
- PR body explicitly defers real E2E with rationale.
- PR body explicitly notes this is PR-A of a two-PR split; PR-B (watchdog) is a follow-up.
