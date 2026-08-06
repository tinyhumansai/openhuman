# Identity Marketplace Seller-Web-Only Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the seller-side dead-end in the desktop Identities → Trading tab with a note pointing users to the web flow, and record that seller-side is web-only by design.

**Architecture:** Additive UI only. A persistent info note at the bottom of `TradingTab` opens `https://tiny.place/identities` via the existing `openUrl()` helper (mirrors the `X402ConfirmDialog` `/fund` precedent). No SDK, core, or `invokeApiClient` changes — the tiny.place backend has no seller routes. Scoping recorded in two doc comments.

**Tech Stack:** React 19 + TypeScript, Vitest + Testing Library, `@tauri-apps/plugin-opener` (via `app/src/utils/openUrl.ts`), Rust doc comment.

## Global Constraints

- **No i18n:** `IdentitiesSection.tsx` is entirely hardcoded English (e.g. `"Commitment submitted."` line 854, `"Purchased {name}"` line 887). New strings are hardcoded English to match. **No locale-file edits** — do NOT add `useT()` here.
- **Web URL is a hardcoded prod constant:** `https://tiny.place/identities`. Mirrors `FUND_PAGE_URL = 'https://tiny.place/fund'` in `X402ConfirmDialog.tsx`. The tiny.place *web* frontend has no per-env base in `config.ts`.
- **External links go through `openUrl`** from `app/src/utils/openUrl.ts` — never a raw `window.open` or `<a target="_blank">` for the CTA (the `<a>` explorer links elsewhere in the file are read-only tx links, a different case).
- **Out of scope (do not touch):** `vendor/tinyplace/sdk/**`, `src/openhuman/tinyplace/manifest.rs`, `app/src/lib/agentworld/invokeApiClient.ts`, buy/bid/offer flows.
- **Run commands from repo root** unless noted. The worktree root is `.claude/worktrees/fix-4920`.

---

### Task 1: Seller "sell on tiny.place" note in the Trading tab

**Files:**
- Modify: `app/src/agentworld/pages/IdentitiesSection.tsx` (imports ~16-37; header docstring ~1-16; `TradingTab` return, insert before line 996 `</div>` that closes the outer `space-y-4`)
- Test: `app/src/agentworld/pages/IdentitiesSection.test.tsx` (add a `describe` block; add an `openUrl` mock near the top mocks)

**Interfaces:**
- Consumes: `openUrl(url: string): Promise<void>` from `../../utils/openUrl`; existing `Button` component from `../../components/ui/Button`.
- Produces: a rendered note with testid `sell-on-web` containing a CTA button testid `sell-on-web-cta` that calls `openUrl('https://tiny.place/identities')`.

- [ ] **Step 1: Add the `openUrl` mock to the existing test file**

In `app/src/agentworld/pages/IdentitiesSection.test.tsx`, directly after the existing `vi.mock('../AgentWorldShell', …)` block (ends ~line 41), add:

```ts
// External-link opener — assert the seller CTA hands off to the OS browser
// without actually invoking Tauri.
vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn() }));
```

And add its import next to the other imports at the top of the file (after the `IdentitiesSection` import line):

```ts
import { openUrl } from '../../utils/openUrl';
```

- [ ] **Step 2: Write the failing test**

Append this `describe` block to `app/src/agentworld/pages/IdentitiesSection.test.tsx` (after the last Trading-tab describe block; reuses the file's existing `gotoTab` helper, `render`, `screen`, `userEvent`):

```ts
describe('Trading tab — seller web-only note', () => {
  test('renders the seller pointer note on the Trading tab', async () => {
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    const note = await screen.findByTestId('sell-on-web');
    expect(note).toHaveTextContent(/selling a handle or responding to offers/i);
  });

  test('CTA opens the tiny.place identities page via openUrl', async () => {
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    await userEvent.click(await screen.findByTestId('sell-on-web-cta'));
    expect(vi.mocked(openUrl)).toHaveBeenCalledWith('https://tiny.place/identities');
  });
});
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `pnpm --filter openhuman-app test -- IdentitiesSection --run -t "seller web-only"`
Expected: FAIL — `Unable to find an element by: [data-testid="sell-on-web"]`.

- [ ] **Step 4: Add the `openUrl` import + web-URL constant to the component**

In `app/src/agentworld/pages/IdentitiesSection.tsx`, add the import. Import order groups deeper relative paths first, so place it directly **above** `import { apiClient } from '../AgentWorldShell';` (line 33):

```ts
import { openUrl } from '../../utils/openUrl';
```

Then, immediately after the import block (before the `// ── Types ──` comment at line 39), add the constant:

```ts
// Seller-side identity actions (list a handle for sale, accept/reject an offer)
// are web-only — the tiny.place backend exposes no seller routes and the
// vendored SDK is a buyer-side compatibility wrapper (see #4920). We point
// sellers at the web app instead. Hardcoded prod URL, matching the
// `FUND_PAGE_URL` precedent in X402ConfirmDialog.tsx.
const SELL_ON_WEB_URL = 'https://tiny.place/identities';
```

- [ ] **Step 5: Insert the note JSX in `TradingTab`**

In the same file, insert the following block immediately **before** line 996 (the `</div>` that closes the outer `<div className="space-y-4">`), i.e. after the Recent Sales `</div>` at line 995:

```tsx
      {/* Seller-side is web-only (#4920): no in-app list-for-sale / accept-offer
          path exists, so point sellers at the tiny.place web app. */}
      <div
        className="rounded-lg border border-line bg-surface-muted/40 p-3"
        data-testid="sell-on-web">
        <p className="text-xs text-content-muted">
          Selling a handle or responding to offers happens on tiny.place.
        </p>
        <Button
          variant="secondary"
          size="xs"
          className="mt-2"
          analyticsId="identities.sellOnWeb"
          onClick={() => void openUrl(SELL_ON_WEB_URL)}
          data-testid="sell-on-web-cta">
          Open tiny.place →
        </Button>
      </div>
```

- [ ] **Step 6: Update the header docstring to record the web-only scope**

In the header docstring, extend the `Write flows are live x402:` list (ends with the "Money only moves…" sentence, ~line 12-14). Add a paragraph immediately before the closing `*/` (line 15):

```text
 *
 * Seller-side actions (list a handle for sale, accept / reject an offer/bid)
 * are intentionally NOT in-app — they are web-only on tiny.place. The backend
 * exposes no seller routes and the vendored SDK is a buyer-side compatibility
 * wrapper, so the Trading tab links sellers to the web app instead (#4920).
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `pnpm --filter openhuman-app test -- IdentitiesSection --run`
Expected: PASS — the new `seller web-only` tests pass and the pre-existing IdentitiesSection tests still pass.

**E2E scope (approved exception):** no desktop WDIO E2E is added for this change. The affordance is a static info note whose CTA hands off to the OS browser via `openUrl` — there is no cross-process behavior an E2E could assert beyond the unit tests above. This is a documented exception to the standard unit-and-E2E expectation; the exception is also recorded in the design spec's Testing section and in a comment on the test's `describe` block.

- [ ] **Step 8: Typecheck, lint, and format**

Run: `pnpm typecheck && pnpm --filter openhuman-app lint -- --fix app/src/agentworld/pages/IdentitiesSection.tsx app/src/agentworld/pages/IdentitiesSection.test.tsx && pnpm --filter openhuman-app exec prettier --write app/src/agentworld/pages/IdentitiesSection.tsx app/src/agentworld/pages/IdentitiesSection.test.tsx`
Expected: no type errors, no lint errors, files formatted (import order may be auto-fixed).

- [ ] **Step 9: Commit**

```bash
git add app/src/agentworld/pages/IdentitiesSection.tsx app/src/agentworld/pages/IdentitiesSection.test.tsx
git commit -m "feat(tinyplace): point sellers to tiny.place web from Trading tab (#4920)"
```

---

### Task 2: Record web-only scope in the tinyplace core domain doc

**Files:**
- Modify: `src/openhuman/tinyplace/mod.rs` (module doc comment, after the `## Seed derivation` block ending line 26)

Docs-only; no test (Rust doc comment). Folded into a single commit.

- [ ] **Step 1: Add a scoping note to the module doc**

In `src/openhuman/tinyplace/mod.rs`, insert this doc-comment section immediately after line 26 (`//! The seed is never logged…`) and before the blank line preceding `pub(crate) mod agent;`:

```rust
//!
//! ## Marketplace scope (buyer-side only)
//!
//! The identity marketplace handlers here are **buyer-side only**: buy a
//! listing, bid, and make an offer. Seller-side actions — listing a handle for
//! sale and accepting / rejecting offers — are **web-only on tiny.place** (the
//! backend exposes no seller routes). The desktop Trading tab links sellers to
//! the web app instead. See #4920.
```

- [ ] **Step 2: Verify it compiles**

Run: `GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml`
Expected: compiles (doc comment only — no code change). Warnings unrelated to this change are fine.

- [ ] **Step 3: Format**

Run: `cargo fmt --manifest-path Cargo.toml`
Expected: no diff on the doc-comment lines (doc comments are not reflowed).

- [ ] **Step 4: Commit**

```bash
git add src/openhuman/tinyplace/mod.rs
git commit -m "docs(tinyplace): note identity marketplace is buyer-side only, selling is web-only (#4920)"
```

---

### Task 3: GitHub bookkeeping (outward-facing — CONFIRM before running)

**Not code.** Do NOT run these until the user explicitly confirms editing the epic and commenting on the issue (both are outward-facing). Until then, capture them in the PR body.

- [ ] **Step 1: Comment the resolution on #4920**

```bash
gh issue comment 4920 --repo tinyhumansai/openhuman --body "Resolved as **web-only by design**: the tiny.place backend exposes no seller routes and the vendored SDK is a buyer-side compatibility wrapper, so listing a handle / accepting-rejecting offers stays on the tiny.place web app. The desktop Trading tab now points sellers there instead of dead-ending. Scope recorded in \`IdentitiesSection.tsx\` and \`src/openhuman/tinyplace/mod.rs\`. See PR <PR_URL>."
```

- [ ] **Step 2: Mark #4776 §9 seller items N/A**

Edit the #4776 body: change the "Sell / list a handle for sale" and "Accept / reject offer" table rows' Status to `N/A (web-only — #4920)`, and change `- [ ] Sell / list flow works` to `- [x] Sell / list flow works — N/A, web-only (#4920)`. (Manual body edit via `gh issue edit 4776 --repo tinyhumansai/openhuman --body-file -` after fetching and patching the current body, to avoid clobbering other sections.)

---

## Self-Review

- **Spec coverage:** UI affordance → Task 1 (steps 4-5). Hardcoded-English constraint → Global Constraints + Task 1 (no `useT`). Docstring scoping → Task 1 step 6 + Task 2. Test → Task 1 steps 1-3, 7. GitHub bookkeeping → Task 3. Out-of-scope items → Global Constraints. All spec sections covered.
- **Placeholder scan:** `<PR_URL>` / `<PR_URL>` in Task 3 are intentional fill-at-runtime values for outward-facing text, not code placeholders. No code step contains TBD/TODO.
- **Type consistency:** testids `sell-on-web` / `sell-on-web-cta` and constant `SELL_ON_WEB_URL` used identically in test (Task 1 steps 2) and component (Task 1 steps 4-5). `openUrl` signature matches `app/src/utils/openUrl.ts`.
