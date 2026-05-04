# WhatsApp Webview Parity — QA Matrix

> Issue: [#1017](https://github.com/tinyhumansai/openhuman/issues/1017)
> Branch: `feat/1017-whatsapp-parity-audit`
> Tester: oxoxDev
> Build: main @ `b11b8f33` + `feat/1017-whatsapp-parity-audit` HEAD
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
| 1 | **Auth** — QR code scanning flow + "Link a device"; session persists across restarts | ✅ pass | User completed QR-scan login. Hard-killed all OpenHuman processes + relaunched `pnpm dev:app` cold → WhatsApp tab signed-in to same number, no QR re-prompt. Per-account CEF `data_directory` persists. | Restart-persistence verified end-to-end on dev:app (post the in-session restart-loop hack). |
| 2 | **Messaging** — text, voice messages, images, videos, documents in individual + group chats | ✅ pass | User confirmed: send to individuals, send to groups, voice-note send. Text + voice-note paths work end-to-end. | Voice-message body still empty in IDB extraction (pre-audit: `type="ptt"` no transcription) — moot until #6 IDB reads work. |
| 3 | **Media** — image/video previews; downloads; camera/voice recording functional | ⚠️ partial | Image preview ✅, image download ✅, voice-note send ✅. **Video playback ❌** — clicking a video in chat or on Status forces download instead of inline play. | **Real bug**. Symptom: `<video>` element either not rendering or codec/MIME blocked. Likely CEF media-stream / codec config issue. Worth checking whether mp4/h264 codec is bundled with our CEF build. Same symptom on Status (#8). |
| 4 | **Voice/video calls** — outbound + inbound connect; audio/video streams | ❌ fail | User attempted both voice and video calls — neither connects. | **Caveat**: WhatsApp Web has structural call limits vs native (voice support is partial, video extra-limited). Recommend cross-test on Safari/Chrome at `web.whatsapp.com` to confirm whether calls work in any browser before treating as our bug. If broken on Safari too → WhatsApp Web limitation, not OpenHuman. |
| 5 | **Notifications** — native OS notifications; per-chat mute; `notification_settings` toggle | ✅ pass | Toast fires when OpenHuman is NOT focused (Terminal in foreground at message time, toast appeared on resend). No toast when OpenHuman IS focused — **correct intentional behavior** per `bypass_when_focused` flag (`webview_accounts/mod.rs:640-657`). | Per-chat mute desync (audit gap) untested — gated on actually using mute on WhatsApp side; defer. |
| 6 | **Memory ingestion (IDB)** — 30s scan walks IndexedDB stores; extracts message metadata, chat names, contact names; `memory_doc_ingest` posts land in core | ❌ **fail** | Scanner DID spawn (no double-Arc bug like Slack #7): `[wa] scanner up account=198d1da7... fast=2s full=30s`. **All 4 IDB reads fail** with CDP error: `[wa][idb] read message failed: cdp error: {"code":-32000,"message":"Could not get index"}` (and same for chat / contact / group-metadata). Result: `full scan ok messages=0 chats=0 dom=0`. RPC query `openhuman.memory_recall_memories {namespace: "whatsapp-web:198d1da7..."}` → `{memories:[]}`. | **Real bug**. WhatsApp Web likely changed its IndexedDB index schema. Code at `idb.rs:154-164` uses `IndexedDB.requestData` with `indexName: ""` (primary-key fallback) but Chromium 124's CDP rejects with "Could not get index". Investigation: (a) call `IndexedDB.requestDatabaseNames` first to enumerate, (b) call `IndexedDB.requestDatabase` to get current store + index list for `model-storage`, (c) confirm `message`/`chat`/`contact`/`group-metadata` store names still exist + figure out current primary-key index name. May need version bump in API params or alternate read path. |
| 7 | **Memory ingestion (DOM)** — 2s scan captures message rows; emits only on visible-set hash change (no idle spam) | ❌ fail (gated on #6) | Same `full scan ok messages=0 chats=0 dom=0` line confirms `dom=0` too. The fast-tick DOM scan returned zero captures despite an active chat being viewed. | Gated by same scanner-side issue as #6 OR DOM selector drift in `dom_snapshot.rs`. Resolve #6 first, then verify whether DOM emits resume. If DOM stays at 0 post-#6-fix, separate root cause (selectors stale). |
| 8 | **Status/Stories** — viewable | ⚠️ partial | Status tab opens, stories list visible. **Video stories don't play** — same symptom as #3 video bug. | Same root cause as #3. One fix likely covers both. |
| 9 | **Session persistence** — tab switch preserves warm session; WhatsApp Web link stays active | ✅ pass | Hard kill (`pkill -9` on all OpenHuman + core processes + port 7788) → cold relaunch → already signed in. CEF profile survives full process termination. | (Restart-during-active-session edge case — phone offline >7 days — not exercised; infra works.) |
| 10 | **User agent** — custom UA in manifest passes WhatsApp's browser detection | ✅ pass | WhatsApp Web loaded cleanly to authenticated chat list — no "browser not supported" banner, no QR-scan rejection. Chrome/124 UA still passes. | Drift risk as CEF advances; not actionable now. |
| 11 | **Navigation** — external links → system browser; internal stay | ✅ pass | User confirmed external links open in system browser; internal WhatsApp links stay in webview. | `allowed_hosts` at `mod.rs:92` = `["whatsapp.com", "whatsapp.net", "wa.me"]` working. |

## Smoke run procedure

For each criterion:

1. Reproduce in running app (`pnpm dev:app`).
2. Capture exact symptom + console/CDP log line if relevant.
3. Mark verdict in table.
4. If ❌: file child issue against `tinyhumansai/openhuman` titled `[Bug] webview/whatsapp: <symptom>` linking back to #1017.
5. If ⚠️: note limitation + scope follow-up; decide whether to fix in this PR or defer.

## Known issues from issue body (verify status)

- QR scan + "Link a device" must be the auth path (no password — WhatsApp has no email/password)
- Voice/video calls require mic/cam access through CEF webview
- Per-chat mute on WhatsApp Web side must propagate through to our notification suppression
- IDB scan interval 30s — messages may lag native push by up to 30s
- Voice-message extraction lossy (no transcription)
- UA pinned to Chrome/124 — may drift out of WhatsApp's supported list

## Pre-audit code-level gaps (from research dossier)

These were confirmed by static read of `main` before smoke. The smoke run will validate which manifest as user-visible bugs vs. intentional non-features.

1. **Auth wiring** — QR/link-device flow is stub-level; no end-to-end UX verified in code. Criterion is UX, not extraction.
2. **Media handling** — stub; type field detected but no preview/download/recording wiring in webview integration.
3. **Voice/video calls** — stub; no event capture in scanner. Calls don't persist in IDB so memory-side is moot, but UX (mic/cam permission, stream rendering) is unverified.
4. **Per-chat mute desync** — WhatsApp Web's per-chat `muted` flag (in IDB chat record) is ignored by `NotificationBypassPrefs`; toasts fire even when WhatsApp side is muted.
5. **Voice messages as noise** — `type="ptt"` extracted with empty body (no transcription); ingested as zero-content memory docs.
6. **EU locale date parser** — `app/src-tauri/src/whatsapp_scanner/dom_snapshot.rs:605-620` parses M/D/YYYY only; DD/MM/YYYY locales misparsed → wrong-day timestamps.
7. **UA hardcoded** — `recipes/whatsapp/manifest.json:6` pins `Chrome/124.0.0.0`; drift risk as CEF advances and WhatsApp updates supported-browser list.
8. **Status/Stories not extracted** — scanner has no Status surface at all.
9. **Session-invalidation has no UI surface** — `scan_once` errors logged at `mod.rs:169` but user sees nothing on phone-offline >7 days; link silently dies.
10. **Hardcoded scan constants** — `FULL_SCAN_INTERVAL` 30s (`mod.rs:43`), `FAST_SCAN_INTERVAL` 2s (`mod.rs:47`), `MAX_RECORDS_PER_STORE` 20_000, `PAGE_SIZE` 500, `MAX_BODY_CHARS` 4000, `DATABASE_NAME` "model-storage", CDP port 19222 (`mod.rs:39`), `allowed_hosts` = `["whatsapp.com", "whatsapp.net", "wa.me"]`. None tunable at runtime.

## Fixes shipped in this PR

| Bug | Root cause | Fix | File:line | Verified |
|-----|-----------|-----|-----------|----------|
| 1 | `whatsapp_scanner/idb.rs:159` sent `"indexName": ""` to CDP `IndexedDB.requestData`. CEF 146 backend rejects empty-string with `{"code":-32000,"message":"Could not get index"}`. CDP spec says empty string == primary-key index, but the C++ backend requires the field UNSET (omitted entirely). All 4 IDB stores (`message`, `chat`, `contact`, `group-metadata`) failed; scanner emitted zero memory docs; `whatsapp-web:<acct>` namespace stayed empty. | Drop the `"indexName": ""` line from the JSON params. Add a comment block mirroring the working pattern already documented in `slack_scanner/idb.rs:210-214` and `telegram_scanner/idb.rs:210` (both have explicit notes). Slack + Telegram had this fix already; only WhatsApp regressed. | `app/src-tauri/src/whatsapp_scanner/idb.rs:152-167` (1-line deletion + 6-line comment) | ✅ runtime: post-fix log shows `[wa][<acct>] full scan ok messages=20000 chats=2249` (was `0/0` pre-fix). Plus `cargo test --lib whatsapp_scanner` 20/20 (incl. new `requestdata_params_omit_index_name` regression test). |
| 2 | Once Bug 1 unblocked the IDB walk, `dom_snapshot::parse_rows` still returned 0 — three drift points in WhatsApp Web's HTML had landed since the parser was last touched. (a) `data-id` is no longer `<fromMe>_<chatId>_<msgId>` — it's bare msgId hex (e.g. `AC2E44BDA…`, 32 hex chars). (b) `span.selectable-text` class gone; bodies live in plain `span[dir="ltr|rtl"]` (existing fallback already covers this — only the doc was stale). (c) Active chat JID is no longer in URL, on `data-id`, or on any DOM attribute we could find — only the conversation header carries it. | (a) `split_data_id` accepts both legacy compound and bare-msgId-hex shapes. Bare format returns `(false, "", msg_id)` and the merge in `mod.rs` recovers the missing fields by msgId-tail and active-chat reverse-lookup. (b) Module-level doc comment refreshed to mention both `selectable-text` and `dir` matchers. (c) New `parse_active_chat_name` walks `header[data-testid="conversation-header"]` for the first non-icon `<span>`'s text, skipping `wds-icon` / Material-style ligatures so the chat title wins. | `app/src-tauri/src/whatsapp_scanner/dom_snapshot.rs` (split_data_id rewrite + parse_active_chat_name + looks_like_icon_ligature + 4 new tests) | ✅ runtime: post-fix log shows `[wa][<acct>] full scan ok … dom=N` with N>0 (was `dom=0` pre-fix); active chat name extracted cleanly for 1:1 (`"Anushka"`-shape), group (`"<group title>"`-shape), broadcast (`"<broadcast title>"`-shape). |
| 6 (partial) | Bug 6 is the DOM↔IDB chat correlation step needed to flip extracted DOM bodies into actual memory docs. This PR ships the **plumbing** — the rest is gated on a follow-up that closes IDB chat_names gaps. | (a) `ScanSnapshot` gains `active_chat_name`. (b) `emit_snapshot` reverse-looks-up the name in `snap.chats` with exact → case-insensitive → substring fallback (substring only when exactly one candidate, to avoid cross-attribution on common tokens). (c) DOM-only appended rows stamp the resolved jid into `chatId` when no DOM-side value exists. (d) Merge by-msg-id loop also tries the trailing segment of the IDB compound id so legacy and bare DOM ids both converge. (e) One `info!` log per tick records the resolution outcome. The plumbing is defensive — when reverse-lookup returns `None`, rows flow through with `chatId=Null` and get dropped at `emit_grouped_whatsapp` exactly as before. No regression. | `app/src-tauri/src/whatsapp_scanner/mod.rs` (active_chat_jid resolver, `ScanSnapshot.active_chat_name`, msgId-tail fallback, DOM-only chatId stamp, diagnostic log) | ✅ static: cargo check / fmt / clippy clean, 20/20 tests. 🔍 runtime: `[wa][<acct>] active chat resolution: name=Some("X") → jid=…` logs every tick. Reverse-lookup returns `Some` only when the IDB chats map already has the matching name — the gaps that block end-to-end memory ingest are tracked in the follow-up issue (link added on PR open). |

## Out of scope (separate child issues recommended)

- **Bug 6 + 7 follow-up — IDB chat_names gaps block end-to-end memory ingest**: post-Bug-2 + Bug-6-plumbing the active-chat name extraction works for 1:1, group, and broadcast titles, but the IDB-side `chats[jid].name` map is missing entries for every chat type tested. Three sub-causes: (a) group-metadata `subject` extraction falters for some group id shapes (normalize_id likely the culprit), (b) un-saved 1:1 contacts have no `pushname`/`notify`/`verifiedName` so `normalize_contact` returns None, (c) the broadcast store isn't walked at all. Estimated 130 LOC fix across 3 sub-tasks. Draft body at `.claude/scratch/whatsapp-bug-6-7-followup.md`; file as separate issue after this PR opens.
- **Bug 3 — Video forces download (criteria #3 + #8 Status)**: CEF build lacks proprietary H.264/AAC codecs. WhatsApp Web's `<video>` element is `video/mp4` (H.264) → unsupported → falls back to download. Build/packaging concern — requires switching CEF prebuilt binary distribution to one that includes proprietary codecs. NOT a code fix in this repo.
- **Bug 4 — Calls don't connect (criterion #4)**: needs cross-browser control test (Safari/Chrome at `web.whatsapp.com`) to disambiguate whether voice/video calls work in any browser for the user's WhatsApp account/region. If they fail in Safari too → WhatsApp Web platform limitation, not OpenHuman. If they work in Safari but not OpenHuman → likely missing `getUserMedia` permission grant for `web.whatsapp.com` origin in the migrated-provider webview path; mirror Bug B's pattern from #1016.
- **Bug 5 — Voice messages with empty body**: gated on Bug 6+7 follow-up — once IDB walks succeed AND the chats map covers all chat types, verify whether `type="ptt"` records carry text body or stay empty. Decide between `[voice message]` placeholder (gives signal but adds noise to text search) or skip (cleaner doc but loses count signal).
- **EU locale date parser**: `dom_snapshot.rs` parses `M/D/YYYY` only — EU locales misparsed. Defer.
- **UA drift**: hardcoded `Chrome/124` in manifest; not currently broken. Defer.
- **Per-chat mute desync**: WhatsApp Web's `muted` flag in IDB ignored by `NotificationBypassPrefs`. Defer until basic IDB walk works.

## Sign-off

- Tester: oxoxDev
- Result: ✅ Bug 1 (IDB walk — `indexName` empty-string trap) and Bug 2 (DOM scrape — bare-msgId + active-chat header parse) both fixed in code, runtime-verified, and locked by unit tests (20/20). Bug 6 plumbing (active-chat-name extraction + chats-map reverse lookup + DOM-only chatId stamp + msgId-tail merge fallback) landed but blocked end-to-end on IDB chat_names gaps tracked in the follow-up issue. The merge plumbing is defensive — when the reverse-lookup misses, rows drop with no chatId exactly as before; no regression.
- Date: 2026-04-30
- Action items: file the follow-up issue immediately after this PR opens (draft at `.claude/scratch/whatsapp-bug-6-7-followup.md`). File separate issues for Bug 3 video codec + Bug 4 calls if/when cross-browser control test pins them on us.
