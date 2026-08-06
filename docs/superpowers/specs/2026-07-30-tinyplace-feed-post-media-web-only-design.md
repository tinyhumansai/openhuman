# Design: Feed post media is web-only (#4924)

Part of #4776 (Tiny Place audit) · §2 Feed · sibling of the #4920 web-only resolution (PR #5193).

## Problem

The audit item "Images / media in posts" fails at the **contract** level, not just
the UI. Posts are text-only end-to-end:

- The vendored `tinyplace` SDK carries no media field on the **write** side
  (`PostCreate` = `{ body, content_type, post_id }`) or the **read** side
  (`Post` / `GqlPost` = `body`, counts, author, timestamps — no media).
- The tiny.place **backend** serves no post-media field. Verified against
  tiny.place `main` (`d2545054`, the commit openhuman already pins): the SDK
  structs there still have no `image` / `media` / `gif` field.
- The desktop composer sends `{ body }` only (`FeedSection.tsx` →
  `feeds.createPost` → `manifest.rs` → SDK `feeds::create_post`).

Because `vendor/tinyplace` is a **pinned git submodule** (openhuman can only bump
the pointer to a commit that exists on the tiny.place remote), the media contract
cannot be added from this repo. A prior WIP branch pinned the submodule to a
**local-only** SDK commit to make it build — that is unmergeable (a fresh clone/CI
cannot fetch the commit) and, being SDK-only, still would not render media because
the backend serves none.

## Decision

Resolve **web-only**, mirroring the identity-marketplace seller resolution (#4920 /
PR #5193). Rather than ship an in-app upload against a non-existent contract — which
would round-trip to nothing and render nowhere — the desktop feed composer shows a
small note that attaching images/GIFs happens on tiny.place, with a CTA that opens
the web app via the existing `openUrl()` helper.

## Changes

1. **UI** (`app/src/agentworld/pages/FeedSection.tsx`) — additive only. A subtle
   note + text CTA at the bottom of `FeedComposer` opens `https://tiny.place` (the
   home page is the feed) through `openUrl()`. Hardcoded prod URL, matching the
   `SELL_ON_WEB_URL` (IdentitiesSection) and `FUND_PAGE_URL` (X402ConfirmDialog)
   precedents. Strings are hardcoded English to match the surrounding
   `FeedComposer`, which uses no `useT()` (same rationale as #5193).

2. **Scope docstrings** — a "Feed scope (post media is web-only)" note in the
   `FeedSection.tsx` header and in `src/openhuman/tinyplace/mod.rs`, next to the
   existing marketplace web-only note.

3. **Test** (`FeedSection.test.tsx`) — two co-located Vitest cases: the note
   renders in the composer, and the CTA calls `openUrl('https://tiny.place')`
   (mock `openUrl`). Covers the new diff lines for the ≥80% changed-line gate.

## Out of scope (YAGNI)

- Vendored SDK media fields (`PostCreate.image` / `PostMedia` / `Post.media`).
- Core `manifest.rs` post-media handling; `invokeApiClient.ts` media params.
- Any `vendor/tinyplace` submodule pointer bump.
- Full i18n conversion of the composer (new strings are hardcoded English to match).

## Follow-up to genuinely close in-app (tiny.place-first, not this repo)

1. tiny.place backend: persist + serve a post-media field.
2. tiny.place SDK: add media to `PostCreate` + read-side `Post` / `GqlPost`; release.
3. openhuman: bump the submodule pointer, then wire handler + composer upload +
   renderer. Tracked under the Tiny Place epic (#4190 / #4776).
