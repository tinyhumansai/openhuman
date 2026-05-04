# Phase 0 — Command Palette + Keyboard Shortcut System

One-page summary. Full spec: [`docs/superpowers/specs/2026-04-21-command-palette-design.md`](../docs/superpowers/specs/2026-04-21-command-palette-design.md)

**Branch:** `feat/frontend-reskin` · **Worktree:** `~/projects/openhuman-frontend`

## What

Superhuman/Linear-style `⌘K` palette + global keyboard shortcut system + `?` help overlay for OpenHuman. Additive keyboard layer — no existing page visuals touched, no feature flag, no new Redux slices.

## Architecture at a glance

```
lib/commands/
├── types.ts · shortcut.ts · registry.ts (singleton)
├── hotkeyManager.ts (singleton capture-phase listener + scope stack)
├── useHotkey.ts (raw)  ·  useRegisterAction.ts (palette, delegates to useHotkey)
└── globalActions.ts

components/commands/
├── CommandProvider.tsx (root mount, one instance)
├── CommandScope.tsx (push/pop scope frame by symbol)
├── CommandPalette.tsx (cmdk + Radix Dialog)
├── HelpOverlay.tsx (Radix Dialog)
└── Kbd.tsx
```

## Non-obvious decisions (decisions log in full spec)

- **`<CommandScope>` primitive, NOT `useLocation().key`** — HashRouter is brittle, fails for tabbed/drawer surfaces.
- **Scope frames keyed by `Symbol`** — nesting + StrictMode double-mount safe.
- **Last-registered-wins** within a frame (iterate reversed at dispatch).
- **`preventDefault` on match, NEVER `stopPropagation`** — don't break cmdk or native inputs.
- **Version-counter memoized snapshots** for `useSyncExternalStore` — same array ref when unchanged.
- **`handlerRef` pattern** — handler ref updated every render, binding re-registers only on shortcut/scope change.
- **Palette and help mutually exclusive.**
- **8 scoped `cmd-*` tokens only** — not a full design system; reskin brainstorm owns that later.
- **Separate `useHotkey` / `useRegisterAction`** — prevents double-registration bug; raw vs palette-visible.

## Seed actions (v1 — six total)

| id | shortcut | group |
|---|---|---|
| `nav.home` | `mod+1` | Navigation |
| `nav.conversations` | `mod+2` | Navigation |
| `nav.intelligence` | `mod+3` | Navigation |
| `nav.skills` | `mod+4` | Navigation |
| `nav.settings` | `mod+,` | Navigation |
| `help.show` | `?` | Help |

Meta hotkeys bound directly in `CommandProvider` (not in registry): `⌘K` open palette, `Esc` close overlay.

## Gates (must pass in order)

0. **Platform verify** — stub keydown listener, confirm `⌘1–⌘4` not swallowed by Tauri/CEF. Blocks everything.
1. **Foundation** — types, shortcut, registry, hotkeyManager, hooks, `<CommandScope>`. Unit tests ≥95% on core.
2. **Tokens** — 8 `cmd-*` in tailwind + CSS vars + `lint:commands-tokens` pre-push script.
3. **Components** — Kbd, install cmdk + `@radix-ui/react-dialog`, Palette, HelpOverlay, CommandProvider, globalActions.
4. **Wire** — one-line edit to `App.tsx` (pinned mount point inside HashRouter, outside routes).
5. **E2E** — command-palette spec + regression probe on one pre-existing shortcut.
6. **Pre-merge** — typecheck, lint, unit, token-lint, e2e, cargo fmt/check, manual smoke + a11y.

## Explicit non-goals

- Chord sequences (v2)
- Full design-system semantic tokens (reskin brainstorm)
- Sign Out / Toggle Theme / per-page actions (future PRs)
- i18n
- Go Back / Forward shortcuts

## New deps

- `cmdk` (palette UI)
- `@radix-ui/react-dialog` (overlay wrapper, focus trap, a11y)
