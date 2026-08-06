# Feed Post Media Web-Only Implementation Plan (#4924)

**Goal:** Replace the text-only feed composer's missing-media gap with a note
pointing users to tiny.place, and record that post media is web-only by design.
Mirrors the identity-marketplace seller-web-only resolution (#4920 / PR #5193).

**Architecture:** Additive UI only. A persistent note at the bottom of
`FeedComposer` opens `https://tiny.place` via the existing `openUrl()` helper. No
SDK, core, `manifest.rs`, or submodule changes — the tiny.place backend serves no
post-media field. Scope recorded in two doc comments.

**Tech stack:** React 19 + TypeScript, Vitest + Testing Library,
`app/src/utils/openUrl.ts`, Rust module doc comment.

## Global constraints

- **No i18n:** `FeedComposer` is hardcoded English (`"What's on your mind?"`,
  `"Post"`, `"Posting…"`). New strings match — no `useT()`, no locale-file edits.
- **Web URL is a hardcoded prod constant:** `https://tiny.place` (home = feed).
  Mirrors `SELL_ON_WEB_URL` / `FUND_PAGE_URL`.
- **External links go through `openUrl`** — never a raw `window.open`.
- **Out of scope (do not touch):** `vendor/tinyplace/**` (incl. the submodule
  pointer), `manifest.rs`, `invokeApiClient.ts`, the compose/like/comment flows.

## Tasks

- [x] **Task 1 — Composer note + CTA.** `FeedSection.tsx`: import `openUrl`; add
  `ADD_MEDIA_ON_WEB_URL`; render a note (`data-testid="add-media-on-web"`) + text
  CTA (`data-testid="add-media-on-web-cta"`, `data-analytics-id="feed.addMediaOnWeb"`)
  at the bottom of `FeedComposer` that `void openUrl(ADD_MEDIA_ON_WEB_URL)`. Header
  docstring gains a "post media is web-only" note.
- [x] **Task 2 — Rust scope note.** `src/openhuman/tinyplace/mod.rs`: add a
  "Feed scope (post media is web-only)" doc-comment section after the marketplace
  one. Docs-only; `cargo fmt --check` verifies.
- [x] **Task 3 — Tests.** `FeedSection.test.tsx`: mock `openUrl`; assert the note
  renders and the CTA calls `openUrl('https://tiny.place')`.
- [x] **Task 4 — GitHub bookkeeping (outward-facing).** Done: commented the
  web-only resolution on #4924, and marked the #4776 §2 "Images / media in posts"
  item N/A (web-only) via an epic status comment (the epic body is owner-gated).

## Verification

- `pnpm --filter openhuman-app test` — FeedSection green (incl. 2 new cases).
- `tsc --noEmit` clean; `eslint` no new errors; `prettier --check` clean.
- `cargo fmt --check` clean (doc-comment-only Rust change).
- `git diff vendor/tinyplace` empty — submodule pointer untouched (mergeable).
