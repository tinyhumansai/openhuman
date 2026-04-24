# Command Palette + Global Keyboard Shortcut System — Design Spec

**Phase:** 0 (Frontend reskin foundation)
**Branch:** `feat/frontend-reskin`
**Worktree:** `~/projects/openhuman-frontend`
**Date:** 2026-04-21
**Status:** Approved, ready for implementation plan

---

## Goal

Ship a Superhuman/Linear/Raycast-style command palette (`⌘K`), a global keyboard shortcut system, and a `?` help overlay for OpenHuman. Additive keyboard layer — no existing page visuals touched, no feature flag, no new Redux slices.

## Non-goals (v1)

- Chord sequences (Vim-style `g h`) — API shape allows v2 extension without breaking changes.
- App-wide design-system semantic tokens — scoped `cmd-*` tokens only.
- Sign Out / Toggle Theme / per-page actions — owned by future feature PRs.
- i18n string externalization.
- Go Back / Go Forward shortcuts.

---

## Architecture

### Module layout

```
app/src/lib/commands/
├── types.ts                 # Action, ScopeKind, Shortcut, HotkeyBinding
├── shortcut.ts              # parseShortcut, matchEvent, formatShortcut
├── registry.ts              # singleton Map + version counter + subscribers
├── hotkeyManager.ts         # singleton capture-phase listener + scope stack
├── useHotkey.ts             # raw binding hook
├── useRegisterAction.ts     # palette-visible action hook (delegates to useHotkey)
├── globalActions.ts         # seed actions registered once at boot
└── __tests__/               # vitest

app/src/components/commands/
├── CommandProvider.tsx      # root mount: init manager, register globals, render overlays
├── CommandScope.tsx         # push/pop scope frame, provide ScopeContext
├── CommandPalette.tsx       # cmdk UI, subscribes to registry
├── HelpOverlay.tsx          # active shortcuts list, grouped
├── Kbd.tsx                  # renders one shortcut (⌘ K)
└── __tests__/
```

### Key invariants

- **Registry is a module singleton.** Components mutate only via hooks, never hold references. Reason: action lifecycle is per-mount, no persistence needed, no cross-slice dependencies.
- **Hotkey manager owns exactly one `window.addEventListener('keydown')`** in capture phase. Second `init()` is a dev-mode no-op warning.
- **Scope stack frames are keyed by `Symbol`** generated at push time; `id` is debug-only metadata. Nesting and StrictMode double-mount are robust.
- **`preventDefault()` on match; never `stopPropagation()`.** Let events bubble so `cmdk` internals and native text inputs keep working.
- **Snapshot stability via version counters.** `registry.version` bumps on register/unregister; `hotkeyManager.stackVersion` bumps on push/pop. `getActiveActions(stackSymbols)` is memoized by `(registryVersion, stackVersion)` and returns the same array reference when unchanged — required for `useSyncExternalStore` to avoid re-render churn.

### Scope model

`ScopeKind = 'global' | 'page' | 'modal'`. Stack priority: modal > page > global. Within a frame, **last-registered wins** (dispatch iterates bindings in reverse insertion order).

`<CommandScope id kind>` component:
- On mount (ref-guarded for StrictMode): `sym = hotkeyManager.pushFrame(kind, id)`.
- Provides `sym` via `ScopeContext`.
- On unmount: `hotkeyManager.popFrame(sym)` by symbol (safe if not on top, handles out-of-order unmount).

Page scope uses `<CommandScope>` wrapping page content — **not** `useLocation().key`. HashRouter keys are inconsistent and miss tabbed/drawer states that are page-like without route change.

---

## Types

```ts
export type ScopeKind = 'global' | 'page' | 'modal';
export type ShortcutString = string;  // "mod+k", "shift+mod+p", "?", "escape", "g", "f1"

export interface Action {
  id: string;                    // stable, kebab-case ("nav.go-home")
  label: string;
  hint?: string;
  group?: string;
  icon?: React.ComponentType<{ className?: string }>;
  shortcut?: ShortcutString;
  scope?: ScopeKind;             // default 'global'
  enabled?: () => boolean;
  handler: () => void | Promise<void>;
  allowInInput?: boolean;        // default false
  repeat?: boolean;              // default false
  preventDefault?: boolean;      // default true
  keywords?: string[];
}

export interface HotkeyBinding {
  shortcut: ShortcutString;
  handler: () => void;
  scope?: ScopeKind;
  enabled?: () => boolean;
  allowInInput?: boolean;
  repeat?: boolean;
  preventDefault?: boolean;
  description?: string;          // shown in help overlay if present
  id?: string;                   // dev diagnostics
}
```

---

## Shortcut parser

```ts
parseShortcut(s): { key, mod, shift, alt, ctrl }
  // Tokens lowercased, split on '+'. Last token = key, rest = modifiers.
  // `mod` = Cmd on mac, Ctrl elsewhere.
  // Memoized by string via WeakMap/Map.

matchEvent(parsed, e): boolean
  // isMac = navigator.platform.includes('Mac')
  // modPressed = isMac ? e.metaKey : e.ctrlKey
  // Single-key + letter match on e.key.toLowerCase() (shift-layer aware: '?' not '/')
  // Named keys: escape, enter, tab, space, arrow{up|down|left|right}, backspace, delete, f1–f12
  // Exact modifier match: shift/alt match e.shiftKey/e.altKey.

formatShortcut(parsed, isMac): string[]
  // mac:   ['⇧','⌘','K'] | ['⌘','K'] | ['?']
  // other: ['Shift','Ctrl','K']
```

---

## Registry API

```ts
registry.registerAction(action, scopeFrame: symbol): () => void  // dispose fn
registry.getActiveActions(scopeStack: symbol[]): RegisteredAction[]
registry.getAction(id: string): RegisteredAction | undefined    // for future Tooltip use
registry.subscribe(listener: () => void): () => void            // feeds useSyncExternalStore
registry.runAction(id: string): boolean                         // programmatic invoke; false if missing/disabled
```

Semantics:
- Registration keyed by `(id, scopeFrame)`. Same id in different frames → top-of-stack wins.
- `getActiveActions`: walks stack top→bottom, dedups by id (first occurrence wins) and by canonicalized shortcut string.
- Dev collision warnings when same shortcut is enabled in active frame twice; silent in prod.
- Version-counter memoization: same array reference returned when `(registryVersion, stackVersion)` unchanged.

---

## Hotkey manager pipeline

Capture-phase `keydown` listener. On each event:

1. Early-reject: `e.isComposing || e.keyCode === 229`.
2. Compute `inEditable` via `composedPath()` walk — input/textarea/`[contenteditable]`, shadow-DOM-safe.
3. Walk scope stack top → bottom. Within each frame, iterate bindings in **reverse insertion order** (last-registered wins).
4. For each binding: `matchEvent`? then `e.repeat && !binding.repeat` skip. Then `inEditable && !allowInInput` skip. Then `enabled?.()` skip-if-false.
5. On first match: `preventDefault()` (unless `preventDefault:false`), `try/catch` the handler (console.error on throw or rejected promise), return. **No `stopPropagation`.**

Edge cases handled explicitly:
- IME composition (`isComposing` + `keyCode===229`)
- Auto-repeat (`e.repeat`) — opt-in only
- Shadow DOM (`composedPath`)
- Tauri-killing shortcuts (`⌘R`/`⌘W`/`⌘Q`) — pass through if not registered; blocked via `preventDefault` if registered
- Handler throwing sync / rejecting promise — logged, listener survives

---

## Hook APIs

```ts
useHotkey(shortcut, handler, options?: {
  scope?, enabled?, allowInInput?, repeat?, preventDefault?, description?, id?
  // reserved for v2: sequence
}): void

useRegisterAction(action: Action): void
  // Registers into scope from ScopeContext.
  // If action.shortcut present, internally calls useHotkey (single source of truth).
```

**Stale-closure defense (handlerRef pattern):**

```ts
const handlerRef = useRef(handler);
useEffect(() => { handlerRef.current = handler; });  // every render
useEffect(() => {
  const stable = () => handlerRef.current();
  const sym = hotkeyManager.bind(frameSymbol, { ...options, handler: stable });
  return () => hotkeyManager.unbind(frameSymbol, sym);
}, [shortcut, frameSymbol, /* stable option primitives */]);
```

Handler updates every render via ref; binding only re-registers when `shortcut` or `frameSymbol` changes.

---

## Component contracts

### `<CommandProvider>` (root mount, one instance)

Responsibilities:
1. `hotkeyManager.init()` (idempotent).
2. Register seed actions from `globalActions.ts`.
3. Push root global `ScopeFrame`, provide symbol via `ScopeContext`.
4. Render `<CommandPalette>` + `<HelpOverlay>` in a Radix Portal.
5. Bind meta hotkeys: `⌘K` (open palette, `allowInInput: true`), `Esc` (close topmost overlay).
6. Dev: one-instance warning via module flag.

**Palette and help are mutually exclusive** — opening one closes the other.

### `<CommandScope id kind?>`

Push/pop scope frame by symbol; StrictMode-safe via ref guard; provides `ScopeContext`.

### `<CommandPalette>` (cmdk + Radix Dialog)

- Open state owned by `CommandProvider`.
- `useSyncExternalStore(registry.subscribe, () => registry.getActiveActions(hotkeyManager.getStackSymbols()))`.
- Rows: icon + label + `hint` subtitle + `<Kbd>` on right.
- Fuzzy search via cmdk's built-in. **Use `<Command.Item value={action.id} keywords={action.keywords}>`** — do NOT join keywords into `value` (breaks Enter-to-run-selected).
- On Enter/click: close palette → `requestAnimationFrame` → fire handler (avoid cmdk close-animation race).
- Focus trap: cmdk + Radix Dialog.
- `prefers-reduced-motion`: skip open/close transitions.
- A11y: `aria-label="Command palette"`; input `aria-label="Search commands"`.
- **Footer:** `Press ? for all shortcuts` in `cmd-foreground-muted`.

### `<HelpOverlay>` (Radix Dialog)

- `useSyncExternalStore(hotkeyManager.subscribe, () => hotkeyManager.getActiveBindings())`.
- Dedup by canonicalized shortcut string.
- Two sections: **Actions** (bindings backed by `Action`) and **Shortcuts** (bare `HotkeyBinding` with `description`).
- Within each, group by scope kind (Modal / Page / Global), alphabetical by label.
- Esc closes; `prefers-reduced-motion` respected.

### `<Kbd shortcut size?>`

Pure presentational. Parses once, memoized. Renders segments in `<kbd>` tags with mac glyphs or PC labels.

### Group ordering (pinned)

```ts
export const GROUP_ORDER = ['Navigation', 'Help'] as const;
// Unknown groups from future useRegisterAction calls append alphabetically after.
```

---

## Styling — 8 scoped `cmd-*` tokens

### Tailwind config additions

```js
extend: {
  colors: {
    'cmd-surface':          'var(--cmd-surface)',
    'cmd-surface-elevated': 'var(--cmd-surface-elevated)',
    'cmd-foreground':       'var(--cmd-foreground)',
    'cmd-foreground-muted': 'var(--cmd-foreground-muted)',
    'cmd-border':           'var(--cmd-border)',
    'cmd-ring':             'var(--cmd-ring)',
    'cmd-accent':           'var(--cmd-accent)',
    'cmd-overlay':          'var(--cmd-overlay)',
  },
  boxShadow: {
    'cmd-palette': 'var(--cmd-shadow-palette)',
  },
}
```

### CSS vars in `app/src/index.css`

```css
:root {
  --cmd-surface:          #FFFFFF;
  --cmd-surface-elevated: #F5F5F5;
  --cmd-foreground:       #171717;
  --cmd-foreground-muted: #737373;
  --cmd-border:           #E5E5E5;
  --cmd-ring:             var(--cmd-accent);
  --cmd-accent:           #2F6EF4;
  --cmd-overlay:          rgba(0, 0, 0, 0.5);
  --cmd-shadow-palette:   0 20px 25px -5px rgba(0,0,0,0.1),
                          0 10px 10px -5px rgba(0,0,0,0.04);
}

:root.dark {
  --cmd-surface:          #171717;
  --cmd-surface-elevated: #262626;
  --cmd-foreground:       #FAFAFA;
  --cmd-foreground-muted: #A3A3A3;
  --cmd-border:           #404040;
  --cmd-accent:           #60A5FA;
  --cmd-overlay:          rgba(0, 0, 0, 0.7);
  --cmd-shadow-palette:   0 20px 25px -5px rgba(0,0,0,0.5),
                          0 10px 10px -5px rgba(0,0,0,0.25);
}

@media (prefers-reduced-motion: reduce) {
  .cmd-palette-enter, .cmd-palette-exit,
  .cmd-help-enter,    .cmd-help-exit {
    animation: none !important;
    transition: none !important;
  }
}
```

### Discipline

- `components/commands/*` uses **only** `cmd-*` tokens. No raw color classes.
- Package script `lint:commands-tokens`:
  ```
  "lint:commands-tokens": "rg -n '(bg|text|border|ring|shadow)-(neutral|primary|sage|amber|canvas|stone|slate)' src/components/commands/ && exit 1 || exit 0"
  ```
  Wired into pre-push Husky hook alongside existing lint.

---

## Seed actions (v1 — six total)

### Meta hotkeys (bound directly in `CommandProvider`, NOT in registry)

| Key | Effect | allowInInput |
|-----|--------|--------------|
| `⌘K` | Open palette | yes |
| `Esc` | Close topmost overlay | — |

Rationale: "Open Command Palette" inside the palette is noise; `Esc` is meta plumbing.

### Global actions (registered at boot)

| id | label | group | shortcut | handler |
|----|-------|-------|----------|---------|
| `nav.home` | Go Home | Navigation | `mod+1` | `navigate('/home')` |
| `nav.conversations` | Go to Conversations | Navigation | `mod+2` | `navigate('/conversations')` |
| `nav.intelligence` | Go to Intelligence | Navigation | `mod+3` | `navigate('/intelligence')` |
| `nav.skills` | Go to Skills | Navigation | `mod+4` | `navigate('/skills')` |
| `nav.settings` | Open Settings | Navigation | `mod+,` | `navigate('/settings')` |
| `help.show` | Show Keyboard Shortcuts | Help | `?` | `openHelpOverlay()` |

- All `allowInInput: false`.
- `help.show` is an Action (not a raw binding) so it's palette-searchable — direct answer to "how do users discover `?`".
- Keywords per nav action: `['navigate', 'open', <page synonyms>]`.
- Icons reuse existing nav icons from `BottomTabBar`.

### `registerGlobalActions` signature

```ts
export function registerGlobalActions(
  navigate: NavigateFunction,
  openHelpOverlay: () => void,
  globalScopeSymbol: symbol,
): void
```

Called once from `<CommandProvider>` after `useNavigate()` resolves. No dynamic registration; lifetime = app lifetime.

### Shortcut platform verification (GATE 0)

**⌘1–⌘4 must be verified against Tauri/CEF before PR opens.** Stub a `useEffect` keydown listener, run `yarn tauri dev`, press each. If any are swallowed by the webview, fall back to `⌘⌥1–⌘⌥4` and update the table. If `⌘K` is eaten, hard blocker — escalate.

---

## Mount point in `App.tsx`

Pinned position (inside `HashRouter`, outside individual routes):

```tsx
<Provider store>
  <PersistGate>
    <CoreStateProvider>
      <SocketProvider>
        <ChatRuntimeProvider>
          <HashRouter>
            <CommandProvider>              {/* ← here */}
              <ServiceBlockingGate>
                <AppRoutes />
              </ServiceBlockingGate>
            </CommandProvider>
          </HashRouter>
```

One-line diff from current `App.tsx`. No Redux changes, no router changes.

---

## Test plan

### Unit (Vitest, co-located `__tests__`)

**`shortcut.test.ts`** — parser + matcher
- Parse: `mod+k`, `shift+mod+p`, `?`, `/`, `escape`, `f1`, `g`, `shift+?`
- Rejects: `""`, `"mod"`, `"mod+mod+k"`, `"meta+k"`
- mac: `mod+k` matches `metaKey+k`; non-mac: `ctrlKey+k`
- Exact modifier match: `k` does NOT match `shift+k`
- `?` matches `e.key === '?'` both platforms
- `formatShortcut` mac glyphs vs PC text

**`registry.test.ts`** — registration
- Register + `getAction`
- Duplicate id same frame → dev warn + last-wins
- Same id across frames → top-of-stack via `getActiveActions`
- `subscribe` fires on register/unregister
- `enabled: () => false` excludes from active list
- Dedup: two frames register `⌘K` → one entry
- Version counter: same `(regVer, stackVer)` returns same array ref
- **`runAction(id)`**: happy path fires + returns true; disabled returns false; unknown id returns false without throw

**`hotkeyManager.test.ts`** — pipeline
- `init()` attaches one listener; second is no-op
- Push/pop by symbol; pop non-top removes correctly
- Dispatch fires handler, calls preventDefault, NOT stopPropagation
- `isComposing` skipped; `keyCode===229` skipped
- `repeat` default skip; opt-in fires
- Input focus: without `allowInInput` suppressed (input/textarea/contenteditable); with → fires
- Scope priority: modal shadows page + global for same shortcut
- **Last-registered wins** within a frame (iterate reversed)
- Handler sync throw → console.error spy fires → listener survives
- Handler rejected promise → console.error spy fires → listener survives
- **Unregister-during-dispatch:** handler A unregisters B mid-walk → no crash, no double-fire
- **Frame pop-during-dispatch:** modal closes self on Esc → no crash

**`useHotkey.test.ts` / `useRegisterAction.test.ts`** (RTL)
- Mount registers, unmount unregisters
- StrictMode double-mount: net registration count = 1
- Handler identity: updating handler via render doesn't re-register; fresh handler runs on next keydown
- Shortcut change re-registers
- Nested `<CommandScope>` doesn't corrupt stack

**`CommandPalette.test.tsx`** (RTL)
- `⌘K` opens, `Esc` closes
- Renders active actions from `useSyncExternalStore`
- Fuzzy filter by label AND keywords (via cmdk `keywords` prop)
- Arrow keys move selection; Enter fires handler exactly once
- Live-updates when action registered while open
- Footer renders `Press ? for all shortcuts`

**`HelpOverlay.test.tsx`** (RTL)
- `?` opens, `Esc` closes
- Shows only active (not shadowed) shortcuts
- Dedup displayed shortcuts
- **Bare `HotkeyBinding` with `description`** renders in "Shortcuts" section separate from Actions
- **Shortcut normalization dedup:** `mod+k` from global + `cmd+k` from page → one entry

**`Kbd.test.tsx`**
- mac glyphs vs PC labels
- Multi-segment separators

**`globalActions.test.ts`**
- All 6 registered at boot into global frame
- `useNavigate` stability: navigate via palette after re-render still works (catches react-router regressions)

**Test utilities (`app/src/test/commandTestUtils.ts`):**
- `renderWithCommands(ui, { scopes?, actions? })` wrapper
- `pressKey(target, { key, mod, shift, alt, ... })` synthetic KeyboardEvent helper routed through capture-phase listener
- **Meta-test:** `pressKey` util asserts the event actually reaches the manager's listener. Prevents silent util breakage.

### E2E (WDIO, `app/test/e2e/specs/command-palette.spec.ts`)

- Launch at `/home`. Press `⌘K` → palette focused.
- Type `"settings"` → row highlighted.
- Press `Enter` → palette closes, hash is `#/settings`.
- Open help (`?`) → assert 6 actions listed → `Esc` closes.
- **Regression probe:** exercise one pre-existing shortcut (audit repo first; candidates: chat composer Enter-to-send, modal-close Escape) → assert still works. Prevents global listener from silently swallowing keys.

### Coverage gates
- `registry.ts`, `hotkeyManager.ts`, `shortcut.ts`: ≥95% line, ≥90% branch.
- Hooks + components: ≥80% line floor + behavior coverage.

### Not tested (explicit)
- Non-US keyboard layouts (jsdom limitation)
- Cross-Tauri-window behavior (single-window app)
- Chord sequences (v2)

---

## Implementation order (gated)

### Gate 0 — Platform verify (BLOCKS EVERYTHING)
1. Stub `useEffect` in `App.tsx` logging keydown + modifiers.
2. `yarn tauri dev`. Press `⌘1..⌘4`, `⌘,`, `?`, `⌘K`.
3. Pass: all reach listener + `preventDefault` blocks native side effect.
4. Fail on ⌘1–⌘4: fall back to `⌘⌥1..⌘⌥4`, update spec.
5. Fail on ⌘K: hard blocker, escalate.

### Gate 1 — Foundation (no UI)
types → shortcut → registry → hotkeyManager → useHotkey → useRegisterAction → CommandScope. Unit tests to ≥95% on core.

### Gate 2 — Tokens
Tailwind config + CSS vars + `lint:commands-tokens` script + pre-push wiring.

### Gate 3 — Components
Kbd → install cmdk + `@radix-ui/react-dialog` → CommandPalette → HelpOverlay → CommandProvider → globalActions.

### Gate 4 — Wire into app
One-line `App.tsx` edit; grep existing `window.addEventListener('keydown')` for conflicts.

### Gate 5 — E2E
`command-palette.spec.ts` including regression probe.

### Gate 6 — Pre-merge
- `yarn typecheck && yarn lint && yarn test:unit && yarn lint:commands-tokens` green.
- `cargo fmt --check && cargo check --manifest-path app/src-tauri/Cargo.toml` green (no drift).
- Manual smoke + a11y + reduced-motion.
- Diff audit: changes only in `lib/commands/`, `components/commands/`, `tailwind.config.js`, `index.css`, `App.tsx`, `package.json`, test files.

---

## Scope summary

**Ships:**
- 4 components (CommandProvider, CommandPalette, HelpOverlay, Kbd) + CommandScope primitive
- 7 `lib/commands/*` modules
- 8 scoped `cmd-*` semantic tokens
- 6 global actions
- ~25 unit test files + 1 E2E spec + token-lint CI script
- 2 new deps: `cmdk`, `@radix-ui/react-dialog`

**Does not ship:**
- Chord sequences (v2)
- Full design-system tokens (reskin brainstorm)
- Sign Out / Toggle Theme / per-page actions (future PRs)
- i18n
- Go Back / Forward

---

## Decisions log

| # | Decision | Alternative considered |
|---|----------|------------------------|
| 1 | Hybrid registry (static + dynamic) | Pure static; pure dynamic |
| 2 | `cmdk` library | `kbar`; hand-roll |
| 3 | Scope priority modal > page > global | Last-registered-wins global; explicit priority numbers |
| 4 | v1 single-key + modifiers only | Ship chords now |
| 5 | Contextual `?` overlay (active shortcuts only) | Full static list |
| 6 | Scoped `cmd-*` tokens only | Full semantic system now |
| 7 | `<CommandScope>` primitive, not `useLocation().key` | Route-derived scope (HashRouter brittle) |
| 8 | Separate `useHotkey` / `useRegisterAction` | Unified hook with optional palette flag |
| 9 | `handlerRef` pattern for stale closures | Re-register on every render |
| 10 | Version-counter memoized snapshots | New array per call (causes re-render churn) |
| 11 | Last-registered-wins within a frame | First-registered-wins |
| 12 | `preventDefault` on match, never `stopPropagation` | Aggressive `stopPropagation` |
| 13 | Palette and help mutually exclusive | Stacked overlays |
| 14 | Radix Dialog for overlay wrappers | Headless UI; hand-roll |
| 15 | Six seed actions (5 nav + 1 help) | More; fewer |
| 16 | Gate 0 platform verify first | Trust shortcuts work |

---

## External reviews

Second opinions from Codex (gpt-5.2-codex) and Gemini CLI (2.5) incorporated. Key contributions:
- Codex: `useSyncExternalStore`, `handlerRef` stability pattern, explicit `enabled` predicate, separate raw/action hooks.
- Gemini: drop `useLocation().key` in favor of `<CommandScope>` primitive, aggressive `preventDefault` for Tauri-killing chords, registry queryable by id for future Tooltip integration.

Divergence: Codex suggested route-derived page scope; Gemini's `<CommandScope>` argument prevailed (HashRouter quirks + non-route surfaces).
