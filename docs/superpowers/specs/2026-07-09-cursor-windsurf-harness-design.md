# Design: Recognize Cursor & Windsurf as first-class harnesses

> Status: **approved** — 2026-07-09. Scope locked to OpenHuman-side recognition only.

## Goal

Teach OpenHuman's orchestration engine to recognize `cursor` and `windsurf` as
harness providers, so their sessions render with a proper roster group, a brand
glyph, and a typed `harnessType` instead of falling into the generic **"Other"**
catch-all group.

This is **recognition-only**. It does not add a session-origination adapter (the
`sdk/plugin-tinyplace/adapters/*` piece that would make a real Cursor/Windsurf
CLI emit these sessions). See [Out of scope](#out-of-scope).

## Background: how harness recognition works today

A coding agent sends a `SessionEnvelopeV1` over the tiny.place Signal relay with a
`harness.provider` field (e.g. `"codex"`). OpenHuman's orchestration ingest
(`src/openhuman/orchestration/ingest.rs`) decrypts it and copies that field
**verbatim** into the session's `source` — there is no provider allowlist at
ingest (`ingest.rs:187`, `ingest.rs:213`).

The single gate that decides whether a `source` is a *known* harness is
`harness_type_for()` in `src/openhuman/orchestration/schemas.rs:276`:

```rust
fn harness_type_for(source: &str) -> Option<String> {
    matches!(source, "claude" | "codex" | "gemini").then(|| source.to_string())
}
```

It is called when building the session summary (`schemas.rs:339`) to populate the
`harnessType` field sent to the UI. An unrecognized `source` yields `None`, and
the roster drops that session into the "Other" group with no glyph.

On the frontend, `harnessType` keys three surfaces:

- `app/src/lib/orchestration/orchestrationClient.ts:24` — the `HarnessType` union.
- `app/src/components/intelligence/HarnessGlyph.tsx:20` — a `Record<GlyphKind, …>`
  mapping each harness to a `{ label, tone }` brand glyph.
- `app/src/components/intelligence/TinyPlaceRoster.tsx:23` — `HARNESS_GROUPS`, the
  ordered list of roster group headers.

Because `GLYPH` is a `Record<GlyphKind, …>`, widening `HarnessType` is
**compile-forced**: the type-checker fails until every new variant has a glyph.

## Data flow (unchanged by this change)

```
Cursor/Windsurf CLI  →  harness.provider = "cursor" | "windsurf"
  → relay → ingest.rs (source = provider, verbatim)
    → harness_type_for(source)  ← THE GATE (widened here)
      → session summary { harnessType }
        → HarnessGlyph + TinyPlaceRoster group  ← follows via the type
```

No change to ingest, pairing, attention, or the relay. Only the recognition gate
and the two UI lookup tables it feeds.

## Changes

### Rust core — `src/openhuman/orchestration/`

1. **`schemas.rs:277`** — widen the match:
   ```rust
   matches!(source, "claude" | "codex" | "gemini" | "cursor" | "windsurf")
   ```
   Refresh the `(claude/codex/gemini)` doc comments at `schemas.rs:192` and
   `schemas.rs:273` to include the new providers.

2. **`schemas.rs:1113` (test)** — extend the `harness_type_for` round-trip test:
   ```rust
   assert_eq!(harness_type_for("cursor").as_deref(),   Some("cursor"));
   assert_eq!(harness_type_for("windsurf").as_deref(), Some("windsurf"));
   ```
   The existing `None` cases (`master`/`user_created`/`orchestration`) stay.

### Frontend — `app/src/`

3. **`lib/orchestration/orchestrationClient.ts:24`**:
   ```ts
   export type HarnessType = 'claude' | 'codex' | 'gemini' | 'cursor' | 'windsurf';
   ```

4. **`components/intelligence/HarnessGlyph.tsx:20`** — add two `GLYPH` entries:
   ```ts
   cursor:   { label: 'Cu', tone: 'bg-slate-800 text-white' },
   windsurf: { label: 'Ws', tone: 'bg-teal-500 text-white' },
   ```
   Labels `Cu` / `Ws` parallel Codex's two-letter `Cx` and avoid colliding with
   Claude's `C` / Gemini's `G`. Tones: Cursor slate/near-black, Windsurf teal.
   **Exact tone classes to be confirmed against the palette classes actually
   available in the app** (the existing glyphs mix literal `bg-[#c96442]` and
   named `bg-ocean-500`/`bg-sage-500`); the values above are the intent, final
   classes chosen from what the Tailwind config exposes.

5. **`components/intelligence/TinyPlaceRoster.tsx:23`** — append to `HARNESS_GROUPS`
   (after Gemini, before the implicit "Other"):
   ```ts
   { key: 'cursor',   label: 'Cursor' },
   { key: 'windsurf', label: 'Windsurf' },
   ```

### Tests

6. **`components/intelligence/HarnessGlyph.test.tsx`** — add label cases
   `['cursor', 'Cu']`, `['windsurf', 'Ws']` to the existing parametrized table.

7. **`components/intelligence/TinyPlaceRoster.test.tsx`** — add a session with
   `harnessType: 'cursor'` (and one `'windsurf'`), assert it groups under the
   `Cursor` / `Windsurf` header and **not** under "Other".

## Testing / verification

- **Rust:** `cargo test` for the orchestration schemas module (the widened
  `harness_type_for` test).
- **Frontend:** the vitest suites for `HarnessGlyph`, `TinyPlaceRoster`, and
  `InstanceCard` (the last already exercises `harnessType`/`source`).
- **Type-check:** widening `HarnessType` is the safety net — `tsc` fails if any
  exhaustive consumer (notably the `GLYPH` `Record`) is left incomplete.

Regression posture: each new variant is asserted both in Rust (`harness_type_for`)
and in the two UI tables' tests — failing before the change, passing after.

## Out of scope

- **Session origination.** A `sdk/plugin-tinyplace/adapters/cursor.mjs` /
  `windsurf.mjs` that wraps a real Cursor/Windsurf CLI and emits
  `harness.provider = "cursor" | "windsurf"` is **not** built here. Feasibility
  depends on those agents exposing a wrappable headless CLI, which is unverified.
  Until such an adapter exists, this recognition is correct but exercised only by
  tests, not live traffic. This is an intentional, separately-scoped follow-up.
- No changes to ingest, pairing/consent, attention/unread, or the relay.

## Risks

- **Low.** The change is additive and type-checked end to end. The only judgment
  call is the glyph tone classes, which are cosmetic and confirmed against the
  available palette during implementation.
