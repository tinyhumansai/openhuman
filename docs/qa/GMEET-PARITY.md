# Google Meet Webview Parity — QA Matrix

> Issue: [#1022](https://github.com/tinyhumansai/openhuman/issues/1022)
> Related: [#1035](https://github.com/tinyhumansai/openhuman/issues/1035) — Meet landing page + post-2FA auth refresh
> Branch: `feat/1022-gmeet-parity-audit`
> Tester: oxoxDev
> Build: `upstream/main @ 680589d8` + `feat/1022-gmeet-parity-audit` HEAD
> Date: 2026-04-30
> Method: manual smoke against `pnpm dev:app` on macOS (per `feedback_validation_test_target.md`)

## Verdict legend

- ✅ **pass** — feature works as native app
- ⚠️ **partial** — works but with limitation; needs follow-up
- ❌ **fail** — broken; child issue filed
- 🔍 **needs investigation** — non-deterministic behavior; revisit
- ⏭️ **deferred** — out of scope for this PR; tracked elsewhere

## Acceptance criteria audit

| # | Criterion | Verdict | Evidence | Notes / child issue |
|---|-----------|---------|----------|---------------------|
| 1 | **Auth** — Google sign-in completes (email → password → 2FA); session persists across restarts | ❌ blocked on #1046 | After password entry the in-app webview was redirected to system Chrome (the post-password `SetSID` hop on `accounts.google.<cctld>` falls through `provider_allowed_hosts` for gmeet because the country-localized hosts aren't suffix-matched against bare `google.com`). Identical to the gmail bug fixed in #1020. | **Fix lives in PR #1046** (`is_google_sso_host` predicate at `webview_accounts/mod.rs`). Once #1046 merges and this branch rebases on main, in-app SSO completes natively and CEF lands the cookie in the right jar — no extra change needed in this PR. |
| 2 | **Landing page** — `webview_account_open("google-meet")` lands on `meet.google.com`, not Google's marketing surface | ✅ pass (after fix) | Baseline behavior: post-account-pick redirect committed to `workspace.google.com/products/meet/...` because the bare `google.com` suffix in `provider_allowed_hosts` matched it as in-app. Fix: `on_navigation` intercepts gmeet `workspace.google.com` (and any subdomain) and rewrites the parent to `https://meet.google.com/`. | **Bug fixed in this PR**: `on_navigation` workspace marketing rewrite. |
| 3 | **Start a new meeting** — "Start an instant meeting" / "New meeting" launches the room in-app | ✅ pass (after fix) | Baseline behavior: clicking "Start now" called `window.open(meet.google.com/<roomid>)`; default popup chain routed it to system Chrome and the user lost the in-app session entirely. Fix: `popup_should_navigate_parent` extended to match `meet.google.com` hosts for the `google-meet` provider so the popup is denied and the embedded parent is navigated into the room. Smoke result: "Start now" instantly drops the user into the meeting tab. | **Bug fixed in this PR**: gmeet popup parent-nav. |
| 4 | **Join via meeting code** — entering a code in the input lands the user in the pre-call greenroom | ✅ pass | After perm-grant fix the join-by-code form correctly takes the user into the pre-call screen. | — |
| 5 | **Pre-call cam/mic preview (greenroom)** — camera tile shows live video, mic level meter responds | ✅ pass (after fix) | Baseline behavior: greenroom showed Meet's "Use microphone and camera" consent dialog; clicking "Use microphone and camera" was a no-op (`getUserMedia` returned `NotAllowedError`). Fix: per-origin `Browser.grantPermissions` for `meet.google.com` now grants `audioCapture` + `videoCapture` so `getUserMedia` succeeds without the consent dialog. | **Bug fixed in this PR**: cam/mic perm grant. |
| 6 | **In-call cam/mic** — toggling cam/mic during call works; tiles show live state | ✅ pass | Confirmed in active call. | — |
| 7 | **Screenshare send** — "Present now" picks a window/screen and broadcasts | ✅ pass | `displayCapture` granted via `Browser.grantPermissions`; the macOS screen-recording prompt resolves once and the share starts. | — |
| 8 | **Screenshare receive** — remote participant's share renders | ✅ pass | Verified live with a colleague's share. | — |
| 9 | **Captions** — "Turn on captions" renders the live transcript over the call | ✅ pass | DOM-rendered captions appear inline on the call surface. | The captions are observable to recipe.js (the existing `meet_captions` event), but not yet ingested into OpenHuman memory — that gap is row 13. |
| 10 | **In-call chat** — open chat panel; send + receive in-meeting messages | ✅ pass | DOM-driven chat works as in stock Chrome. | — |
| 11 | **Reactions / raise hand** — emoji reactions and raise-hand toggles | ✅ pass | Confirmed live. | — |
| 12 | **Session persistence** — close + reopen webview tab without sign-out; account stays active | ✅ pass | CEF profile persistence behaves identically to gmail/slack. | — |
| 13 | **Captions ingestion → memory** — `meet_captions` rows + `meet_call_ended` lifecycle event flush a transcript document via `openhuman.memory_doc_ingest` | ⏭️ deferred to #1052 | Current handler at `webview_accounts/mod.rs:2219-2247` is log-only. Not a regression — recipe.js has been log-only since inception. Filed as **feature** at #1052 with module sketch (`google_meet/` mirroring `gmail/` shape, persistent `MeetingTranscriptStore`, opt-in toggle for the privacy-sensitive transcript). | Out of scope for this PR. |
| 14 | **Hangup return UX** — leaving the call returns to a recognizable state (rejoin / 5-star review prompt) | ✅ pass | Behavior is identical to stock Chrome — rejoin button + 5-star review with 60s auto-dismiss. | — |
| 15 | **Background effects** — virtual background / blur applies without breaking the camera | ⚠️ partial — static effects work, dynamic (video) effects fail | **Phase A diagnostic 2026-05-05** ruled out the original hypothesis. Probe results inside an active Meet call: `crossOriginIsolated:false` (irrelevant — SAB is exposed via the existing `--enable-features=SharedArrayBuffer` flag), `hasSAB:true`, `hasInsertableStreams:true` (MediaStreamTrackProcessor/Generator both present), `hasWebGL2:true`, `hasWebGPU:true`, `hasAtomics:true`. **Static backgrounds (blur, classroom, library, etc.) WORK** — verified live. **Dynamic (video) backgrounds fail** with `MEDIA_ERR_SRC_NOT_SUPPORTED: PipelineStatus::DEMUXER_ERROR_NO_SUPPORTED_STREAMS: FFmpegDemuxer: no supported streams` when fetching `https://www.gstatic.com/video_effects/assets/<bg>.mp4`. Real root cause: vendored CEF ships the standard distribution (no proprietary codecs), so H.264-in-MP4 dynamic-bg assets can't be demuxed. Not a Chromium runtime gap. Memory `feedback_cef_runtime_gaps.md` gap #3 reclassified accordingly. | Build infra landed in [`scripts/cef-with-codecs/`](../../scripts/cef-with-codecs/README.md) via #1251 (Chrome-branded FFmpeg build via `automate-git.py` + local install helper + license posture memo). Codec absence persists in the shipped binary until the build is run, the resulting CEF archive is hosted, and `tauri-cef` is re-pinned to fetch from that host. Remaining blockers are operational, not code: (a) license clearance per [`scripts/cef-with-codecs/README.md`](../../scripts/cef-with-codecs/README.md#license-posture-read-before-building), (b) binary hosting + checksum manifest. Flip row to ✅ once the codec-built CEF lands in a vendored `tauri-cef` pin and the smoke harness confirms `MediaSource.isTypeSupported('video/mp4; codecs="avc1.42E01E"') === true`. |

## Smoke run procedure

For each criterion:

1. Reproduce in running app (`pnpm dev:app` from this worktree).
2. Capture exact symptom + log line if relevant.
3. Mark verdict in table.
4. If ❌: file child issue against `tinyhumansai/openhuman` titled `[Bug] webview/gmeet: <symptom>` linking back to #1022.
5. If ⚠️: note limitation + scope follow-up; decide whether to fix in this PR or defer.

## Pre-audit code-level gaps confirmed during smoke

These were diagnosed against `main` before this PR's fixes landed; the smoke run confirmed each manifests as a user-visible bug.

1. **Cam/mic permission gap** — `Browser.grantPermissions` for the gmeet CDP session granted only `notifications` (added by #1028). `getUserMedia` and `getDisplayMedia` had no underlying CEF permission and Gmeet's web client fell back to its own consent dialog, which the embedded view couldn't satisfy. Fixed by per-origin grant of `audioCapture` / `videoCapture` / `displayCapture` / `clipboardReadWrite` for `meet.google.com`.
2. **Workspace marketing redirect** — `provider_allowed_hosts("google-meet")` listed bare `google.com`; the SSR-redirect to `workspace.google.com/products/meet/...` matched as in-app and committed. Fixed by `on_navigation` intercept that rewrites the parent to `meet.google.com/`.
3. **"Start now" popup leak** — `window.open(meet.google.com/<roomid>)` had no in-app handler for the gmeet provider in `popup_should_navigate_parent` / `popup_should_stay_in_app`. Fixed by extending `popup_should_navigate_parent` to match `meet.google.com` hosts for `google-meet`.
4. **Auth post-2FA redirect** — same root cause as #1020 row #1 (Google SSO ccTLD coverage). Fix lives in PR #1046 (`is_google_sso_host`); this branch rebases on main once #1046 merges.

## Hardcoded constants worth capturing

| Constant | Value | File:line |
|----------|-------|-----------|
| Provider URL | `https://meet.google.com/` | `webview_accounts/mod.rs:58` |
| Allow-list suffixes (gmeet) | `google.com`, `googleusercontent.com`, `gstatic.com`, `googleapis.com` | `webview_accounts/mod.rs:108-113` |
| Recipe.js bundle (legacy) | `app/src-tauri/recipes/google-meet/recipe.js` | bundled via `include_str!` at `mod.rs:46` |
| URL fragment account marker | `#openhuman-account-<id>` | `cdp/session.rs:46-48` |
| Granted permissions (gmeet) | `notifications`, `audioCapture`, `videoCapture`, `displayCapture`, `clipboardReadWrite` | `cdp/session.rs` |

## CDP-migration sketch (out of scope)

The legacy `recipe.js` polls the call DOM for caption rows. A future epic should migrate this to a Rust-side `google_meet_scanner` module driven by `DOMSnapshot.captureSnapshot` / `Network.responseReceived`, mirroring the `slack_scanner` / `whatsapp_scanner` shape. Captions are DOM-rendered (not network-delivered), so the migration cannot use `Network.responseReceived` alone — needs a polling DOM snapshot. This work would also enable native dedup and let us drop the recipe.js bundle. **Filed under follow-up — not gated on this PR.**

## Sign-off

- Tester: oxoxDev
- Result: 12 ✅ / 1 ⚠️ partial (#1223 row 15 — CEF codec gap; build infra shipped in #1251, awaits license + binary hosting) / 1 ❌ blocked on #1046 (row 1) / 1 ⏭️ deferred to #1052 (row 13)
- Date: 2026-04-30 (row 15 reclassified 2026-05-05 after Phase A diagnostic; row 15 status updated 2026-05-06 after #1251 codec-build infra merged)
- Action items:
  - Land #1046 (gmail PR) → rebase this branch → row #1 Auth flips ✅
  - Track #1052 (caption ingest feature) as follow-up issue
  - Track #1223 (codec-built CEF: license clearance + binary hosting) — code-side build infra already in `scripts/cef-with-codecs/` per #1251; row 15 flips ✅ once vendored `tauri-cef` is re-pinned to a hosted Chrome-branded archive
