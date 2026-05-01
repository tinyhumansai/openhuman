# Slack Webview Parity — QA Matrix

> Issue: [#1016](https://github.com/tinyhumansai/openhuman/issues/1016)
> Branch: `feat/1016-slack-parity-audit`
> Tester: oxoxDev
> Build: main @ `b11b8f33` + `feat/1016-slack-parity-audit` HEAD
> Date: 2026-04-29
> Method: manual smoke against `pnpm dev:app` on macOS (per `feedback_validation_test_target.md`)

## Verdict legend

- ✅ **pass** — feature works as native app
- ⚠️ **partial** — works but with limitation; needs follow-up
- ❌ **fail** — broken; child issue filed
- 🔍 **needs investigation** — non-deterministic behavior; revisit
- ⏭️ **skipped** — could not test (env / dependency missing)

## Acceptance criteria audit

| # | Criterion | Verdict | Evidence | Notes / child issue |
|---|-----------|---------|----------|---------------------|
| 1 | **Auth** — Google SSO, email/password, SAML SSO; session persists across app restarts | ✅ pass (login) / 🔍 (restart) | Vezures workspace loaded, signed-in as Nikhil Bajaj. Restart-persistence untestable in dev:app due to dev-mode restart-loop hack (separate child issue) | Login flow works through in-app webview. SAML not tested (no SAML org). Restart re-auth needs to be validated in a packaged `.app`. |
| 2 | **Messaging** — channels, DMs, group DMs; threads | ✅ pass | User confirmed send-receive works in DMs/channels. Threads navigable. UI parity matches web Slack. | Memory-side extraction blocked by #7 (scanner doesn't run); will re-verify post-fix. |
| 3 | **Reactions & emoji** — picker opens; reactions post correctly | ✅ pass | User confirmed reactions + threads work end-to-end. UI render confirmed in screenshot (`🙏 1`, `👍 1`). | Pre-audit "extraction missing" claim DEFERRED — unverifiable until #7 fix lands and scanner actually runs. Will re-recall memories post-fix; if reactions show in memory docs, the static audit was wrong (like #4 was). Only if missing post-#7-fix do we touch `extract.rs`. |
| 4 | **File sharing** — upload, download; image previews | ✅ pass | User confirmed upload/download/preview all work. | Pre-audit "allowed_hosts mismatch" was a **false alarm**: `slackb.com` IS the working CDN host. Spec's `slack-imgs.com` + `slack-files.com` are stale — DO NOT change `webview_accounts/mod.rs:101`. Drop that fix from the plan. |
| 5 | **Huddles** — popup spawn (about:blank → huddle URL whitelisted); audio/video; popup cleanup on end | ❌ **fail** (paint lifecycle bug; deferred to CEF tracking issue) | `OPENHUMAN_SLACK_HUDDLE_PROBE=1` debug instrumentation captured the popup lifecycle 12:28:29–12:28:37: `windowOpen url=about:blank`, `target_created`, `target_info_changed url=about:blank` (and stays at about:blank for 8s), `target_destroyed`. **No `Page.frameNavigated` ever fires on the popup target** — URL never leaves `about:blank`. Combined with user-observed behaviour ("name + voice + camera tile briefly render then go white"), the working hypothesis (CEF doesn't honor cross-window `popup.location = huddleURL`) was **invalidated**. Real root cause: Slack uses `popup.document.write(html)` to inject huddle UI into the same `about:blank` document — there is no URL navigation to intercept; the bug is CEF's paint/compositor lifecycle on same-document popups. | Tracked in **[#1074](https://github.com/tinyhumansai/openhuman/issues/1074)** (popup paint) → root-cause refined to **[#1079](https://github.com/tinyhumansai/openhuman/issues/1079)** (CEF same-document popup paint lifecycle). The #1074 PR ships partial fixes (media perms grant + slack:// deep-link isolation, see "Fixes shipped in this PR" below); full huddle paint resolution gated on the CEF tracking issue. Joins the runtime-gap family in `feedback_cef_runtime_gaps.md` (Web Push, BrowserChannel, MediaPipe segmentation, **same-document popup paint**). |
| 6 | **Notifications** — native OS notifications; per-channel mute; DND; `notification_settings` toggle | ❌ **fail** | User received Slack DM from Shanu while NOT focused on OpenHuman. macOS Notifications perm granted for OpenHuman.app. Result: **no native toast fired**. Log shows zero `forward_native_notification` / `webview_notification` events at message-arrival time. CEF shim `installed shims browser_id=3 origin=https://app.slack.com/client` was registered, but page-side `new Notification(...)` never reached Rust handler. | **CHILD ISSUE TO FILE**: title `[Bug] webview/slack: native OS notifications never fire — page→Rust bridge broken`. Repro: signed-in Slack workspace, OpenHuman backgrounded, peer sends DM → no toast. Hypothesis: (a) Slack web suppresses Notification when its own permission state for the origin isn't `"granted"`, or (b) CEF Notification shim wrap doesn't actually call `forward_native_notification`. Need to verify CEF `Notification.permission` value at slack.com origin and confirm the constructor wrap path. Pre-audit gaps (per-channel mute missing; default=true) are still valid but moot until basic toast path works. |
| 7 | **Memory ingestion** — IDB scanner; `memory_doc_ingest` posts; current behavior groups by `channel_id` (per-day grouping deferred) | ❌ **fail (scanner never spawns)** | RPC call `openhuman.memory_recall_memories {"namespace":"slack-web:29da7de6...","limit":20}` returns `{memories:[]}`. Log shows: `[webview-accounts] slack ScannerRegistry not in app state` immediately after Slack account opens. Zero `memory_doc_ingest` events ever fired. **Root cause**: `app/src-tauri/src/lib.rs:998` does `manage(std::sync::Arc::new(slack_scanner::ScannerRegistry::new()))` — but `slack_scanner::ScannerRegistry::new()` already returns `Arc<Self>` (`mod.rs:744-746`). Result: managed state is `Arc<Arc<ScannerRegistry>>`. Lookup at `webview_accounts/mod.rs:1751` is `try_state::<Arc<ScannerRegistry>>()` → returns None → scanner never spawns. WhatsApp/Discord/Telegram lines (997/999/1000) correctly use the bare `ScannerRegistry::new()` (no extra Arc wrap). Slack alone is double-wrapped. | **ONE-LINE FIX**. Change `lib.rs:998` from `.manage(std::sync::Arc::new(slack_scanner::ScannerRegistry::new()))` to `.manage(slack_scanner::ScannerRegistry::new())`. Post-fix: confirmed scanner ingests one doc per channel (`emit_and_persist` at `mod.rs:230` explicitly groups by `channel_id` only — no per-day split); the `(channel_id, day)` shape from the original spec remains a deferred follow-up rather than a regression. Reactions/threads extraction gaps still apply but blocked by getting scanner running first. |
| 8 | **DOM snapshot** — fast-tick captures unread badges + channel list | ✅ pass | Sidebar matches web Slack: channel list (general, random, team-backend/frontend/product, notify-frontend-gi/se, External connections), DMs (Sanil 1, Alan, Aniketh, Cyrus, Mega Mind, Shanu, Steven), DMs+Activity nav badges (1 each), per-DM unread badge on Sanil (1). | DOM extraction working as designed. |
| 9 | **Multi-workspace** — switching workspaces; scanner tracks `team_id` | _TBD_ | _TBD_ | Pre-audit: `infer_team_id()` parses DB-name pattern (`slack_scanner/mod.rs:162-175`) — fragile |
| 10 | **Session persistence** — tab switch preserves warm session; no re-auth | ✅ pass | Hard-killed all OpenHuman + core processes (`pkill -9` + `kill 75132` on port 7788). Relaunched `pnpm dev:app` cold. Clicked Slack tile → already signed in to Vezures workspace, no prompts. Per-account `data_directory` (CEF profile + cookies) survives full process termination. | Persisted via `~/.openhuman-staging/users/<id>/cef/` profile. Logout-from-inside-Slack edge case (user-reported): in-app logout removes Slack tile from OpenHuman left rail; re-add comes back already signed in (Slack web's logout doesn't clear CEF cookies, OR sidebar removal isn't tied to actual session purge — UX quirk worth a follow-up). |
| 11 | **Search** — Slack built-in search functional | ✅ pass | User confirmed search works in-webview. | No app-layer interceptor needed. |
| 12 | **Navigation** — external links → system browser; allowed hosts `slack.com`, `slack-edge.com`, `slack-imgs.com`, `slack-files.com` resolve | ✅ pass | User confirmed external links open in system browser, in-app links stay. | Per #4 finding, `slackb.com` is correct CDN host — spec's `slack-imgs.com` + `slack-files.com` claim was outdated. |

## Smoke run procedure

For each criterion:

1. Reproduce in running app (`pnpm dev:app`).
2. Capture exact symptom + console/CDP log line if relevant.
3. Mark verdict in table.
4. If ❌: file child issue against `tinyhumansai/openhuman` titled `[Bug] webview/slack: <symptom>` linking back to #1016.
5. If ⚠️: note limitation + scope follow-up; decide whether to fix in this PR or defer.

## Additional bug discovered during smoke (not in issue body)

### Slack CEF surface goes blank after huddle interaction + tab switch

**Symptom**: After spawning a huddle popup (white blank window per #5) and dismissing it, the parent Slack webview becomes unclickable. Switching to OpenHuman home and clicking back to Slack: sidebar UI renders, but the entire CEF webview area is white.

**Repro**:
1. Open Slack account in OpenHuman.
2. Click "Start huddle" or any feature that triggers a popup `window.open`.
3. Close / dismiss popup.
4. Switch to OpenHuman home via sidebar.
5. Click Slack tab again → sidebar shown, CEF area is **white / blank**.

**Log evidence** (timestamps from `b9qimj6ka.output`):
- `14:14:57` first huddle popup spawned
- `14:35:02` second huddle popup spawned (the one that broke things)
- `14:39:18` Slack tab re-opened: `[webview-accounts] reused existing label=acct_29da7de6...`, `revealed bounds=Bounds { x: 76.0, y: 0.0, width: 924.0, height: 768.0 }`
- `14:39:23` Same again — Tauri-side reveal fires correctly with right bounds; CEF surface stays white

**Root-cause hypothesis**: CEF child popup window holds the GPU render context or some lifecycle state. When parent webview is hidden (tab switch) and revealed, CEF doesn't repaint. May share root cause with #5 (huddle popup blank).

**Child issue to file**: `[Bug] webview/slack: parent CEF webview goes blank-white after huddle popup interaction + tab switch`. Tauri-side reveal/bounds events fire correctly — bug is purely CEF render lifecycle.

## Known issues from issue body (verify status)

- Huddle popup uses `about:blank` whitelisting — fragile if Slack changes flow
- IDB scan interval 30s — messages may lag native push by up to 30s
- `OPENHUMAN_DISABLE_SLACK_SCANNER=1` env escape hatch (debug only)

## Pre-audit code-level gaps (from research dossier)

These were confirmed by static read of `main` before smoke. The smoke run will validate which manifest as user-visible bugs vs. intentional non-features.

1. **`webview_accounts/mod.rs:101`** — allowed_hosts has `slackb.com`; spec asks `slack-imgs.com` + `slack-files.com`. Image/file CDN may bounce out to system browser.
2. **`slack_scanner/mod.rs:178-225`** — memory grouping by `channel_id` only; spec requires `(channel_id, day)`. Single doc per channel may grow unbounded.
3. **`slack_scanner/extract.rs`** — reactions, threads (thread_ts + reply_count) not extracted.
4. **`webview_accounts/mod.rs:754-793`** (`forward_native_notification`) — only per-account mute; ignores Slack's own per-channel mute state.
5. **`notification_settings/mod.rs:33`** — default `true` (toast storm on first run).

## Fixes shipped in this PR

| Bug | Root cause | Fix | File:line | Verified |
|-----|-----------|-----|-----------|----------|
| A | `lib.rs:998` double-Arc-wrapped `slack_scanner::ScannerRegistry::new()` (which already returns `Arc<Self>`). Tauri lookup at `webview_accounts/mod.rs:1751` for `Arc<ScannerRegistry>` missed the `Arc<Arc<…>>` shape. Scanner never spawned. | Drop the redundant outer `Arc::new(...)` so the managed type matches the lookup. Mirrors the pattern already used for whatsapp/discord/telegram (lines 997/999/1000). | `app/src-tauri/src/lib.rs:998` | ✅ post-fix log shows `[sl] scanner up account=… interval=30s` and no `slack ScannerRegistry not in app state` warning |
| B | Native Slack notifications never fired. CEF Notification permission for `slack.com` origin remained `default`; the existing JS shim only masked `Notification.permission === "granted"` for the page check, but the real CEF Notification path silently no-op'd at the C++ level when no actual grant existed. | Issue a browser-scoped `Browser.grantPermissions(["notifications"])` CDP call against the provider's origin right after attach. Adds an `origin_of()` helper to extract `scheme://host[:port]` from `real_url`. | `app/src-tauri/src/cdp/session.rs` (new helper + grant call between shim injection and `Page.enable`) | ✅ post-fix log shows `[cdp-session][…] granted notifications for origin=https://app.slack.com`; macOS toasts now fire when OpenHuman is unfocused |
| D | (Surfaced once Bug A let the scanner run.) Slack's client router does `pushState('/client/<workspace>/<channel>')` shortly after first load, stripping the `#openhuman-account-<id>` fragment from the page-target URL. Scanner's `find` matcher `starts_with(prefix) && ends_with(fragment)` failed every tick after pushState. Memory ingest stayed empty. | Relax the matcher: try strict (prefix + fragment) first, fall back to any same-origin Slack page target. Per-account `data_directory` isolation guarantees one Slack page-target per origin per account, so the broader match is safe. Same fix in both `scan_once` and `dom_scan_once`. | `app/src-tauri/src/slack_scanner/mod.rs:114-135` (`scan_once`) and `:740-755` (`dom_scan_once`) | ✅ post-fix log shows multiple `[sl][…] memory upsert ok namespace=slack-web:… key=<channel_or_dm> msgs=<N>` lines (general/random/team-product channels + alan/sanil/shanu/elvin516/nikhil DMs); RPC `openhuman.memory_recall_memories {namespace:"slack-web:<acct>"}` returns 7 docs |
| C | Slack `slack://T.../magic-login/<token>` deep-links from inside the embedded Slack webview were being routed to `open_in_system_browser`. macOS handed them to the native Slack desktop app, which consumed the magic-login token and signed the workspace into the native client — breaking embedded-webview workspace isolation. Same risk for `discord://`, `tg://`, `msteams://`. Discovered while instrumenting #1074. | Introduce `is_provider_native_deep_link_scheme()` helper. In both the on_navigation external-fallback and on_new_window external-fallback paths, suppress these schemes BEFORE the `open_in_system_browser` call — the page's HTTPS fallback completes sign-in without the deep link. `zoomus://` / `zoommtg://` continue to take the existing `rewrite_provider_deep_link` path (locked by `zoomus_join_still_rewrites_and_is_recognized_as_native_scheme`). | `app/src-tauri/src/webview_accounts/mod.rs` (new helper near `popup_should_stay_in_app`; wired into both navigation paths) | ✅ unit tests in `webview_accounts::tests::deep_link_scheme_*`; manual smoke deferred to merge of [#1074](https://github.com/tinyhumansai/openhuman/issues/1074) PR |
| E | Slack Huddles' getUserMedia / getDisplayMedia surface had no browser-level permission grants — `enumerateDevices()` returned empty inside the huddle popup, the join button silently no-op'd. Same class as Bug B (notifications) but for media. Mirrors the gmeet pattern landed in PR #1054. | Extend the per-origin grant list in `cdp/session.rs::run_session_cycle` to add `audioCapture` / `videoCapture` / `displayCapture` / `clipboardReadWrite` for `app.slack.com` alongside `notifications`. | `app/src-tauri/src/cdp/session.rs` (slack arm in the `Browser.grantPermissions` block) | ✅ unit test `origin_host_is_matches_app_slack_com_for_huddle_grant`; functional verification deferred to the CEF popup-paint fix in [#1079](https://github.com/tinyhumansai/openhuman/issues/1079) (popup currently can't render long enough to use the perms; once it can, mic/cam/screenshare are wired) |

## Out of scope (separate child issues recommended)

- **Huddle popup blank** — root cause refined after [#1074 Phase 0 CDP probe](https://github.com/tinyhumansai/openhuman/issues/1074). Slack uses `popup.document.write(html)` to inject huddle UI into the same `about:blank` document, NOT `popup.location = url` — so there is no URL signal to intercept. The bug is CEF's paint/compositor lifecycle on same-document popups (popup paints first frame correctly, then surface goes white). Tracked in **[#1079](https://github.com/tinyhumansai/openhuman/issues/1079)**. Pre-conditions for huddle audio/video are now in place (`Browser.grantPermissions` for `audioCapture`/`videoCapture`/`displayCapture` on `app.slack.com`, see Fixes shipped in this PR row C); huddle becomes functional immediately once the CEF paint bug clears.
- **CEF parent webview blanks after huddle interaction + tab switch** — likely shares root cause with the huddle popup; verify after that lands.
- **`pnpm dev:app` `restart_app` regression on PR #1007** — orthogonal bug, broader than Slack; affects all dev:app sessions. Requires instrumenting `getActiveUserIdFromCore()` to confirm whether it returns `null` in dev mode and triggers the seed/identity mismatch.
- **In-app Slack logout removes the OpenHuman sidebar tile but cookies persist** — UX inconsistency (re-add = already signed in). Decide between (a) keep the tile after in-app logout, or (b) purge the per-account CEF profile on the logout signal.
- **Pre-audit hypotheses dropped after smoke**: `slackb.com` is the working CDN host (spec's `slack-imgs.com`/`slack-files.com` were stale) — DO NOT change `webview_accounts/mod.rs:101`; reactions + threads + per-channel mute + per-day grouping all dropped or deferred (gated on confirming actual gaps after Bug A+D made the scanner usable).

## Sign-off

- Tester: oxoxDev
- Result: ✅ Three confirmed bugs (#7 scanner spawn / #6 notifications / Bug-D scanner target match) fixed. Memory ingest end-to-end working for 7 channels/DMs. Five other criteria pass; one skipped (no second workspace); huddle popup paint deferred to CEF tracking issue [#1079](https://github.com/tinyhumansai/openhuman/issues/1079) (root cause refined from the original "popup.location lost" hypothesis to "Slack uses popup.document.write into about:blank → CEF same-document popup paint dies after first frame"); related slack:// deep-link workspace-isolation leak fixed as Bug C; Slack Huddles media perms grant landed as Bug E (functional once #1079 clears).
- Date: 2026-04-29 (initial smoke); 2026-05-01 (huddle root-cause + Bug C/E follow-ups via #1074 instrumentation)
- Action items: [#1079](https://github.com/tinyhumansai/openhuman/issues/1079) tracks the CEF same-document popup paint lifecycle (the new path for "huddle popup blank"). The legacy "CEF blank-after-huddle on tab switch" entry above likely shares root cause with #1079 — revisit once that lands. dev:app restart-loop regression remains a separate ticket.
