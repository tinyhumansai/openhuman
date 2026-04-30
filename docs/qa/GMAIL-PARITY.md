# Gmail Webview Parity — QA Matrix

> Issue: [#1020](https://github.com/tinyhumansai/openhuman/issues/1020)
> Branch: `feat/1020-gmail-parity-audit`
> Tester: oxoxDev
> Build: main @ `b11b8f33` + `feat/1020-gmail-parity-audit` HEAD
> Date: 2026-04-30
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
| 1 | **Auth** — Google sign-in completes (email → password → 2FA); Google auth popup rewriting works (navigate parent instead of popup); session persists across restarts | ✅ pass (after fix) | First attempt got stuck after password — `accounts.google.co.in/SetSID` redirect leaked to system Chrome (the SetSID hop's host is country-localized; `provider_allowed_hosts` for gmail only suffix-matched `google.com`). Fix added `is_google_sso_host` predicate matching `accounts.google.<any-cctld>` + `accounts.youtube.com`. Post-fix login completed end-to-end into inbox in 2-3s. | **Bug fixed in this PR**: Google SSO ccTLD coverage. 5 new unit tests lock the predicate. Same fix benefits Google Meet (#1022). |
| 2 | **Read email** — Inbox loads; email threads open; HTML rendering correct; inline images display | ✅ pass | Inbox loads (3,508 messages visible in screenshot); threads open and render natively. | Browser-native rendering. |
| 3 | **Compose** — New email compose window opens; To/CC/BCC fields work; rich text formatting; send completes | ✅ pass | User composed and sent a test email successfully. | Web UI handles send directly — bypasses our `writes.rs::gmail_send` stub since the user is interacting with Gmail's own compose dialog, not our Tauri command. The OpenHuman API stub for `gmail_send` (line 160-170) is a separate surface for programmatic send (e.g. agent-driven) and remains stubbed — see #9. |
| 4 | **Reply / Forward** — Reply, Reply All, Forward from thread view | ✅ pass | User confirmed reply + forward work in webview. | Gmail web UI handles directly. |
| 5 | **Search** — Gmail search bar works; results render correctly | ✅ pass | User confirmed search returns results in fullscreen (screenshot shows search dropdown with prefix matches + filter chips). | At narrow window widths the search bar visually compacts — Gmail's own responsive layout, cosmetic, not a bug. |
| 6 | **Labels** — `list_labels` returns sidebar labels; apply/remove labels from messages | ✅ pass | Sidebar labels visible (Inbox, Starred, Snoozed, Sent, Drafts, Categories, More, custom Labels section). | Read works via `reads.rs::list_labels`. Apply/remove from messages goes through Gmail web UI directly (not our `writes.rs::gmail_add_label` stub which is an OpenHuman API surface for programmatic labeling). |
| 7 | **Attachments** — Download attachments; attach files to compose; inline preview | ✅ pass | User confirmed preview + download work. | Web UI handles directly. |
| 8 | **API: list_messages** — Atom feed returns unread messages; extend to cover read messages and pagination | _TBD_ | _TBD_ | Pre-audit: partial. Atom feed only ~20 most-recent unread; no pagination by Gmail design. |
| 9 | **API: send** — Implement (currently stubbed) | _TBD_ | _TBD_ | Pre-audit: stub. Requires CDP Input automation of compose dialog — fragile. |
| 10 | **API: trash** — Implement (currently stubbed) | _TBD_ | _TBD_ | Pre-audit: stub at `writes.rs:20-25`. |
| 11 | **API: add_label** — Implement (currently stubbed) | _TBD_ | _TBD_ | Pre-audit: stub at `writes.rs:27-35`. |
| 12 | **Notifications** — Native OS notifications for new emails; `notification_settings` toggle honored | ✅ pass (after fix) | First test: zero `forward_native_notification` events fired even with #1028's `Browser.grantPermissions` in place — colleague mail arrived only after manual Gmail refresh. Diagnosed: Gmail's BrowserChannel real-time push (`mail.google.com/mail/u/0/channel/bind`) does NOT deliver to CEF webviews, and Web Push (FCM service-worker) requires Chromium glue absent in CEF builds, so the page never observes new mail and never calls `new Notification(...)`. Fix: new `gmail::notify_poll` task polls the Atom feed every 30 s, dispatches synthetic toasts for newly-seen unread IDs via `forward_synthetic_notification`. Verified end-to-end against staging: colleague test mail produced macOS banner under "OpenHuman" attribution within 30 s, click routed to Gmail account. | **Bugs fixed in this PR**: (a) Gmail real-time delivery bypass via Atom-feed poll (new `gmail/notify_poll.rs`); (b) per-fetch `disable_cache` option on `cdp_fetch` so polls observe fresh atom bytes; (c) toasts now attribute to `app_id` always (dropped the dev-mode `com.apple.Terminal` hack at `webview_accounts/mod.rs:929,947` so packaged-bundle and dev share the OpenHuman attribution that the user has notification permissions on). 6 unit tests cover seen-set FIFO + persistence; lifecycle wired through `webview_account_close` / `purge` and the `drain_for_shutdown` shutdown path. |
| 13 | **Memory ingestion** — Recipe.js DOM scraper feeds ingest events; plan migration to CDP-based ingestion with `memory_doc_ingest` | _TBD_ | _TBD_ | Pre-audit: partial. `recipe.js` polls `tr.zA` rows every 2s, pushes `{messages, unread, snapshotKey}` to `api.ingest()` → `provider_surfaces_ingest_event`. No `memory_doc_ingest` call. |
| 14 | **Page load reliability** — Triple-signal fallback (native + CDP + 15s watchdog) verified stable; no missed loads | _TBD_ | _TBD_ | Pre-audit: wired. Triple-signal fallback at `webview_accounts/mod.rs:1084-1085, 1583-1626`. Watchdog 15s at line 498. |
| 15 | **Session persistence** — Tab switch preserves session; Google cookies retained | ✅ pass | User confirmed switching to OpenHuman home and back to Gmail keeps the session live. Earlier in session, Gmail tab loaded already-signed-in across full process kill + relaunch (CEF data_directory persistence working). | Same UX quirk surfaced as Slack #1016: in-app Gmail logout removes the OpenHuman sidebar tile but doesn't purge CEF profile cookies, so re-add lands already signed in. Track in the existing logout-UX follow-up. |
| 16 | **Multi-account** — `/mail/u/0/`, `/mail/u/1/` paths handled correctly | _TBD_ | _TBD_ | Pre-audit: partial. Atom feed URL hardcoded `/mail/u/0/` at `reads.rs:78`. |

## Smoke run procedure

For each criterion:

1. Reproduce in running app (`pnpm dev:app`).
2. Capture exact symptom + console/CDP log line if relevant.
3. Mark verdict in table.
4. If ❌: file child issue against `tinyhumansai/openhuman` titled `[Bug] webview/gmail: <symptom>` linking back to #1020.
5. If ⚠️: note limitation + scope follow-up; decide whether to fix in this PR or defer.

## Known issues from issue body (verify status)

- Write ops (`send`, `trash`, `add_label`) all stubs — require CDP Input-event automation that is fragile against Gmail UI churn.
- `Page.loadEventFired` unreliable — triple-signal fallback mitigates but is not ideal.
- Google auth popup rewriting intercepts `accounts.google.com` popups and redirects parent — may break on Google login flow changes.
- Atom feed only returns ~20 most-recent unread per label (Gmail-imposed limit).
- Recipe.js (legacy JS injection) still active — should be migrated to pure CDP.

## Pre-audit code-level gaps (from research dossier)

These were confirmed by static read of `main` before smoke. The smoke run will validate which manifest as user-visible bugs vs. intentional non-features.

1. **Auth wiring** — popup rewriting only covers `accounts.google.com` parent-navigation case (`webview_accounts/mod.rs:2655-2712`); no explicit 2FA path verified end-to-end in code.
2. **Read email** — feed-based listing + print-view fetch only; no thread-state, no read/unread flag toggles, no inline-image cache strategy beyond browser-native rendering.
3. **Compose stub** — `writes.rs:12-18` returns "not implemented" string; no compose-dialog UI automation exists.
4. **Reply / Forward** — no code path; would need DOM/CDP automation of thread-action buttons.
5. **Search fragility** — `reads.rs:121-203` polls snapshot 15× at 0.4s after `Page.navigate` to `#search/<query>`; brittle to Gmail table redesign and slow on cold loads.
6. **Labels read-only + English-only** — sidebar snapshot at `reads.rs:686-727`; system label names hardcoded English at `reads.rs:779-797`; `add_label` stubbed at `writes.rs:27-35`.
7. **Attachments missing** — no download capture, no compose-attach automation, no inline-preview hook.
8. **list_messages limit** — Atom feed returns ~20 most-recent unread by Gmail design; no pagination is possible from this surface.
9. **trash + add_label stubs** — `writes.rs:20-25` and `writes.rs:27-35` return stub errors.
10. **Recipe.js still active** — legacy JS injection at `recipe.js` polls `tr.zA` rows every 2s and emits via `api.ingest()`; runs counter to the "no new JS injection in CEF webviews" rule and should migrate to a CDP-driven scanner with `memory_doc_ingest` calls.
11. **Multi-account hardcoded** — Atom feed URL pinned to `/mail/u/0/` at `reads.rs:78`; secondary accounts (`/mail/u/1/`, `/mail/u/2/`, …) not handled.

## Hardcoded constants worth capturing

| Constant | Value | File:line |
|----------|-------|-----------|
| Atom feed base URL | `https://mail.google.com/mail/u/0/feed/atom` | `reads.rs:78` |
| Print-view URL template | `https://mail.google.com/mail/u/0/?ui=2&view=pt&search=all&th={escaped}` | `reads.rs:672-673` |
| Search row selector | `tr.zA` | `reads.rs:256, 532` |
| Watchdog timeout | 15s | `webview_accounts/mod.rs:498, 1628` |
| URL fragment account marker | `#openhuman-account-<id>` | `session.rs:8, 28` |
| Recipe.js poll interval | 2000 ms | `runtime.js:31` |
| Search snapshot polling | 400 ms × 15 attempts | `reads.rs:181-182` |
| System label names (English-only) | hardcoded | `reads.rs:779-797` |
| Notification poll interval | 30 s | `notify_poll.rs:44` |
| Notification poll initial delay | 15 s | `notify_poll.rs:49` |
| Notification poll limit | 20 entries | `notify_poll.rs:54` |
| Seen-set FIFO cap | 200 entries | `notify_poll.rs:60` |

## Sign-off

- Tester: oxoxDev
- Result: _TBD_
- Date: _TBD_
- Action items: _TBD_
