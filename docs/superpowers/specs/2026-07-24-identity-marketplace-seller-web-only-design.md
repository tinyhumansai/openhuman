# Design: Identity marketplace seller-side is web-only (#4920)

**Date:** 2026-07-24
**Issue:** [#4920](https://github.com/tinyhumansai/openhuman/issues/4920) — *[Tiny Place] Identity marketplace is buyer-only: no list-for-sale or accept/reject-offer path*
**Epic:** #4776 §9 (Trading / Identities marketplace)

## Problem

The desktop app's identity marketplace (Agent World → Identities → Trading tab) supports
only **buyer-side** actions: browse listings, buy, bid, and make an offer. There is no path
to **sell** (list one's own `@handle`) and no path to **accept / reject** an incoming offer
or bid. A user who wants to sell opens the Trading tab and finds a dead end.

## Decision

**Seller-side actions are web-only by design.** The tiny.place backend does not expose
seller routes (create-listing, accept/reject-offer), and the vendored Rust SDK
(`vendor/tinyplace/sdk/rust/src/api/marketplace.rs`) is explicitly a *"compatibility wrapper
for marketplace endpoints used by OpenHuman"* — buyer-side only. Listing a handle and
responding to offers happens on the tiny.place web app.

This resolves #4920 not by building the full seller stack (SDK → core handler →
`invokeApiClient` → UI), but by:

1. Removing the in-app dead-end with a clear pointer to the web flow.
2. Recording the web-only scoping decision in-repo.
3. Marking the corresponding #4776 §9 items N/A.

## Scope

### In scope

1. **UI affordance** — `app/src/agentworld/pages/IdentitiesSection.tsx`, Trading tab.
   Add an informational note beneath the listings / recent-sales area:

   > ℹ️ Selling a handle or responding to offers happens on tiny.place. **[Open tiny.place →]**

   - The CTA opens `https://tiny.place/identities` via `openUrl()`
     (`app/src/utils/openUrl.ts` → `tauri-plugin-opener`), mirroring the existing
     `https://tiny.place/fund` precedent in `X402ConfirmDialog.tsx`.
   - The web URL is a hardcoded production constant, matching the `/fund` precedent
     (the tiny.place *web* frontend has no per-env base in `config.ts`; only the API
     host is env-derived).
   - Strings are **hardcoded English**, matching this file's existing convention. Unlike
     sibling agentworld pages, `IdentitiesSection.tsx` is entirely un-i18n'd (its buy/commit
     banners use literal strings, e.g. `"Commitment submitted."`, `"Purchased {name}"`), and
     this passes CI (`i18n:check` / `i18n:english:check` validate locale-file parity, not
     hardcoded JSX). Introducing a single `useT()` string here would be inconsistent; a
     full-file i18n conversion is out of scope. No locale-file changes.

2. **Scoping documentation**
   - Extend the `IdentitiesSection.tsx` header docstring to state that seller-side
     (list-for-sale, accept/reject offer) is **web-only by design**, with the reason.
   - Add a one-line pointer in the tinyplace domain module doc
     (`src/openhuman/tinyplace/mod.rs`).

3. **Test** — co-located Vitest test asserting the Trading tab renders the seller note and
   the CTA invokes `openUrl` with `https://tiny.place/identities` (mock `openUrl`). Covers
   the new diff lines for the ≥80% changed-line coverage gate.

4. **GitHub bookkeeping** (not code; done via `gh`, noted in the PR body)
   - #4776 §9: mark "Sell / list a handle for sale", "Accept / reject offer", and the
     "Sell / list flow works" checkbox as N/A, referencing #4920's resolution.
   - Comment the web-only resolution on #4920.

### Out of scope (YAGNI)

- Vendored SDK seller methods (`create_listing` / `accept_offer` / `reject_offer`).
- Core `handle_tinyplace_marketplace_*` seller handlers.
- `invokeApiClient.ts` seller methods.
- Full i18n conversion of `IdentitiesSection.tsx` (the new strings are hardcoded English,
  matching the file).

## Components & data flow

```text
Trading tab (IdentitiesSection.tsx)
  └─ SellOnWebNote  ──click──▶  openUrl('https://tiny.place/identities')
                                   └─ tauri-plugin-opener → OS default browser
```

No core RPC, no network from the app itself — the CTA hands off to the OS browser.

## Error handling

`openUrl()` already handles the CEF IPC-bridge gap by falling back to `window.open` for
http(s) URLs and logs a low-PII telemetry breadcrumb. No new error surface is introduced.

## Testing

- **Unit (Vitest):** render `TradingTab`, assert the seller note text is present, that the
  CTA carries the `identities.sellOnWeb` analytics id, and that clicking it calls the mocked
  `openUrl` with the expected URL.
- No new Rust behavior → no Rust test changes.
- **E2E: intentionally skipped.** The affordance is a static info note whose CTA hands off to
  the OS browser via `openUrl` — there is no cross-process behavior a desktop WDIO E2E could
  assert beyond the unit tests. Documented as an approved exception to the standard
  unit-and-E2E expectation for this change.

## Rollback / risk

Low risk: additive UI note + docstring. No behavior change to existing buy/bid/offer flows,
and no i18n/locale files touched (strings are hardcoded English, matching the file). Revert
is a clean removal of the note block plus its docstring note and the two unit tests.
