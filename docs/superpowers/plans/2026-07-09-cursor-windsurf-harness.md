# Cursor & Windsurf Harness Recognition — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make OpenHuman's orchestration engine recognize `cursor` and `windsurf` as harness providers so their sessions render with a first-class roster group and brand glyph instead of the "Other" catch-all.

**Architecture:** Widen the single backend recognition gate (`harness_type_for()` in the Rust orchestration crate), then widen the frontend `HarnessType` union it feeds — the TypeScript type-checker forces the two UI lookup tables (`HarnessGlyph` glyph map, `TinyPlaceRoster` groups) to stay complete. No changes to ingest, pairing, relay, or attention.

**Tech Stack:** Rust (`openhuman` crate, `cargo test`), TypeScript/React (`app/`, `vitest`, `tsc`).

**Design doc:** `docs/superpowers/specs/2026-07-09-cursor-windsurf-harness-design.md`

## Global Constraints

- **Scope is recognition-only.** Do NOT create any `sdk/plugin-tinyplace/adapters/*` file or touch ingest/pairing/relay/attention. Only the recognition gate and the two UI tables it feeds.
- **Anchor edits on exact code strings, not line numbers.** This repo has concurrent activity and line numbers drift. Every edit below quotes the exact `old` string to match.
- **Glyph labels:** Cursor → `Cu`, Windsurf → `Ws` (two-letter, parallel to Codex's `Cx`).
- **Glyph tones (confirmed present in `app/tailwind.config.js`):** Cursor → `bg-slate-800 text-white`; Windsurf → `bg-teal-500 text-white`.
- **Roster order:** append Cursor then Windsurf after Gemini, before the implicit "Other" group.
- **Provider ordering in Rust match / everywhere:** `claude`, `codex`, `gemini`, `cursor`, `windsurf`.
- **Working branch:** `feat/cursor-windsurf-harness` (already created; the design-doc commit `2af56b1e0` is its tip). Do NOT commit on `main`. Never `git add -A`; stage only the exact paths listed.

---

### Task 1: Rust core — widen the recognition gate

**Files:**
- Modify: `src/openhuman/orchestration/schemas.rs` (fn `harness_type_for`, its two doc comments, and the test `harness_type_only_for_known_providers`)

**Interfaces:**
- Consumes: nothing (leaf change).
- Produces: `harness_type_for(source: &str) -> Option<String>` now returns `Some(source)` for `"cursor"` and `"windsurf"` in addition to `"claude" | "codex" | "gemini"`. This is the value that populates the `harnessType` field on the session summary consumed by the frontend (Task 2).

- [ ] **Step 1: Extend the failing test**

In `src/openhuman/orchestration/schemas.rs`, find the test `fn harness_type_only_for_known_providers()`. It contains these assertions:

```rust
        assert_eq!(harness_type_for("claude").as_deref(), Some("claude"));
        assert_eq!(harness_type_for("codex").as_deref(), Some("codex"));
        assert_eq!(harness_type_for("gemini").as_deref(), Some("gemini"));
```

Add two lines immediately after the `gemini` assertion:

```rust
        assert_eq!(harness_type_for("cursor").as_deref(), Some("cursor"));
        assert_eq!(harness_type_for("windsurf").as_deref(), Some("windsurf"));
```

Leave the existing `None` assertions (`master` / `user_created` / `orchestration`) unchanged — they guard against over-broadening.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p openhuman harness_type_only_for_known_providers`
Expected: FAIL — `assertion \`left == right\` failed` with `left: None, right: Some("cursor")` (the gate doesn't recognize `cursor` yet).

- [ ] **Step 3: Widen the gate**

In the same file, find the function body:

```rust
    matches!(source, "claude" | "codex" | "gemini").then(|| source.to_string())
```

Replace it with:

```rust
    matches!(source, "claude" | "codex" | "gemini" | "cursor" | "windsurf")
        .then(|| source.to_string())
```

- [ ] **Step 4: Refresh the doc comments (same file)**

There are two doc comments naming the provider set as `(claude/codex/gemini)`. Update both to `(claude/codex/gemini/cursor/windsurf)`:

Doc comment above the `HarnessSession`/`source` field — find:
```rust
    /// The emitting harness (claude/codex/gemini) when this is an external agent
```
Replace `(claude/codex/gemini)` → `(claude/codex/gemini/cursor/windsurf)`.

Doc comment above `fn harness_type_for` — find:
```rust
/// windows persist the emitting harness (claude/codex/gemini) in `source` (see
```
Replace `(claude/codex/gemini)` → `(claude/codex/gemini/cursor/windsurf)`.

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p openhuman harness_type_only_for_known_providers`
Expected: PASS (`test result: ok. 1 passed`).

- [ ] **Step 6: Commit**

```bash
git add src/openhuman/orchestration/schemas.rs
git commit -m "feat(orchestration): recognize cursor & windsurf harness providers"
```

---

### Task 2: Frontend — widen the harness type and its UI tables

**Files:**
- Modify: `app/src/lib/orchestration/orchestrationClient.ts` (the `HarnessType` union)
- Modify: `app/src/components/intelligence/HarnessGlyph.tsx` (the `GLYPH` record)
- Modify: `app/src/components/intelligence/TinyPlaceRoster.tsx` (the `HARNESS_GROUPS` list)
- Test: `app/src/components/intelligence/HarnessGlyph.test.tsx`
- Test: `app/src/components/intelligence/TinyPlaceRoster.test.tsx`

**Interfaces:**
- Consumes: the widened backend gate from Task 1 (a session whose `source` is `cursor`/`windsurf` now arrives with `harnessType: 'cursor' | 'windsurf'`).
- Produces: `HarnessType` union widened to `'claude' | 'codex' | 'gemini' | 'cursor' | 'windsurf'`; `GLYPH` and `HARNESS_GROUPS` gain matching entries. No new exported functions.

- [ ] **Step 1: Extend the failing tests**

In `app/src/components/intelligence/HarnessGlyph.test.tsx`, the `it.each` table is:

```tsx
  it.each<[GlyphKind, string]>([
    ['claude', 'C'],
    ['codex', 'Cx'],
    ['gemini', 'G'],
    ['openhuman', 'OH'],
  ])('renders the %s mark', (harness, label) => {
```

Add two rows after `['gemini', 'G'],`:

```tsx
    ['cursor', 'Cu'],
    ['windsurf', 'Ws'],
```

In `app/src/components/intelligence/TinyPlaceRoster.test.tsx`, add a new test after the existing `groups instances by harness ...` test (insert before the `marks the selected instance` test):

```tsx
  it('groups cursor and windsurf sessions under their own headers', () => {
    const sessions = [
      session({ sessionId: 'cu1', harnessType: 'cursor', source: 'cursor' }),
      session({ sessionId: 'ws1', harnessType: 'windsurf', source: 'windsurf' }),
    ];
    render(<TinyPlaceRoster sessions={sessions} />);
    expect(screen.getByText('Cursor')).toBeInTheDocument();
    expect(screen.getByText('Windsurf')).toBeInTheDocument();
    expect(screen.getByTestId('instance-card-cu1')).toBeInTheDocument();
    expect(screen.getByTestId('instance-card-ws1')).toBeInTheDocument();
    // Neither falls into the Other catch-all.
    expect(screen.queryByText('tinyplaceOrchestration.roster.other')).toBeNull();
  });
```

- [ ] **Step 2: Run the tests to verify they fail**

Run from the `app/` directory: `pnpm test src/components/intelligence/HarnessGlyph.test.tsx src/components/intelligence/TinyPlaceRoster.test.tsx`
Expected: FAIL — `HarnessGlyph` throws destructuring `undefined` for `GLYPH['cursor']`, and `TinyPlaceRoster` cannot find the `Cursor`/`Windsurf` headers (sessions currently land in "Other").

- [ ] **Step 3: Widen the `HarnessType` union**

In `app/src/lib/orchestration/orchestrationClient.ts`, find:

```ts
export type HarnessType = 'claude' | 'codex' | 'gemini';
```

Replace with:

```ts
export type HarnessType = 'claude' | 'codex' | 'gemini' | 'cursor' | 'windsurf';
```

- [ ] **Step 4: Add the glyph entries**

In `app/src/components/intelligence/HarnessGlyph.tsx`, find the `GLYPH` record:

```ts
const GLYPH: Record<GlyphKind, { label: string; tone: string }> = {
  claude: { label: 'C', tone: 'bg-[#c96442] text-white' },
  codex: { label: 'Cx', tone: 'bg-content text-surface' },
  gemini: { label: 'G', tone: 'bg-ocean-500 text-white' },
  openhuman: { label: 'OH', tone: 'bg-sage-500 text-white' },
};
```

Insert the two new entries after `gemini`:

```ts
  cursor: { label: 'Cu', tone: 'bg-slate-800 text-white' },
  windsurf: { label: 'Ws', tone: 'bg-teal-500 text-white' },
```

- [ ] **Step 5: Add the roster groups**

In `app/src/components/intelligence/TinyPlaceRoster.tsx`, find the `HARNESS_GROUPS` list:

```ts
const HARNESS_GROUPS: Array<{ key: HarnessType; label: string }> = [
  { key: 'claude', label: 'Claude' },
  { key: 'codex', label: 'Codex' },
  { key: 'gemini', label: 'Gemini' },
```

Add the two new groups after the `gemini` entry (before the closing `];`):

```ts
  { key: 'cursor', label: 'Cursor' },
  { key: 'windsurf', label: 'Windsurf' },
```

- [ ] **Step 6: Run the tests to verify they pass**

Run from the `app/` directory: `pnpm test src/components/intelligence/HarnessGlyph.test.tsx src/components/intelligence/TinyPlaceRoster.test.tsx`
Expected: PASS (both files green; the parametrized `HarnessGlyph` cases now include `cursor`/`windsurf`).

- [ ] **Step 7: Type-check the frontend**

Run from the `app/` directory: `pnpm tsc --noEmit` (or the project's `typecheck` script if present — check `app/package.json`).
Expected: PASS with no errors. This confirms the widened union left no exhaustive consumer incomplete.

- [ ] **Step 8: Commit**

```bash
git add app/src/lib/orchestration/orchestrationClient.ts \
        app/src/components/intelligence/HarnessGlyph.tsx \
        app/src/components/intelligence/HarnessGlyph.test.tsx \
        app/src/components/intelligence/TinyPlaceRoster.tsx \
        app/src/components/intelligence/TinyPlaceRoster.test.tsx
git commit -m "feat(orchestration): render cursor & windsurf harness glyphs and roster groups"
```

---

## Self-Review

**Spec coverage:**
- Design change 1 (widen `harness_type_for`) → Task 1 Steps 3. ✅
- Design change 2 (Rust test) → Task 1 Steps 1–2, 5. ✅
- Design doc-comment refresh → Task 1 Step 4. ✅
- Design change 3 (`HarnessType` union) → Task 2 Step 3. ✅
- Design change 4 (glyph entries, `bg-slate-800` / `bg-teal-500`) → Task 2 Step 4. ✅
- Design change 5 (roster groups) → Task 2 Step 5. ✅
- Design tests 6 (`HarnessGlyph.test.tsx`) → Task 2 Step 1. ✅
- Design tests 7 (`TinyPlaceRoster.test.tsx`) → Task 2 Step 1. ✅
- Verification (cargo test, vitest, tsc) → Task 1 Step 5, Task 2 Steps 6–7. ✅
- Out-of-scope (no adapter, no ingest/attention change) → Global Constraints. ✅

**Placeholder scan:** No TBD/TODO/"handle edge cases"; every code step shows exact strings. ✅

**Type consistency:** Provider order `claude, codex, gemini, cursor, windsurf` and labels `Cu`/`Ws` are identical across the Rust match, the TS union, the glyph map, the roster groups, and both test files. ✅
