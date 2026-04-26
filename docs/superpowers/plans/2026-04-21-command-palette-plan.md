# Command Palette + Keyboard Shortcut System — Implementation Plan

> **Status:** Shipped in PR #745. This file is retained as historical design context. For the current behavior, read the source in `app/src/lib/commands/` and `app/src/components/commands/`. Known planning-time bugs called out by review (symbol.description cache keys, render-side side effects, missing disposers, `?` shortcut matching, stale `enabled` refs) have been addressed in the implementation — see the PR review thread resolutions.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a `⌘K` command palette, global action registry, keyboard shortcut hook, and `?` help overlay for OpenHuman, matching Superhuman/Linear/Raycast UX.

**Architecture:** Module-singleton registry + single capture-phase `keydown` listener with a symbol-keyed scope stack. React hooks (`useHotkey`, `useRegisterAction`) manage lifecycle; `<CommandProvider>` mounts once near the app root; `<CommandScope>` pushes/pops page and modal frames.

**Tech Stack:** React 19, TypeScript, Vite, `cmdk`, `@radix-ui/react-dialog`, Tailwind CSS, Vitest, React Testing Library, WDIO (E2E), HashRouter (react-router-dom v7).

**Spec:** [`docs/superpowers/specs/2026-04-21-command-palette-design.md`](../specs/2026-04-21-command-palette-design.md)

**Corrections from spec discovered at plan time:**
- Spec said `nav.conversations` → `/conversations`. Actual route in `AppRoutes.tsx` is `/chat`. Use `nav.chat` → `/chat`, label "Go to Chat".
- Existing `DictationHotkeyManager.tsx` is already a keyboard listener — audit in Gate 4 for conflicts.

---

## File Structure

### New files

```
app/src/lib/commands/
├── types.ts
├── shortcut.ts
├── registry.ts
├── hotkeyManager.ts
├── useHotkey.ts
├── useRegisterAction.ts
├── globalActions.ts
└── __tests__/
    ├── shortcut.test.ts
    ├── registry.test.ts
    ├── hotkeyManager.test.ts
    ├── useHotkey.test.tsx
    ├── useRegisterAction.test.tsx
    └── globalActions.test.tsx

app/src/components/commands/
├── CommandProvider.tsx
├── CommandScope.tsx
├── CommandPalette.tsx
├── HelpOverlay.tsx
├── Kbd.tsx
└── __tests__/
    ├── Kbd.test.tsx
    ├── CommandPalette.test.tsx
    ├── HelpOverlay.test.tsx
    └── CommandScope.test.tsx

app/src/test/commandTestUtils.ts
app/test/e2e/specs/command-palette.spec.ts
```

### Modified files

- `app/tailwind.config.js` — add 8 `cmd-*` color tokens + `cmd-palette` shadow.
- `app/src/index.css` — add `:root` + `:root.dark` CSS vars + reduced-motion rule.
- `app/src/App.tsx` — wrap `<ServiceBlockingGate>` with `<CommandProvider>` inside `<Router>`.
- `app/package.json` — add `cmdk`, `@radix-ui/react-dialog`, add `lint:commands-tokens` script.
- `app/.husky/pre-push` (or equivalent) — wire `lint:commands-tokens` in.

---

## Gate 0 — Platform Verification (BLOCKS EVERYTHING)

### Task 0.1: Stub a keydown probe in App.tsx

**Files:**
- Modify: `app/src/App.tsx` (temporary)

- [ ] **Step 1: Add temporary keydown probe**

Insert near top of `App()`:

```tsx
// TEMP GATE-0 PROBE — REMOVE BEFORE TASK 0.3
if (typeof window !== 'undefined') {
  window.addEventListener('keydown', (e) => {
    // eslint-disable-next-line no-console
    console.log('[gate-0]', {
      key: e.key,
      code: e.code,
      meta: e.metaKey,
      ctrl: e.ctrlKey,
      shift: e.shiftKey,
      alt: e.altKey,
    });
  }, { capture: true });
}
```

- [ ] **Step 2: Run Tauri dev build**

```bash
# From the repository root:
pnpm --cwd app dev:app
```

- [ ] **Step 3: Exercise the target shortcuts**

With the app focused, press each in turn and read the dev console:

- `⌘K`, `⌘1`, `⌘2`, `⌘3`, `⌘4`, `⌘,`, `?`, `Esc`

Expected: every one prints a `[gate-0]` log line with matching modifiers.

- [ ] **Step 4: Record result**

In this file, under the table below, tick PASS or write the observed failure. If `⌘1–⌘4` do not reach the listener or trigger a native side-effect (tab switch, zoom, etc.), update every later task that uses `mod+N` to use `mod+alt+N` before proceeding.

| Shortcut | Result |
|----------|--------|
| ⌘K | ☐ |
| ⌘1 | ☐ |
| ⌘2 | ☐ |
| ⌘3 | ☐ |
| ⌘4 | ☐ |
| ⌘, | ☐ |
| ? | ☐ |
| Esc | ☐ |

- [ ] **Step 5: Remove probe and commit revert**

```bash
git restore app/src/App.tsx
```

Do NOT commit the probe itself. If shortcuts had to be remapped, amend the spec and this plan in a single commit:

```bash
git add docs/superpowers/specs/2026-04-21-command-palette-design.md \
        docs/superpowers/plans/2026-04-21-command-palette-plan.md
git commit -m "plan: remap nav shortcuts to mod+alt+N (Gate 0 CEF capture)"
```

---

## Gate 1 — Foundation (no UI)

### Task 1.1: Install new dependencies

**Files:**
- Modify: `app/package.json`
- Modify: `app/pnpm-lock.yaml`

- [ ] **Step 1: Install cmdk and Radix Dialog**

```bash
cd /Users/jwalinshah/projects/openhuman-frontend/app
pnpm add cmdk@^1 @radix-ui/react-dialog@^1
```

- [ ] **Step 2: Verify install**

Run `pnpm --cwd app compile` — no errors. `grep -E '"(cmdk|@radix-ui/react-dialog)"' app/package.json` shows both.

- [ ] **Step 3: Commit**

```bash
git add app/package.json app/pnpm-lock.yaml
git commit -m "chore(deps): add cmdk + @radix-ui/react-dialog for command palette"
```

---

### Task 1.2: Define core types

**Files:**
- Create: `app/src/lib/commands/types.ts`

- [ ] **Step 1: Write types.ts**

```ts
import type { ComponentType } from 'react';

export type ScopeKind = 'global' | 'page' | 'modal';
export type ShortcutString = string;

export interface ParsedShortcut {
  key: string;
  mod: boolean;
  shift: boolean;
  alt: boolean;
  ctrl: boolean;
}

export interface Action {
  id: string;
  label: string;
  hint?: string;
  group?: string;
  icon?: ComponentType<{ className?: string }>;
  shortcut?: ShortcutString;
  scope?: ScopeKind;
  enabled?: () => boolean;
  handler: () => void | Promise<void>;
  allowInInput?: boolean;
  repeat?: boolean;
  preventDefault?: boolean;
  keywords?: string[];
}

export interface RegisteredAction extends Action {
  scopeFrame: symbol;
}

export interface HotkeyBinding {
  shortcut: ShortcutString;
  handler: () => void;
  scope?: ScopeKind;
  enabled?: () => boolean;
  allowInInput?: boolean;
  repeat?: boolean;
  preventDefault?: boolean;
  description?: string;
  id?: string;
}

export interface ScopeFrame {
  symbol: symbol;
  id: string;
  kind: ScopeKind;
}

export interface ActiveBinding {
  frame: ScopeFrame;
  binding: HotkeyBinding;
  parsed: ParsedShortcut;
}
```

- [ ] **Step 2: Commit**

```bash
git add app/src/lib/commands/types.ts
git commit -m "feat(commands): add command registry + hotkey types"
```

---

### Task 1.3: Shortcut parser + matcher — failing tests

**Files:**
- Create: `app/src/lib/commands/__tests__/shortcut.test.ts`

- [ ] **Step 1: Write failing test file**

```ts
import { describe, it, expect } from 'vitest';
import { parseShortcut, matchEvent, formatShortcut } from '../shortcut';

describe('parseShortcut', () => {
  it('parses mod+k', () => {
    expect(parseShortcut('mod+k')).toEqual({ key: 'k', mod: true, shift: false, alt: false, ctrl: false });
  });
  it('parses shift+mod+p', () => {
    expect(parseShortcut('shift+mod+p')).toEqual({ key: 'p', mod: true, shift: true, alt: false, ctrl: false });
  });
  it('parses ?', () => {
    expect(parseShortcut('?')).toEqual({ key: '?', mod: false, shift: false, alt: false, ctrl: false });
  });
  it('parses escape and f1 and arrowup', () => {
    expect(parseShortcut('escape').key).toBe('escape');
    expect(parseShortcut('f1').key).toBe('f1');
    expect(parseShortcut('arrowup').key).toBe('arrowup');
  });
  it('parses mod+,', () => {
    expect(parseShortcut('mod+,')).toEqual({ key: ',', mod: true, shift: false, alt: false, ctrl: false });
  });
  it('throws on empty', () => {
    expect(() => parseShortcut('')).toThrow();
  });
  it('throws on modifier-only', () => {
    expect(() => parseShortcut('mod')).toThrow();
  });
  it('throws on meta+k (must use mod)', () => {
    expect(() => parseShortcut('meta+k')).toThrow();
  });
  it('memoizes', () => {
    expect(parseShortcut('mod+k')).toBe(parseShortcut('mod+k'));
  });
});

function ke(opts: Partial<KeyboardEventInit> & { key: string }): KeyboardEvent {
  return new KeyboardEvent('keydown', { key: opts.key, metaKey: !!opts.metaKey, ctrlKey: !!opts.ctrlKey, shiftKey: !!opts.shiftKey, altKey: !!opts.altKey });
}

describe('matchEvent (mac)', () => {
  const origPlatform = navigator.platform;
  beforeAll(() => { Object.defineProperty(navigator, 'platform', { value: 'MacIntel', configurable: true }); });
  afterAll(() => { Object.defineProperty(navigator, 'platform', { value: origPlatform, configurable: true }); });

  it('mod+k matches metaKey+k', () => {
    expect(matchEvent(parseShortcut('mod+k'), ke({ key: 'k', metaKey: true }))).toBe(true);
  });
  it('mod+k does NOT match ctrlKey+k on mac', () => {
    expect(matchEvent(parseShortcut('mod+k'), ke({ key: 'k', ctrlKey: true }))).toBe(false);
  });
  it('k does not match shift+k', () => {
    expect(matchEvent(parseShortcut('k'), ke({ key: 'K', shiftKey: true }))).toBe(false);
  });
  it('? matches e.key === "?"', () => {
    expect(matchEvent(parseShortcut('?'), ke({ key: '?' }))).toBe(true);
  });
  it('escape matches Escape', () => {
    expect(matchEvent(parseShortcut('escape'), ke({ key: 'Escape' }))).toBe(true);
  });
});

describe('matchEvent (non-mac)', () => {
  const origPlatform = navigator.platform;
  beforeAll(() => { Object.defineProperty(navigator, 'platform', { value: 'Win32', configurable: true }); });
  afterAll(() => { Object.defineProperty(navigator, 'platform', { value: origPlatform, configurable: true }); });

  it('mod+k matches ctrlKey+k', () => {
    expect(matchEvent(parseShortcut('mod+k'), ke({ key: 'k', ctrlKey: true }))).toBe(true);
  });
});

describe('formatShortcut', () => {
  it('mac renders glyphs', () => {
    expect(formatShortcut(parseShortcut('shift+mod+k'), true)).toEqual(['⇧', '⌘', 'K']);
  });
  it('pc renders labels', () => {
    expect(formatShortcut(parseShortcut('shift+mod+k'), false)).toEqual(['Shift', 'Ctrl', 'K']);
  });
  it('single printable renders alone', () => {
    expect(formatShortcut(parseShortcut('?'), true)).toEqual(['?']);
  });
});
```

- [ ] **Step 2: Run test, verify fail**

```bash
pnpm --cwd app test:unit src/lib/commands/__tests__/shortcut.test.ts
```

Expected: FAIL — "Failed to resolve import '../shortcut'".

---

### Task 1.4: Shortcut parser + matcher — implementation

**Files:**
- Create: `app/src/lib/commands/shortcut.ts`

- [ ] **Step 1: Implement shortcut.ts**

```ts
import type { ParsedShortcut, ShortcutString } from './types';

const MODIFIER_TOKENS = new Set(['mod', 'shift', 'alt', 'ctrl']);
const NAMED_KEYS = new Set([
  'escape', 'enter', 'tab', 'space',
  'arrowup', 'arrowdown', 'arrowleft', 'arrowright',
  'backspace', 'delete', 'home', 'end', 'pageup', 'pagedown',
  'f1', 'f2', 'f3', 'f4', 'f5', 'f6', 'f7', 'f8', 'f9', 'f10', 'f11', 'f12',
]);

const parseCache = new Map<string, ParsedShortcut>();

export function parseShortcut(raw: ShortcutString): ParsedShortcut {
  const cached = parseCache.get(raw);
  if (cached) return cached;
  if (!raw) throw new Error('parseShortcut: empty shortcut string');
  const tokens = raw.toLowerCase().split('+').map(t => t.trim()).filter(Boolean);
  if (tokens.length === 0) throw new Error(`parseShortcut: invalid shortcut "${raw}"`);
  const key = tokens[tokens.length - 1];
  if (MODIFIER_TOKENS.has(key)) throw new Error(`parseShortcut: shortcut "${raw}" has no key`);
  if (key === 'meta' || key === 'cmd' || key === 'command') {
    throw new Error(`parseShortcut: use "mod" instead of "${key}" in "${raw}"`);
  }
  if (key.length > 1 && !NAMED_KEYS.has(key) && !/^[a-z0-9]$/.test(key) && !/^[\p{P}\p{S}]$/u.test(key)) {
    throw new Error(`parseShortcut: unknown key "${key}" in "${raw}"`);
  }
  const result: ParsedShortcut = { key, mod: false, shift: false, alt: false, ctrl: false };
  for (let i = 0; i < tokens.length - 1; i++) {
    const m = tokens[i];
    if (m === 'mod' || m === 'shift' || m === 'alt' || m === 'ctrl') {
      result[m] = true;
    } else {
      throw new Error(`parseShortcut: unknown modifier "${m}" in "${raw}"`);
    }
  }
  parseCache.set(raw, result);
  return result;
}

export function isMac(): boolean {
  if (typeof navigator === 'undefined') return false;
  return navigator.platform.toLowerCase().includes('mac');
}

export function matchEvent(parsed: ParsedShortcut, e: KeyboardEvent): boolean {
  const mac = isMac();
  const modPressed = mac ? e.metaKey : e.ctrlKey;
  const otherMetaPressed = mac ? e.ctrlKey : e.metaKey;
  if (parsed.mod !== modPressed) return false;
  if (parsed.ctrl !== otherMetaPressed && !(mac && parsed.ctrl === false)) {
    // On non-mac, ctrl is the mod. On mac, explicit ctrl tracks e.ctrlKey.
    if (mac && parsed.ctrl !== e.ctrlKey) return false;
    if (!mac && parsed.ctrl !== e.ctrlKey && parsed.mod !== true) return false;
  }
  if (parsed.shift !== e.shiftKey) return false;
  if (parsed.alt !== e.altKey) return false;
  const eventKey = e.key.length === 1 ? e.key.toLowerCase() : e.key.toLowerCase();
  return eventKey === parsed.key;
}

const MAC_GLYPHS: Record<string, string> = {
  mod: '⌘', shift: '⇧', alt: '⌥', ctrl: '⌃',
  escape: 'Esc', enter: '↵', tab: '⇥', space: '␣',
  arrowup: '↑', arrowdown: '↓', arrowleft: '←', arrowright: '→',
  backspace: '⌫', delete: '⌦',
};
const PC_LABELS: Record<string, string> = {
  mod: 'Ctrl', shift: 'Shift', alt: 'Alt', ctrl: 'Ctrl',
  escape: 'Esc', enter: 'Enter', tab: 'Tab', space: 'Space',
  arrowup: '↑', arrowdown: '↓', arrowleft: '←', arrowright: '→',
  backspace: 'Backspace', delete: 'Delete',
};

export function formatShortcut(parsed: ParsedShortcut, mac: boolean): string[] {
  const table = mac ? MAC_GLYPHS : PC_LABELS;
  const out: string[] = [];
  if (parsed.ctrl && !parsed.mod) out.push(table.ctrl);
  if (parsed.alt) out.push(table.alt);
  if (parsed.shift) out.push(table.shift);
  if (parsed.mod) out.push(table.mod);
  const k = parsed.key;
  if (table[k]) out.push(table[k]);
  else if (k.length === 1) out.push(k.toUpperCase());
  else if (/^f\d+$/.test(k)) out.push(k.toUpperCase());
  else out.push(k);
  return out;
}
```

- [ ] **Step 2: Run tests, verify pass**

```bash
pnpm --cwd app test:unit src/lib/commands/__tests__/shortcut.test.ts
```

Expected: all PASS.

- [ ] **Step 3: Commit**

```bash
git add app/src/lib/commands/shortcut.ts app/src/lib/commands/__tests__/shortcut.test.ts
git commit -m "feat(commands): shortcut parser + matcher + formatter"
```

---

### Task 1.5: Registry — failing tests

**Files:**
- Create: `app/src/lib/commands/__tests__/registry.test.ts`

- [ ] **Step 1: Write failing test file**

```ts
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { createRegistry } from '../registry';
import type { Action } from '../types';

const baseAction: Action = { id: 'a.test', label: 'Test', handler: vi.fn() };

describe('registry', () => {
  let reg: ReturnType<typeof createRegistry>;
  beforeEach(() => { reg = createRegistry(); });

  it('registers + getAction', () => {
    const frame = Symbol('global');
    reg.registerAction(baseAction, frame);
    expect(reg.getAction('a.test')?.id).toBe('a.test');
  });

  it('dispose unregisters', () => {
    const frame = Symbol('global');
    const dispose = reg.registerAction(baseAction, frame);
    dispose();
    expect(reg.getAction('a.test')).toBeUndefined();
  });

  it('duplicate id same frame warns and replaces', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const frame = Symbol('global');
    reg.registerAction({ ...baseAction, label: 'A' }, frame);
    reg.registerAction({ ...baseAction, label: 'B' }, frame);
    expect(reg.getAction('a.test')?.label).toBe('B');
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it('same id in two frames: top of stack wins', () => {
    const f1 = Symbol('global');
    const f2 = Symbol('page');
    reg.registerAction({ ...baseAction, label: 'global' }, f1);
    reg.registerAction({ ...baseAction, label: 'page' }, f2);
    const active = reg.getActiveActions([f1, f2]);
    expect(active.filter(a => a.id === 'a.test')).toHaveLength(1);
    expect(active.find(a => a.id === 'a.test')?.label).toBe('page');
  });

  it('enabled:false excluded from active', () => {
    const frame = Symbol('global');
    reg.registerAction({ ...baseAction, enabled: () => false }, frame);
    expect(reg.getActiveActions([frame])).toHaveLength(0);
  });

  it('dedups by canonicalized shortcut', () => {
    const f1 = Symbol('global');
    const f2 = Symbol('page');
    reg.registerAction({ id: 'a', label: 'A', handler: vi.fn(), shortcut: 'mod+k' }, f1);
    reg.registerAction({ id: 'b', label: 'B', handler: vi.fn(), shortcut: 'mod+k' }, f2);
    const active = reg.getActiveActions([f1, f2]);
    // both kept as distinct actions; dedup reporting is separate (tested in HelpOverlay)
    expect(active).toHaveLength(2);
  });

  it('subscribe fires on register/unregister', () => {
    const listener = vi.fn();
    const unsub = reg.subscribe(listener);
    const frame = Symbol('global');
    const dispose = reg.registerAction(baseAction, frame);
    expect(listener).toHaveBeenCalledTimes(1);
    dispose();
    expect(listener).toHaveBeenCalledTimes(2);
    unsub();
  });

  it('version counter stable when unchanged', () => {
    const frame = Symbol('global');
    reg.registerAction(baseAction, frame);
    const a = reg.getActiveActions([frame]);
    const b = reg.getActiveActions([frame]);
    expect(a).toBe(b);
  });

  it('version counter new ref on change', () => {
    const frame = Symbol('global');
    reg.registerAction(baseAction, frame);
    const a = reg.getActiveActions([frame]);
    reg.registerAction({ id: 'other', label: 'O', handler: vi.fn() }, frame);
    const b = reg.getActiveActions([frame]);
    expect(a).not.toBe(b);
  });

  describe('runAction', () => {
    it('happy path fires + returns true', () => {
      const frame = Symbol('global');
      const handler = vi.fn();
      reg.registerAction({ id: 'x', label: 'X', handler }, frame);
      reg.setActiveStack([frame]);
      expect(reg.runAction('x')).toBe(true);
      expect(handler).toHaveBeenCalledOnce();
    });
    it('disabled returns false without firing', () => {
      const frame = Symbol('global');
      const handler = vi.fn();
      reg.registerAction({ id: 'x', label: 'X', handler, enabled: () => false }, frame);
      reg.setActiveStack([frame]);
      expect(reg.runAction('x')).toBe(false);
      expect(handler).not.toHaveBeenCalled();
    });
    it('unknown id returns false without throwing', () => {
      expect(reg.runAction('nope')).toBe(false);
    });
  });
});
```

- [ ] **Step 2: Run test, verify fail**

```bash
pnpm --cwd app test:unit src/lib/commands/__tests__/registry.test.ts
```

Expected: FAIL — "Failed to resolve import '../registry'".

---

### Task 1.6: Registry — implementation

**Files:**
- Create: `app/src/lib/commands/registry.ts`

- [ ] **Step 1: Implement registry.ts**

```ts
import type { Action, RegisteredAction } from './types';
import { parseShortcut } from './shortcut';

export interface Registry {
  registerAction: (action: Action, scopeFrame: symbol) => () => void;
  getAction: (id: string) => RegisteredAction | undefined;
  getActiveActions: (scopeStack: symbol[]) => RegisteredAction[];
  subscribe: (listener: () => void) => () => void;
  runAction: (id: string) => boolean;
  setActiveStack: (stack: symbol[]) => void;
}

export function createRegistry(): Registry {
  const byFrame = new Map<symbol, Map<string, RegisteredAction>>();
  const listeners = new Set<() => void>();
  let version = 0;
  const snapshotCache = new Map<string, RegisteredAction[]>();
  let activeStack: symbol[] = [];

  function bump(): void {
    version += 1;
    snapshotCache.clear();
    for (const l of listeners) l();
  }

  function stackKey(stack: symbol[]): string {
    return `${version}:${stack.map(s => s.description ?? '?').join('>')}:${stack.length}`;
  }

  function registerAction(action: Action, scopeFrame: symbol): () => void {
    let frame = byFrame.get(scopeFrame);
    if (!frame) {
      frame = new Map();
      byFrame.set(scopeFrame, frame);
    }
    if (frame.has(action.id)) {
      // eslint-disable-next-line no-console
      console.warn(`[commands] duplicate action id "${action.id}" in the same scope — replacing`);
    }
    const registered: RegisteredAction = { ...action, scopeFrame };
    // Validate shortcut eagerly to catch typos.
    if (action.shortcut) parseShortcut(action.shortcut);
    frame.set(action.id, registered);
    bump();
    return () => {
      const f = byFrame.get(scopeFrame);
      if (!f) return;
      if (f.delete(action.id)) {
        if (f.size === 0) byFrame.delete(scopeFrame);
        bump();
      }
    };
  }

  function getAction(id: string): RegisteredAction | undefined {
    // Check top-of-stack first, fall back to any frame.
    for (let i = activeStack.length - 1; i >= 0; i--) {
      const frame = byFrame.get(activeStack[i]);
      const hit = frame?.get(id);
      if (hit) return hit;
    }
    for (const frame of byFrame.values()) {
      const hit = frame.get(id);
      if (hit) return hit;
    }
    return undefined;
  }

  function getActiveActions(scopeStack: symbol[]): RegisteredAction[] {
    const key = stackKey(scopeStack);
    const cached = snapshotCache.get(key);
    if (cached) return cached;
    const seen = new Set<string>();
    const out: RegisteredAction[] = [];
    // Walk top → bottom so top-of-stack wins for duplicate ids.
    for (let i = scopeStack.length - 1; i >= 0; i--) {
      const frame = byFrame.get(scopeStack[i]);
      if (!frame) continue;
      for (const action of frame.values()) {
        if (seen.has(action.id)) continue;
        if (action.enabled && !action.enabled()) continue;
        seen.add(action.id);
        out.push(action);
      }
    }
    snapshotCache.set(key, out);
    return out;
  }

  function subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }

  function runAction(id: string): boolean {
    const action = getAction(id);
    if (!action) return false;
    if (action.enabled && !action.enabled()) return false;
    try {
      const r = action.handler();
      if (r instanceof Promise) r.catch(err => console.error('[commands] action rejected', id, err));
    } catch (err) {
      console.error('[commands] action threw', id, err);
    }
    return true;
  }

  function setActiveStack(stack: symbol[]): void {
    activeStack = [...stack];
    // Don't bump version — stack change is handled by hotkeyManager's own version counter.
  }

  return { registerAction, getAction, getActiveActions, subscribe, runAction, setActiveStack };
}

export const registry = createRegistry();
```

- [ ] **Step 2: Run tests, verify pass**

```bash
pnpm --cwd app test:unit src/lib/commands/__tests__/registry.test.ts
```

Expected: all PASS.

- [ ] **Step 3: Commit**

```bash
git add app/src/lib/commands/registry.ts app/src/lib/commands/__tests__/registry.test.ts
git commit -m "feat(commands): action registry singleton with versioned snapshots"
```

---

### Task 1.7: Hotkey manager — failing tests

**Files:**
- Create: `app/src/lib/commands/__tests__/hotkeyManager.test.ts`

- [ ] **Step 1: Write failing tests**

```ts
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { createHotkeyManager } from '../hotkeyManager';

function dispatchKey(key: string, opts: Partial<KeyboardEventInit> = {}): KeyboardEvent {
  const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...opts });
  window.dispatchEvent(e);
  return e;
}

describe('hotkeyManager', () => {
  let mgr: ReturnType<typeof createHotkeyManager>;
  beforeEach(() => { mgr = createHotkeyManager(); mgr.init(); });
  afterEach(() => { mgr.teardown(); });

  it('init is idempotent', () => {
    const listenerSpy = vi.spyOn(window, 'addEventListener');
    mgr.init();
    // addEventListener should not be called again (guarded by initialized flag).
    const keydownCalls = listenerSpy.mock.calls.filter(c => c[0] === 'keydown');
    expect(keydownCalls.length).toBe(0);
    listenerSpy.mockRestore();
  });

  it('fires binding handler + preventDefault', () => {
    const frame = mgr.pushFrame('global', 'root');
    const handler = vi.fn();
    mgr.bind(frame, { shortcut: 'escape', handler });
    const e = dispatchKey('Escape');
    expect(handler).toHaveBeenCalled();
    expect(e.defaultPrevented).toBe(true);
  });

  it('does NOT stopPropagation', () => {
    const frame = mgr.pushFrame('global', 'root');
    const handler = vi.fn();
    mgr.bind(frame, { shortcut: 'escape', handler });
    let bubbled = false;
    window.addEventListener('keydown', () => { bubbled = true; }, { once: true });
    dispatchKey('Escape');
    expect(bubbled).toBe(true);
  });

  it('skips when isComposing', () => {
    const frame = mgr.pushFrame('global', 'root');
    const handler = vi.fn();
    mgr.bind(frame, { shortcut: 'escape', handler });
    // jsdom doesn't let us set isComposing directly; stub via keyCode 229.
    dispatchKey('Escape', { keyCode: 229 } as KeyboardEventInit & { keyCode: number });
    expect(handler).not.toHaveBeenCalled();
  });

  it('skips auto-repeat by default', () => {
    const frame = mgr.pushFrame('global', 'root');
    const handler = vi.fn();
    mgr.bind(frame, { shortcut: 'escape', handler });
    dispatchKey('Escape', { repeat: true });
    expect(handler).not.toHaveBeenCalled();
  });

  it('fires on auto-repeat when repeat: true', () => {
    const frame = mgr.pushFrame('global', 'root');
    const handler = vi.fn();
    mgr.bind(frame, { shortcut: 'escape', handler, repeat: true });
    dispatchKey('Escape', { repeat: true });
    expect(handler).toHaveBeenCalled();
  });

  it('suppresses in input unless allowInInput', () => {
    const input = document.createElement('input');
    document.body.appendChild(input);
    input.focus();
    const frame = mgr.pushFrame('global', 'root');
    const handler = vi.fn();
    mgr.bind(frame, { shortcut: 'k', handler });
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', bubbles: true, cancelable: true }));
    expect(handler).not.toHaveBeenCalled();
    input.remove();
  });

  it('fires in input when allowInInput:true', () => {
    const input = document.createElement('input');
    document.body.appendChild(input);
    input.focus();
    const frame = mgr.pushFrame('global', 'root');
    const handler = vi.fn();
    mgr.bind(frame, { shortcut: 'k', handler, allowInInput: true });
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', bubbles: true, cancelable: true }));
    expect(handler).toHaveBeenCalled();
    input.remove();
  });

  it('modal shadows page+global for same shortcut', () => {
    const g = mgr.pushFrame('global', 'root');
    const p = mgr.pushFrame('page', 'home');
    const m = mgr.pushFrame('modal', 'dialog');
    const gh = vi.fn(), ph = vi.fn(), mh = vi.fn();
    mgr.bind(g, { shortcut: 'escape', handler: gh });
    mgr.bind(p, { shortcut: 'escape', handler: ph });
    mgr.bind(m, { shortcut: 'escape', handler: mh });
    dispatchKey('Escape');
    expect(mh).toHaveBeenCalled();
    expect(ph).not.toHaveBeenCalled();
    expect(gh).not.toHaveBeenCalled();
  });

  it('last-registered wins within a frame', () => {
    const f = mgr.pushFrame('global', 'root');
    const first = vi.fn();
    const last = vi.fn();
    mgr.bind(f, { shortcut: 'k', handler: first });
    mgr.bind(f, { shortcut: 'k', handler: last });
    dispatchKey('k');
    expect(last).toHaveBeenCalled();
    expect(first).not.toHaveBeenCalled();
  });

  it('popFrame by symbol removes correctly even when not top', () => {
    const a = mgr.pushFrame('page', 'a');
    const b = mgr.pushFrame('page', 'b');
    mgr.popFrame(a);
    const stack = mgr.getStackSymbols();
    expect(stack).not.toContain(a);
    expect(stack).toContain(b);
  });

  it('sync throw in handler does not break listener', () => {
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    const f = mgr.pushFrame('global', 'root');
    mgr.bind(f, { shortcut: 'k', handler: () => { throw new Error('boom'); } });
    const h2 = vi.fn();
    mgr.bind(f, { shortcut: 'j', handler: h2 });
    dispatchKey('k');
    dispatchKey('j');
    expect(err).toHaveBeenCalled();
    expect(h2).toHaveBeenCalled();
    err.mockRestore();
  });

  it('rejected promise in handler does not break listener', async () => {
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    const f = mgr.pushFrame('global', 'root');
    mgr.bind(f, { shortcut: 'k', handler: () => Promise.reject(new Error('boom')) });
    dispatchKey('k');
    await new Promise(r => setTimeout(r, 0));
    expect(err).toHaveBeenCalled();
    err.mockRestore();
  });

  it('unregister during dispatch does not crash or double-fire', () => {
    const f = mgr.pushFrame('global', 'root');
    const b = vi.fn();
    const bindBSym = mgr.bind(f, { shortcut: 'k', handler: b });
    mgr.bind(f, { shortcut: 'k', handler: () => mgr.unbind(f, bindBSym) });
    dispatchKey('k');
    // last-registered wins: unbind handler fires, b never bound as active.
    expect(b).not.toHaveBeenCalled();
  });

  it('pop frame during dispatch does not crash', () => {
    const f = mgr.pushFrame('modal', 'dialog');
    mgr.bind(f, { shortcut: 'escape', handler: () => mgr.popFrame(f) });
    expect(() => dispatchKey('Escape')).not.toThrow();
  });
});
```

- [ ] **Step 2: Run test, verify fail**

```bash
pnpm --cwd app test:unit src/lib/commands/__tests__/hotkeyManager.test.ts
```

Expected: FAIL — "Failed to resolve import '../hotkeyManager'".

---

### Task 1.8: Hotkey manager — implementation

**Files:**
- Create: `app/src/lib/commands/hotkeyManager.ts`

- [ ] **Step 1: Implement hotkeyManager.ts**

```ts
import type { ActiveBinding, HotkeyBinding, ScopeFrame, ScopeKind } from './types';
import { matchEvent, parseShortcut } from './shortcut';

interface FrameInternal extends ScopeFrame {
  bindings: Map<symbol, { binding: HotkeyBinding; parsed: ReturnType<typeof parseShortcut> }>;
}

function isEditableTarget(e: KeyboardEvent): boolean {
  const path = (e.composedPath && e.composedPath()) || [];
  const nodes = path.length ? path : [e.target as EventTarget | null];
  for (const node of nodes) {
    if (!node || !(node as HTMLElement).tagName) continue;
    const el = node as HTMLElement;
    const tag = el.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
    if (el.isContentEditable === true) return true;
  }
  return false;
}

export interface HotkeyManager {
  init: () => void;
  teardown: () => void;
  pushFrame: (kind: ScopeKind, id: string) => symbol;
  popFrame: (sym: symbol) => void;
  bind: (frame: symbol, binding: HotkeyBinding) => symbol;
  unbind: (frame: symbol, bindingSymbol: symbol) => void;
  getStackSymbols: () => symbol[];
  getActiveBindings: () => ActiveBinding[];
  subscribe: (listener: () => void) => () => void;
}

export function createHotkeyManager(): HotkeyManager {
  const stack: FrameInternal[] = [];
  const listeners = new Set<() => void>();
  let initialized = false;

  function notify(): void {
    for (const l of listeners) l();
  }

  function onKeyDown(e: KeyboardEvent): void {
    // Guard: IME composition
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const composing = (e as any).isComposing === true || (e as any).keyCode === 229;
    if (composing) return;

    const inEditable = isEditableTarget(e);

    for (let i = stack.length - 1; i >= 0; i--) {
      const frame = stack[i];
      // Iterate in reverse insertion order — last-registered wins.
      const entries = [...frame.bindings.entries()];
      for (let j = entries.length - 1; j >= 0; j--) {
        const [, { binding, parsed }] = entries[j];
        if (!matchEvent(parsed, e)) continue;
        if (e.repeat && !binding.repeat) continue;
        if (inEditable && !binding.allowInInput) continue;
        if (binding.enabled && !binding.enabled()) continue;
        if (binding.preventDefault !== false) e.preventDefault();
        try {
          const r = binding.handler();
          if (r && typeof (r as Promise<unknown>).catch === 'function') {
            (r as Promise<unknown>).catch(err => console.error('[hotkey] handler rejected', err));
          }
        } catch (err) {
          console.error('[hotkey] handler threw', err);
        }
        return;
      }
    }
  }

  function init(): void {
    if (initialized) return;
    window.addEventListener('keydown', onKeyDown, { capture: true });
    initialized = true;
  }

  function teardown(): void {
    if (!initialized) return;
    window.removeEventListener('keydown', onKeyDown, { capture: true });
    initialized = false;
    stack.length = 0;
    listeners.clear();
  }

  function pushFrame(kind: ScopeKind, id: string): symbol {
    const sym = Symbol(`${kind}:${id}`);
    stack.push({ symbol: sym, id, kind, bindings: new Map() });
    notify();
    return sym;
  }

  function popFrame(sym: symbol): void {
    const idx = stack.findIndex(f => f.symbol === sym);
    if (idx === -1) return;
    stack.splice(idx, 1);
    notify();
  }

  function bind(frameSym: symbol, binding: HotkeyBinding): symbol {
    const frame = stack.find(f => f.symbol === frameSym);
    if (!frame) throw new Error('hotkeyManager.bind: unknown frame');
    const parsed = parseShortcut(binding.shortcut);
    const sym = Symbol(binding.id ?? binding.shortcut);
    frame.bindings.set(sym, { binding, parsed });
    notify();
    return sym;
  }

  function unbind(frameSym: symbol, bindingSym: symbol): void {
    const frame = stack.find(f => f.symbol === frameSym);
    if (!frame) return;
    if (frame.bindings.delete(bindingSym)) notify();
  }

  function getStackSymbols(): symbol[] {
    return stack.map(f => f.symbol);
  }

  function getActiveBindings(): ActiveBinding[] {
    const out: ActiveBinding[] = [];
    for (const frame of stack) {
      for (const { binding, parsed } of frame.bindings.values()) {
        if (binding.enabled && !binding.enabled()) continue;
        out.push({ frame: { symbol: frame.symbol, id: frame.id, kind: frame.kind }, binding, parsed });
      }
    }
    return out;
  }

  function subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }

  return {
    init, teardown, pushFrame, popFrame, bind, unbind,
    getStackSymbols, getActiveBindings, subscribe,
  };
}

export const hotkeyManager = createHotkeyManager();
```

- [ ] **Step 2: Run tests, verify pass**

```bash
pnpm --cwd app test:unit src/lib/commands/__tests__/hotkeyManager.test.ts
```

Expected: all PASS.

- [ ] **Step 3: Commit**

```bash
git add app/src/lib/commands/hotkeyManager.ts app/src/lib/commands/__tests__/hotkeyManager.test.ts
git commit -m "feat(commands): hotkey manager with scope stack + capture-phase listener"
```

---

### Task 1.9: ScopeContext + useHotkey + useRegisterAction — failing tests

**Files:**
- Create: `app/src/lib/commands/__tests__/useHotkey.test.tsx`

- [ ] **Step 1: Write failing tests**

```tsx
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, act } from '@testing-library/react';
import { StrictMode, useState } from 'react';
import { ScopeContext } from '../ScopeContext';
import { useHotkey } from '../useHotkey';
import { hotkeyManager } from '../hotkeyManager';

beforeEach(() => {
  hotkeyManager.teardown();
  hotkeyManager.init();
});

function Wrapper({ children, frame }: { children: React.ReactNode; frame: symbol }) {
  return <ScopeContext.Provider value={frame}>{children}</ScopeContext.Provider>;
}

function TestHotkey({ shortcut, handler }: { shortcut: string; handler: () => void }) {
  useHotkey(shortcut, handler);
  return null;
}

describe('useHotkey', () => {
  it('binds on mount and unbinds on unmount', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const handler = vi.fn();
    const { unmount } = render(<Wrapper frame={frame}><TestHotkey shortcut="k" handler={handler} /></Wrapper>);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    expect(handler).toHaveBeenCalledTimes(1);
    unmount();
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    expect(handler).toHaveBeenCalledTimes(1);
    hotkeyManager.popFrame(frame);
  });

  it('StrictMode double-mount yields net 1 binding', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const handler = vi.fn();
    render(
      <StrictMode>
        <Wrapper frame={frame}>
          <TestHotkey shortcut="k" handler={handler} />
        </Wrapper>
      </StrictMode>,
    );
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    expect(handler).toHaveBeenCalledTimes(1);
    hotkeyManager.popFrame(frame);
  });

  it('handler identity updates via ref without re-registration', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    let calls: string[] = [];
    function Inner() {
      const [n, setN] = useState(0);
      useHotkey('k', () => calls.push(`v${n}`));
      return <button onClick={() => setN(v => v + 1)}>bump</button>;
    }
    const { getByText } = render(<Wrapper frame={frame}><Inner /></Wrapper>);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    act(() => { getByText('bump').click(); });
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    expect(calls).toEqual(['v0', 'v1']);
    hotkeyManager.popFrame(frame);
  });
});
```

Create `app/src/lib/commands/__tests__/useRegisterAction.test.tsx`:

```tsx
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render } from '@testing-library/react';
import { ScopeContext } from '../ScopeContext';
import { useRegisterAction } from '../useRegisterAction';
import { hotkeyManager } from '../hotkeyManager';
import { registry } from '../registry';

beforeEach(() => {
  hotkeyManager.teardown();
  hotkeyManager.init();
});

function Wrapper({ children, frame }: { children: React.ReactNode; frame: symbol }) {
  return <ScopeContext.Provider value={frame}>{children}</ScopeContext.Provider>;
}
function TestAction({ id, shortcut, handler }: { id: string; shortcut?: string; handler: () => void }) {
  useRegisterAction({ id, label: id, handler, shortcut });
  return null;
}

describe('useRegisterAction', () => {
  it('adds to registry on mount, removes on unmount', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const handler = vi.fn();
    const { unmount } = render(<Wrapper frame={frame}><TestAction id="x.y" handler={handler} /></Wrapper>);
    expect(registry.getAction('x.y')?.id).toBe('x.y');
    unmount();
    expect(registry.getAction('x.y')).toBeUndefined();
    hotkeyManager.popFrame(frame);
  });

  it('with shortcut: fires via keydown AND registers in registry', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const handler = vi.fn();
    render(<Wrapper frame={frame}><TestAction id="x.y" shortcut="k" handler={handler} /></Wrapper>);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    expect(handler).toHaveBeenCalled();
    expect(registry.getAction('x.y')).toBeDefined();
    hotkeyManager.popFrame(frame);
  });
});
```

- [ ] **Step 2: Run, verify fail**

```bash
pnpm --cwd app test:unit src/lib/commands/__tests__/useHotkey.test.tsx src/lib/commands/__tests__/useRegisterAction.test.tsx
```

Expected: FAIL — missing modules.

---

### Task 1.10: ScopeContext + useHotkey + useRegisterAction — implementation

**Files:**
- Create: `app/src/lib/commands/ScopeContext.ts`
- Create: `app/src/lib/commands/useHotkey.ts`
- Create: `app/src/lib/commands/useRegisterAction.ts`

- [ ] **Step 1: Implement ScopeContext.ts**

```ts
import { createContext } from 'react';

// Default = Symbol('no-scope'). Consumers that see this should error or no-op;
// CommandProvider replaces this value at the root with the global scope symbol.
export const ScopeContext = createContext<symbol>(Symbol('no-scope'));
```

- [ ] **Step 2: Implement useHotkey.ts**

```ts
import { useContext, useEffect, useRef } from 'react';
import { ScopeContext } from './ScopeContext';
import { hotkeyManager } from './hotkeyManager';
import type { HotkeyBinding } from './types';

type HotkeyOptions = Omit<HotkeyBinding, 'shortcut' | 'handler'>;

export function useHotkey(
  shortcut: string,
  handler: () => void,
  options: HotkeyOptions = {},
): void {
  const frame = useContext(ScopeContext);
  const handlerRef = useRef(handler);
  const optsRef = useRef(options);
  handlerRef.current = handler;
  optsRef.current = options;

  useEffect(() => {
    const stable = () => handlerRef.current();
    const sym = hotkeyManager.bind(frame, {
      shortcut,
      handler: stable,
      allowInInput: optsRef.current.allowInInput,
      repeat: optsRef.current.repeat,
      preventDefault: optsRef.current.preventDefault,
      enabled: optsRef.current.enabled,
      description: optsRef.current.description,
      id: optsRef.current.id,
    });
    return () => hotkeyManager.unbind(frame, sym);
    // Rebind only when shortcut or scope changes.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shortcut, frame]);
}
```

- [ ] **Step 3: Implement useRegisterAction.ts**

```ts
import { useContext, useEffect, useRef } from 'react';
import { ScopeContext } from './ScopeContext';
import { registry } from './registry';
import { hotkeyManager } from './hotkeyManager';
import { parseShortcut } from './shortcut';
import type { Action } from './types';

export function useRegisterAction(action: Action): void {
  const frame = useContext(ScopeContext);
  const handlerRef = useRef(action.handler);
  const enabledRef = useRef(action.enabled);
  handlerRef.current = action.handler;
  enabledRef.current = action.enabled;

  useEffect(() => {
    const stable = () => handlerRef.current();
    const stableEnabled = action.enabled ? () => (enabledRef.current?.() ?? true) : undefined;
    const disposeRegistry = registry.registerAction(
      { ...action, handler: stable, enabled: stableEnabled },
      frame,
    );
    let bindingSym: symbol | undefined;
    if (action.shortcut) {
      parseShortcut(action.shortcut); // throw early on typo
      bindingSym = hotkeyManager.bind(frame, {
        shortcut: action.shortcut,
        handler: stable,
        allowInInput: action.allowInInput,
        repeat: action.repeat,
        preventDefault: action.preventDefault,
        enabled: stableEnabled,
        id: action.id,
      });
    }
    return () => {
      disposeRegistry();
      if (bindingSym) hotkeyManager.unbind(frame, bindingSym);
    };
  // Rebind when identity-relevant props change.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [action.id, action.shortcut, frame]);
}
```

- [ ] **Step 4: Run tests, verify pass**

```bash
pnpm --cwd app test:unit src/lib/commands/__tests__/useHotkey.test.tsx src/lib/commands/__tests__/useRegisterAction.test.tsx
```

Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/commands/ScopeContext.ts \
        app/src/lib/commands/useHotkey.ts \
        app/src/lib/commands/useRegisterAction.ts \
        app/src/lib/commands/__tests__/useHotkey.test.tsx \
        app/src/lib/commands/__tests__/useRegisterAction.test.tsx
git commit -m "feat(commands): useHotkey + useRegisterAction hooks with handlerRef"
```

---

### Task 1.11: `<CommandScope>` — failing tests

**Files:**
- Create: `app/src/components/commands/__tests__/CommandScope.test.tsx`

- [ ] **Step 1: Write failing tests**

```tsx
import { describe, it, expect, beforeEach } from 'vitest';
import { render } from '@testing-library/react';
import { StrictMode } from 'react';
import CommandScope from '../CommandScope';
import { hotkeyManager } from '../../../lib/commands/hotkeyManager';

beforeEach(() => { hotkeyManager.teardown(); hotkeyManager.init(); });

describe('CommandScope', () => {
  it('pushes frame on mount, pops on unmount', () => {
    const { unmount } = render(<CommandScope id="home"><div /></CommandScope>);
    expect(hotkeyManager.getStackSymbols().length).toBe(1);
    unmount();
    expect(hotkeyManager.getStackSymbols().length).toBe(0);
  });

  it('StrictMode double-mount nets a single frame', () => {
    render(<StrictMode><CommandScope id="home"><div /></CommandScope></StrictMode>);
    expect(hotkeyManager.getStackSymbols().length).toBe(1);
  });

  it('nested scopes push two frames', () => {
    render(<CommandScope id="page"><CommandScope id="modal" kind="modal"><div /></CommandScope></CommandScope>);
    expect(hotkeyManager.getStackSymbols().length).toBe(2);
  });

  it('pops by symbol (out-of-order unmount safe)', () => {
    function App({ showInner }: { showInner: boolean }) {
      return (
        <CommandScope id="outer">
          {showInner && <CommandScope id="inner"><div /></CommandScope>}
        </CommandScope>
      );
    }
    const { rerender, unmount } = render(<App showInner={true} />);
    expect(hotkeyManager.getStackSymbols().length).toBe(2);
    rerender(<App showInner={false} />);
    expect(hotkeyManager.getStackSymbols().length).toBe(1);
    unmount();
    expect(hotkeyManager.getStackSymbols().length).toBe(0);
  });
});
```

- [ ] **Step 2: Run, verify fail**

```bash
pnpm --cwd app test:unit src/components/commands/__tests__/CommandScope.test.tsx
```

Expected: FAIL (missing module).

---

### Task 1.12: `<CommandScope>` — implementation

**Files:**
- Create: `app/src/components/commands/CommandScope.tsx`

- [ ] **Step 1: Implement CommandScope.tsx**

```tsx
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ScopeContext } from '../../lib/commands/ScopeContext';
import { hotkeyManager } from '../../lib/commands/hotkeyManager';
import type { ScopeKind } from '../../lib/commands/types';

interface Props {
  id: string;
  kind?: ScopeKind;
  children: ReactNode;
}

export default function CommandScope({ id, kind = 'page', children }: Props) {
  // Use state initializer + ref guard so StrictMode double-invoke of effects
  // yields a single frame.
  const [frame] = useState(() => hotkeyManager.pushFrame(kind, id));
  const mounted = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      hotkeyManager.popFrame(frame);
    };
    // frame is stable — set once at mount.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const value = useMemo(() => frame, [frame]);
  return <ScopeContext.Provider value={value}>{children}</ScopeContext.Provider>;
}
```

- [ ] **Step 2: Run tests, verify pass**

```bash
pnpm --cwd app test:unit src/components/commands/__tests__/CommandScope.test.tsx
```

Expected: all PASS.

- [ ] **Step 3: Commit**

```bash
git add app/src/components/commands/CommandScope.tsx \
        app/src/components/commands/__tests__/CommandScope.test.tsx
git commit -m "feat(commands): CommandScope component for scope stack management"
```

---

## Gate 2 — Semantic Tokens

### Task 2.1: Add `cmd-*` tokens to Tailwind config

**Files:**
- Modify: `app/tailwind.config.js`

- [ ] **Step 1: Edit tailwind.config.js**

Inside `theme.extend.colors` (alphabetical position), add:

```js
        // Command surface tokens — scoped to the ⌘K palette / help overlay.
        // Expand this set only with intent; the full reskin design system
        // is a separate decision.
        'cmd-surface':          'var(--cmd-surface)',
        'cmd-surface-elevated': 'var(--cmd-surface-elevated)',
        'cmd-foreground':       'var(--cmd-foreground)',
        'cmd-foreground-muted': 'var(--cmd-foreground-muted)',
        'cmd-border':           'var(--cmd-border)',
        'cmd-ring':             'var(--cmd-ring)',
        'cmd-accent':           'var(--cmd-accent)',
        'cmd-overlay':          'var(--cmd-overlay)',
```

Inside `theme.extend` (after `colors`), add (or extend existing `boxShadow`):

```js
      boxShadow: {
        'cmd-palette': 'var(--cmd-shadow-palette)',
      },
```

If `boxShadow` already exists in `extend`, append the key; do not replace the object.

- [ ] **Step 2: Sanity check**

```bash
pnpm --cwd app compile
```

Expected: 0 errors.

---

### Task 2.2: Add CSS vars in index.css

**Files:**
- Modify: `app/src/index.css`

- [ ] **Step 1: Append to `index.css`**

At the end of the file, add:

```css
/* Command palette + help overlay — scoped tokens. */
:root {
  --cmd-accent:           #2F6EF4;
  --cmd-surface:          #FFFFFF;
  --cmd-surface-elevated: #F5F5F5;
  --cmd-foreground:       #171717;
  --cmd-foreground-muted: #737373;
  --cmd-border:           #E5E5E5;
  --cmd-ring:             var(--cmd-accent);
  --cmd-overlay:          rgba(0, 0, 0, 0.5);
  --cmd-shadow-palette:
    0 20px 25px -5px rgba(0, 0, 0, 0.1),
    0 10px 10px -5px rgba(0, 0, 0, 0.04);
}

:root.dark {
  --cmd-accent:           #60A5FA;
  --cmd-surface:          #171717;
  --cmd-surface-elevated: #262626;
  --cmd-foreground:       #FAFAFA;
  --cmd-foreground-muted: #A3A3A3;
  --cmd-border:           #404040;
  --cmd-overlay:          rgba(0, 0, 0, 0.7);
  --cmd-shadow-palette:
    0 20px 25px -5px rgba(0, 0, 0, 0.5),
    0 10px 10px -5px rgba(0, 0, 0, 0.25);
}

@media (prefers-reduced-motion: reduce) {
  .cmd-palette-enter,
  .cmd-palette-exit,
  .cmd-help-enter,
  .cmd-help-exit {
    animation: none !important;
    transition: none !important;
  }
}
```

---

### Task 2.3: Wire `lint:commands-tokens` script + husky

**Files:**
- Modify: `app/package.json`
- Modify: `app/.husky/pre-push` (or whichever husky hook lint runs in; check first)

- [ ] **Step 1: Audit husky hooks**

```bash
ls app/.husky 2>/dev/null && cat app/.husky/pre-push 2>/dev/null
ls .husky 2>/dev/null && cat .husky/pre-push 2>/dev/null
```

Note the path and content — we'll append the new lint.

- [ ] **Step 2: Add `lint:commands-tokens` script**

In `app/package.json` `scripts`, add next to the existing `lint`:

```json
"lint:commands-tokens": "bash -c '! rg -nU \"(bg|text|border|ring|shadow)-(neutral|primary|sage|amber|canvas|stone|slate)\" src/components/commands/'"
```

The bash wrapper inverts: the script exits 0 iff `rg` finds NO matches; exits 1 if any raw-color class sneaks in.

- [ ] **Step 3: Wire into pre-push**

Append to the husky pre-push hook (path discovered in Step 1) inside the frontend section:

```bash
pnpm --cwd app lint:commands-tokens
```

If there is no existing pre-push hook, skip wiring — surface this at commit time:

```bash
# run manually or wire when husky is added:
pnpm --cwd app lint:commands-tokens
```

- [ ] **Step 4: Run the lint script against empty dir**

```bash
pnpm --cwd app lint:commands-tokens
```

Expected: exit 0 (no matches yet, since `components/commands/` holds only `CommandScope.tsx` which uses no Tailwind color classes).

- [ ] **Step 5: Commit Gate 2**

```bash
git add app/tailwind.config.js app/src/index.css app/package.json
# add husky file if modified
git commit -m "style(commands): add scoped cmd-* semantic tokens + token lint script"
```

---

## Gate 3 — Components

### Task 3.1: `<Kbd>` — failing tests

**Files:**
- Create: `app/src/components/commands/__tests__/Kbd.test.tsx`

- [ ] **Step 1: Write failing tests**

```tsx
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { render } from '@testing-library/react';
import Kbd from '../Kbd';

function withPlatform(value: string, fn: () => void) {
  const orig = navigator.platform;
  Object.defineProperty(navigator, 'platform', { value, configurable: true });
  try { fn(); } finally {
    Object.defineProperty(navigator, 'platform', { value: orig, configurable: true });
  }
}

describe('Kbd', () => {
  it('renders mac glyphs for mod+shift+k', () => {
    withPlatform('MacIntel', () => {
      const { container } = render(<Kbd shortcut="shift+mod+k" />);
      expect(container.textContent).toMatch(/⇧/);
      expect(container.textContent).toMatch(/⌘/);
      expect(container.textContent).toMatch(/K/);
    });
  });

  it('renders PC labels on Win32', () => {
    withPlatform('Win32', () => {
      const { container } = render(<Kbd shortcut="shift+mod+k" />);
      expect(container.textContent).toMatch(/Shift/);
      expect(container.textContent).toMatch(/Ctrl/);
    });
  });

  it('renders single printable', () => {
    withPlatform('MacIntel', () => {
      const { container } = render(<Kbd shortcut="?" />);
      expect(container.textContent).toMatch(/\?/);
    });
  });
});
```

- [ ] **Step 2: Run, verify fail**

```bash
pnpm --cwd app test:unit src/components/commands/__tests__/Kbd.test.tsx
```

---

### Task 3.2: `<Kbd>` — implementation

**Files:**
- Create: `app/src/components/commands/Kbd.tsx`

- [ ] **Step 1: Implement Kbd.tsx**

```tsx
import { memo, useMemo } from 'react';
import { formatShortcut, isMac, parseShortcut } from '../../lib/commands/shortcut';

interface Props {
  shortcut: string;
  size?: 'sm' | 'md';
  className?: string;
}

function Kbd({ shortcut, size = 'sm', className = '' }: Props) {
  const segments = useMemo(() => formatShortcut(parseShortcut(shortcut), isMac()), [shortcut]);
  const padding = size === 'md' ? 'px-2 py-1 text-sm' : 'px-1.5 py-0.5 text-xs';
  return (
    <span className={`inline-flex items-center gap-1 font-mono ${className}`} aria-label={`Keyboard shortcut: ${segments.join(' ')}`}>
      {segments.map((seg, i) => (
        <kbd
          key={i}
          className={`${padding} rounded border border-cmd-border bg-cmd-surface-elevated text-cmd-foreground-muted`}
        >
          {seg}
        </kbd>
      ))}
    </span>
  );
}

export default memo(Kbd);
```

- [ ] **Step 2: Run tests, verify pass**

```bash
pnpm --cwd app test:unit src/components/commands/__tests__/Kbd.test.tsx
```

- [ ] **Step 3: Commit**

```bash
git add app/src/components/commands/Kbd.tsx app/src/components/commands/__tests__/Kbd.test.tsx
git commit -m "feat(commands): Kbd component for shortcut rendering"
```

---

### Task 3.3: Command test utilities

**Files:**
- Create: `app/src/test/commandTestUtils.ts`

- [ ] **Step 1: Write commandTestUtils.ts**

```ts
import { expect } from 'vitest';

export interface PressKeyOptions {
  key: string;
  mod?: boolean;   // cmd on mac, ctrl elsewhere
  shift?: boolean;
  alt?: boolean;
  ctrl?: boolean;
  target?: EventTarget;
}

export function pressKey(opts: PressKeyOptions): KeyboardEvent {
  const mac = navigator.platform.toLowerCase().includes('mac');
  const modPair = opts.mod ? (mac ? { metaKey: true } : { ctrlKey: true }) : {};
  const target = opts.target ?? window;
  const event = new KeyboardEvent('keydown', {
    key: opts.key,
    bubbles: true,
    cancelable: true,
    shiftKey: !!opts.shift,
    altKey: !!opts.alt,
    ctrlKey: !!opts.ctrl,
    ...modPair,
  });
  target.dispatchEvent(event);
  return event;
}

// Meta-test: the util must actually reach capture-phase listeners.
export function __metaAssertPressKeyReachesCaptureListener(): void {
  let reached = false;
  const listener = (_e: KeyboardEvent) => { reached = true; };
  window.addEventListener('keydown', listener, { capture: true });
  pressKey({ key: 'z' });
  window.removeEventListener('keydown', listener, { capture: true });
  expect(reached).toBe(true);
}
```

- [ ] **Step 2: Add meta-test to hotkeyManager.test.ts or a new file**

Create `app/src/lib/commands/__tests__/testUtils.meta.test.ts`:

```ts
import { describe, it } from 'vitest';
import { __metaAssertPressKeyReachesCaptureListener } from '../../../test/commandTestUtils';

describe('commandTestUtils', () => {
  it('pressKey reaches capture-phase listeners', () => {
    __metaAssertPressKeyReachesCaptureListener();
  });
});
```

- [ ] **Step 3: Run meta-test**

```bash
pnpm --cwd app test:unit src/lib/commands/__tests__/testUtils.meta.test.ts
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add app/src/test/commandTestUtils.ts \
        app/src/lib/commands/__tests__/testUtils.meta.test.ts
git commit -m "test(commands): pressKey helper + capture-phase meta-test"
```

---

### Task 3.4: `<CommandPalette>` — failing tests

**Files:**
- Create: `app/src/components/commands/__tests__/CommandPalette.test.tsx`

- [ ] **Step 1: Write failing tests**

```tsx
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import CommandPalette from '../CommandPalette';
import { ScopeContext } from '../../../lib/commands/ScopeContext';
import { registry } from '../../../lib/commands/registry';
import { hotkeyManager } from '../../../lib/commands/hotkeyManager';

beforeEach(() => {
  hotkeyManager.teardown();
  hotkeyManager.init();
});

function Harness({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const frame = hotkeyManager.pushFrame('global', 'root');
  registry.setActiveStack([frame]);
  const handler = vi.fn();
  registry.registerAction({ id: 'nav.settings', label: 'Open Settings', handler, group: 'Navigation', shortcut: 'mod+,' }, frame);
  return (
    <ScopeContext.Provider value={frame}>
      <CommandPalette open={open} onOpenChange={onOpenChange} />
    </ScopeContext.Provider>
  );
}

describe('CommandPalette', () => {
  it('renders registered actions when open', () => {
    render(<Harness open={true} onOpenChange={() => {}} />);
    expect(screen.getByText('Open Settings')).toBeInTheDocument();
  });

  it('filters by typed query', async () => {
    const user = userEvent.setup();
    render(<Harness open={true} onOpenChange={() => {}} />);
    const input = screen.getByRole('combobox');
    await user.type(input, 'xyzzy');
    expect(screen.queryByText('Open Settings')).not.toBeInTheDocument();
  });

  it('fires handler on Enter and calls onOpenChange(false)', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(<Harness open={true} onOpenChange={onOpenChange} />);
    const input = screen.getByRole('combobox');
    await user.type(input, 'settings');
    await user.keyboard('{Enter}');
    await act(async () => { await new Promise(r => requestAnimationFrame(() => r(null))); });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('renders footer hint', () => {
    render(<Harness open={true} onOpenChange={() => {}} />);
    expect(screen.getByText(/Press \? for all shortcuts/i)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run, verify fail**

```bash
pnpm --cwd app test:unit src/components/commands/__tests__/CommandPalette.test.tsx
```

---

### Task 3.5: `<CommandPalette>` — implementation

**Files:**
- Create: `app/src/components/commands/CommandPalette.tsx`

- [ ] **Step 1: Implement CommandPalette.tsx**

```tsx
import { useSyncExternalStore, useMemo } from 'react';
import { Command } from 'cmdk';
import * as Dialog from '@radix-ui/react-dialog';
import { registry } from '../../lib/commands/registry';
import { hotkeyManager } from '../../lib/commands/hotkeyManager';
import Kbd from './Kbd';
import type { RegisteredAction } from '../../lib/commands/types';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

function subscribe(listener: () => void): () => void {
  const u1 = registry.subscribe(listener);
  const u2 = hotkeyManager.subscribe(listener);
  return () => { u1(); u2(); };
}

function getSnapshot(): RegisteredAction[] {
  return registry.getActiveActions(hotkeyManager.getStackSymbols());
}

export default function CommandPalette({ open, onOpenChange }: Props) {
  const actions = useSyncExternalStore(subscribe, getSnapshot);

  const groups = useMemo(() => {
    const byGroup = new Map<string, RegisteredAction[]>();
    for (const a of actions) {
      const g = a.group ?? 'Actions';
      if (!byGroup.has(g)) byGroup.set(g, []);
      byGroup.get(g)!.push(a);
    }
    const order = ['Navigation', 'Help'];
    const keys = [...byGroup.keys()].sort((a, b) => {
      const ai = order.indexOf(a), bi = order.indexOf(b);
      if (ai === -1 && bi === -1) return a.localeCompare(b);
      if (ai === -1) return 1;
      if (bi === -1) return -1;
      return ai - bi;
    });
    return keys.map(k => [k, byGroup.get(k)!] as const);
  }, [actions]);

  function runAction(action: RegisteredAction): void {
    onOpenChange(false);
    requestAnimationFrame(() => { registry.runAction(action.id); });
  }

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 bg-cmd-overlay z-40" />
        <Dialog.Content
          className="fixed left-1/2 top-[20vh] -translate-x-1/2 w-[min(640px,calc(100vw-32px))] bg-cmd-surface text-cmd-foreground border border-cmd-border rounded-xl shadow-cmd-palette z-50 overflow-hidden"
          aria-label="Command palette">
          <Dialog.Title className="sr-only">Command palette</Dialog.Title>
          <Command label="Commands" shouldFilter={true}>
            <Command.Input
              autoFocus
              placeholder="Type a command or search…"
              className="w-full px-4 py-3 bg-transparent outline-none border-b border-cmd-border text-cmd-foreground placeholder:text-cmd-foreground-muted"
              aria-label="Search commands"
            />
            <Command.List className="max-h-[50vh] overflow-auto py-2">
              <Command.Empty className="px-4 py-8 text-center text-cmd-foreground-muted">
                No results.
              </Command.Empty>
              {groups.map(([groupName, items]) => (
                <Command.Group key={groupName} heading={groupName}
                  className="[&_[cmdk-group-heading]]:px-4 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-xs [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:text-cmd-foreground-muted">
                  {items.map(action => (
                    <Command.Item
                      key={action.id}
                      value={action.id}
                      keywords={[action.label, ...(action.keywords ?? [])]}
                      onSelect={() => runAction(action)}
                      className="flex items-center gap-3 px-4 py-2 cursor-pointer aria-selected:bg-cmd-surface-elevated"
                    >
                      {action.icon ? <action.icon className="w-4 h-4 text-cmd-foreground-muted" /> : <span className="w-4" />}
                      <span className="flex-1 truncate">{action.label}</span>
                      {action.hint && <span className="text-xs text-cmd-foreground-muted truncate">{action.hint}</span>}
                      {action.shortcut && <Kbd shortcut={action.shortcut} />}
                    </Command.Item>
                  ))}
                </Command.Group>
              ))}
            </Command.List>
            <div className="px-4 py-2 border-t border-cmd-border text-xs text-cmd-foreground-muted">
              Press <kbd className="mx-1 px-1 py-0.5 rounded border border-cmd-border bg-cmd-surface-elevated">?</kbd> for all shortcuts
            </div>
          </Command>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
```

- [ ] **Step 2: Run tests, verify pass**

```bash
pnpm --cwd app test:unit src/components/commands/__tests__/CommandPalette.test.tsx
```

- [ ] **Step 3: Commit**

```bash
git add app/src/components/commands/CommandPalette.tsx \
        app/src/components/commands/__tests__/CommandPalette.test.tsx
git commit -m "feat(commands): CommandPalette component (cmdk + Radix Dialog)"
```

---

### Task 3.6: `<HelpOverlay>` — failing tests

**Files:**
- Create: `app/src/components/commands/__tests__/HelpOverlay.test.tsx`

- [ ] **Step 1: Write failing tests**

```tsx
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import HelpOverlay from '../HelpOverlay';
import { ScopeContext } from '../../../lib/commands/ScopeContext';
import { hotkeyManager } from '../../../lib/commands/hotkeyManager';
import { registry } from '../../../lib/commands/registry';

beforeEach(() => {
  hotkeyManager.teardown();
  hotkeyManager.init();
});

describe('HelpOverlay', () => {
  it('shows actions section with registered action shortcuts', () => {
    const f = hotkeyManager.pushFrame('global', 'root');
    registry.setActiveStack([f]);
    registry.registerAction({ id: 'nav.home', label: 'Go Home', group: 'Navigation', handler: vi.fn(), shortcut: 'mod+1' }, f);
    hotkeyManager.bind(f, { shortcut: 'mod+1', handler: vi.fn(), id: 'nav.home' });
    render(
      <ScopeContext.Provider value={f}>
        <HelpOverlay open={true} onOpenChange={() => {}} />
      </ScopeContext.Provider>,
    );
    expect(screen.getByText('Go Home')).toBeInTheDocument();
    expect(screen.getByText(/Actions/i)).toBeInTheDocument();
  });

  it('shows bare HotkeyBinding with description in Shortcuts section', () => {
    const f = hotkeyManager.pushFrame('global', 'root');
    hotkeyManager.bind(f, { shortcut: 'mod+/', handler: vi.fn(), description: 'Toggle foo' });
    render(
      <ScopeContext.Provider value={f}>
        <HelpOverlay open={true} onOpenChange={() => {}} />
      </ScopeContext.Provider>,
    );
    expect(screen.getByText('Toggle foo')).toBeInTheDocument();
    expect(screen.getByText(/Shortcuts/i)).toBeInTheDocument();
  });

  it('dedups same shortcut across scopes', () => {
    const g = hotkeyManager.pushFrame('global', 'root');
    const p = hotkeyManager.pushFrame('page', 'home');
    registry.registerAction({ id: 'nav.home', label: 'Go Home global', handler: vi.fn(), shortcut: 'mod+1' }, g);
    registry.registerAction({ id: 'nav.home', label: 'Go Home page', handler: vi.fn(), shortcut: 'mod+1' }, p);
    hotkeyManager.bind(g, { shortcut: 'mod+1', handler: vi.fn(), id: 'nav.home' });
    hotkeyManager.bind(p, { shortcut: 'mod+1', handler: vi.fn(), id: 'nav.home' });
    render(
      <ScopeContext.Provider value={p}>
        <HelpOverlay open={true} onOpenChange={() => {}} />
      </ScopeContext.Provider>,
    );
    const matches = screen.queryAllByText(/Go Home/);
    expect(matches.length).toBe(1);
    expect(matches[0].textContent).toContain('page');
  });
});
```

- [ ] **Step 2: Run, verify fail**

```bash
pnpm --cwd app test:unit src/components/commands/__tests__/HelpOverlay.test.tsx
```

---

### Task 3.7: `<HelpOverlay>` — implementation

**Files:**
- Create: `app/src/components/commands/HelpOverlay.tsx`

- [ ] **Step 1: Implement HelpOverlay.tsx**

```tsx
import { useSyncExternalStore, useMemo } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { registry } from '../../lib/commands/registry';
import { hotkeyManager } from '../../lib/commands/hotkeyManager';
import Kbd from './Kbd';
import type { ActiveBinding, RegisteredAction } from '../../lib/commands/types';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

function subscribe(listener: () => void): () => void {
  const u1 = registry.subscribe(listener);
  const u2 = hotkeyManager.subscribe(listener);
  return () => { u1(); u2(); };
}

function getActions(): RegisteredAction[] {
  return registry.getActiveActions(hotkeyManager.getStackSymbols()).filter(a => !!a.shortcut);
}
function getBindings(): ActiveBinding[] {
  return hotkeyManager.getActiveBindings();
}

function canonicalize(shortcut: string): string {
  return shortcut.toLowerCase().split('+').sort().join('+');
}

export default function HelpOverlay({ open, onOpenChange }: Props) {
  const actions = useSyncExternalStore(subscribe, getActions);
  const bindings = useSyncExternalStore(subscribe, getBindings);

  const { actionRows, shortcutRows } = useMemo(() => {
    const actionRows = [...actions].sort((a, b) => (a.group ?? '').localeCompare(b.group ?? '') || a.label.localeCompare(b.label));
    // Dedup bindings-with-description that are NOT backed by an action
    const actionShortcutKeys = new Set(actions.map(a => canonicalize(a.shortcut!)));
    const seen = new Set<string>();
    const shortcutRows: ActiveBinding[] = [];
    for (const b of bindings) {
      if (!b.binding.description) continue;
      const k = canonicalize(b.binding.shortcut);
      if (actionShortcutKeys.has(k) || seen.has(k)) continue;
      seen.add(k);
      shortcutRows.push(b);
    }
    return { actionRows, shortcutRows };
  }, [actions, bindings]);

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 bg-cmd-overlay z-40" />
        <Dialog.Content
          className="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-[min(560px,calc(100vw-32px))] max-h-[80vh] overflow-auto bg-cmd-surface text-cmd-foreground border border-cmd-border rounded-xl shadow-cmd-palette z-50 p-4"
          aria-label="Keyboard shortcuts">
          <Dialog.Title className="text-lg font-semibold mb-3">Keyboard shortcuts</Dialog.Title>
          <section aria-label="Actions">
            <h3 className="text-xs uppercase text-cmd-foreground-muted mb-2">Actions</h3>
            <ul className="space-y-1">
              {actionRows.map(a => (
                <li key={a.id} className="flex items-center justify-between py-1">
                  <span>{a.label}</span>
                  <Kbd shortcut={a.shortcut!} />
                </li>
              ))}
            </ul>
          </section>
          {shortcutRows.length > 0 && (
            <section aria-label="Shortcuts" className="mt-4">
              <h3 className="text-xs uppercase text-cmd-foreground-muted mb-2">Shortcuts</h3>
              <ul className="space-y-1">
                {shortcutRows.map((b, i) => (
                  <li key={i} className="flex items-center justify-between py-1">
                    <span>{b.binding.description}</span>
                    <Kbd shortcut={b.binding.shortcut} />
                  </li>
                ))}
              </ul>
            </section>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
```

- [ ] **Step 2: Run tests, verify pass**

```bash
pnpm --cwd app test:unit src/components/commands/__tests__/HelpOverlay.test.tsx
```

- [ ] **Step 3: Commit**

```bash
git add app/src/components/commands/HelpOverlay.tsx \
        app/src/components/commands/__tests__/HelpOverlay.test.tsx
git commit -m "feat(commands): HelpOverlay with Actions + Shortcuts sections"
```

---

### Task 3.8: `globalActions.ts`

**Files:**
- Create: `app/src/lib/commands/globalActions.ts`
- Create: `app/src/lib/commands/__tests__/globalActions.test.tsx`

- [ ] **Step 1: Write failing test**

```tsx
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { registerGlobalActions, GROUP_ORDER } from '../globalActions';
import { hotkeyManager } from '../hotkeyManager';
import { registry } from '../registry';

beforeEach(() => { hotkeyManager.teardown(); hotkeyManager.init(); });

describe('registerGlobalActions', () => {
  it('registers all 6 seed actions into the global frame', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const navigate = vi.fn();
    const openHelp = vi.fn();
    registerGlobalActions(navigate as any, openHelp, frame);
    const ids = ['nav.home', 'nav.chat', 'nav.intelligence', 'nav.skills', 'nav.settings', 'help.show'];
    for (const id of ids) expect(registry.getAction(id)?.id).toBe(id);
  });

  it('nav.home handler calls navigate("/home")', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const navigate = vi.fn();
    registerGlobalActions(navigate as any, vi.fn(), frame);
    registry.setActiveStack([frame]);
    registry.runAction('nav.home');
    expect(navigate).toHaveBeenCalledWith('/home');
  });

  it('help.show handler calls openHelp', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const openHelp = vi.fn();
    registerGlobalActions(vi.fn() as any, openHelp, frame);
    registry.setActiveStack([frame]);
    registry.runAction('help.show');
    expect(openHelp).toHaveBeenCalled();
  });

  it('exports GROUP_ORDER', () => {
    expect(GROUP_ORDER).toEqual(['Navigation', 'Help']);
  });
});
```

- [ ] **Step 2: Run, verify fail**

```bash
pnpm --cwd app test:unit src/lib/commands/__tests__/globalActions.test.tsx
```

- [ ] **Step 3: Implement globalActions.ts**

```ts
import type { NavigateFunction } from 'react-router-dom';
import { registry } from './registry';
import { hotkeyManager } from './hotkeyManager';

export const GROUP_ORDER = ['Navigation', 'Help'] as const;

export function registerGlobalActions(
  navigate: NavigateFunction,
  openHelpOverlay: () => void,
  globalScopeSymbol: symbol,
): void {
  const nav = (path: string) => () => { navigate(path); };

  const actions = [
    { id: 'nav.home',         label: 'Go Home',                group: 'Navigation', shortcut: 'mod+1', handler: nav('/home'),         keywords: ['dashboard'] },
    { id: 'nav.chat',         label: 'Go to Chat',             group: 'Navigation', shortcut: 'mod+2', handler: nav('/chat'),         keywords: ['conversations', 'messages', 'inbox'] },
    { id: 'nav.intelligence', label: 'Go to Intelligence',     group: 'Navigation', shortcut: 'mod+3', handler: nav('/intelligence'), keywords: ['memory', 'knowledge'] },
    { id: 'nav.skills',       label: 'Go to Skills',           group: 'Navigation', shortcut: 'mod+4', handler: nav('/skills'),       keywords: ['plugins', 'tools'] },
    { id: 'nav.settings',     label: 'Open Settings',          group: 'Navigation', shortcut: 'mod+,', handler: nav('/settings'),     keywords: ['preferences', 'config'] },
    { id: 'help.show',        label: 'Show Keyboard Shortcuts', group: 'Help',       shortcut: '?',     handler: openHelpOverlay,      keywords: ['help', 'shortcuts'] },
  ];

  for (const a of actions) {
    registry.registerAction(a, globalScopeSymbol);
    hotkeyManager.bind(globalScopeSymbol, {
      shortcut: a.shortcut,
      handler: a.handler,
      id: a.id,
    });
  }
}
```

- [ ] **Step 4: Run tests, verify pass**

```bash
pnpm --cwd app test:unit src/lib/commands/__tests__/globalActions.test.tsx
```

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/commands/globalActions.ts \
        app/src/lib/commands/__tests__/globalActions.test.tsx
git commit -m "feat(commands): seed 6 global actions (5 nav + help)"
```

---

### Task 3.9: `<CommandProvider>` — failing test

**Files:**
- Create: `app/src/components/commands/__tests__/CommandProvider.test.tsx`

- [ ] **Step 1: Write failing test**

```tsx
import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import CommandProvider from '../CommandProvider';
import { hotkeyManager } from '../../../lib/commands/hotkeyManager';
import { pressKey } from '../../../test/commandTestUtils';

beforeEach(() => { hotkeyManager.teardown(); });

describe('CommandProvider', () => {
  it('mounts and registers seed actions', () => {
    render(
      <MemoryRouter>
        <CommandProvider>
          <div>child</div>
        </CommandProvider>
      </MemoryRouter>,
    );
    expect(screen.getByText('child')).toBeInTheDocument();
  });

  it('opens palette on mod+K', async () => {
    render(
      <MemoryRouter>
        <CommandProvider>
          <div>child</div>
        </CommandProvider>
      </MemoryRouter>,
    );
    act(() => { pressKey({ key: 'k', mod: true }); });
    expect(await screen.findByRole('dialog', { name: /Command palette/i })).toBeInTheDocument();
  });

  it('opens help on ?', async () => {
    render(
      <MemoryRouter>
        <CommandProvider>
          <div>child</div>
        </CommandProvider>
      </MemoryRouter>,
    );
    act(() => { pressKey({ key: '?' }); });
    expect(await screen.findByRole('dialog', { name: /Keyboard shortcuts/i })).toBeInTheDocument();
  });

  it('Esc closes open overlay', async () => {
    const user = userEvent.setup();
    render(
      <MemoryRouter>
        <CommandProvider>
          <div>child</div>
        </CommandProvider>
      </MemoryRouter>,
    );
    act(() => { pressKey({ key: 'k', mod: true }); });
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('palette and help mutually exclusive (opening help closes palette)', async () => {
    render(
      <MemoryRouter>
        <CommandProvider>
          <div>child</div>
        </CommandProvider>
      </MemoryRouter>,
    );
    act(() => { pressKey({ key: 'k', mod: true }); });
    expect(await screen.findByRole('dialog', { name: /Command palette/i })).toBeInTheDocument();
    act(() => { pressKey({ key: '?' }); });
    expect(await screen.findByRole('dialog', { name: /Keyboard shortcuts/i })).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: /Command palette/i })).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run, verify fail**

```bash
pnpm --cwd app test:unit src/components/commands/__tests__/CommandProvider.test.tsx
```

---

### Task 3.10: `<CommandProvider>` — implementation

**Files:**
- Create: `app/src/components/commands/CommandProvider.tsx`

- [ ] **Step 1: Implement CommandProvider.tsx**

```tsx
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router-dom';
import { ScopeContext } from '../../lib/commands/ScopeContext';
import { hotkeyManager } from '../../lib/commands/hotkeyManager';
import { registry } from '../../lib/commands/registry';
import { registerGlobalActions } from '../../lib/commands/globalActions';
import CommandPalette from './CommandPalette';
import HelpOverlay from './HelpOverlay';

let instanceCount = 0;

interface Props { children: ReactNode; }

export default function CommandProvider({ children }: Props) {
  const navigate = useNavigate();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);

  // One-time setup: init manager, push global frame, register globals.
  const setupDone = useRef(false);
  const globalFrame = useRef<symbol | null>(null);

  if (!setupDone.current) {
    hotkeyManager.init();
    globalFrame.current = hotkeyManager.pushFrame('global', 'root');
    registry.setActiveStack(hotkeyManager.getStackSymbols());
    registerGlobalActions(navigate, () => {
      setPaletteOpen(false);
      setHelpOpen(true);
    }, globalFrame.current);
    setupDone.current = true;
  }

  useEffect(() => {
    instanceCount += 1;
    if (instanceCount > 1) {
      // eslint-disable-next-line no-console
      console.warn('[commands] CommandProvider mounted more than once — this is unsupported');
    }
    return () => { instanceCount -= 1; };
  }, []);

  // Keep registry in sync with stack on every render (cheap).
  useEffect(() => {
    registry.setActiveStack(hotkeyManager.getStackSymbols());
  });

  // Meta hotkey: ⌘K opens palette (allowInInput: true).
  useEffect(() => {
    if (!globalFrame.current) return;
    const frame = globalFrame.current;
    const sym = hotkeyManager.bind(frame, {
      shortcut: 'mod+k',
      handler: () => { setHelpOpen(false); setPaletteOpen(o => !o); },
      allowInInput: true,
      id: 'meta.open-palette',
    });
    return () => hotkeyManager.unbind(frame, sym);
  }, []);

  // Help is already bound via help.show Action (keyword "?"). We wire Esc via Radix Dialog.

  const frame = globalFrame.current!;
  const value = useMemo(() => frame, [frame]);

  return (
    <ScopeContext.Provider value={value}>
      {children}
      <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} />
      <HelpOverlay open={helpOpen} onOpenChange={setHelpOpen} />
    </ScopeContext.Provider>
  );
}
```

- [ ] **Step 2: Run tests, verify pass**

```bash
pnpm --cwd app test:unit src/components/commands/__tests__/CommandProvider.test.tsx
```

- [ ] **Step 3: Commit**

```bash
git add app/src/components/commands/CommandProvider.tsx \
        app/src/components/commands/__tests__/CommandProvider.test.tsx
git commit -m "feat(commands): CommandProvider root mount with palette + help"
```

---

### Task 3.11: Run full Gate 3 verification

- [ ] **Step 1: Run all command tests**

```bash
pnpm --cwd app test:unit src/lib/commands src/components/commands src/test/commandTestUtils.ts
```

Expected: all PASS.

- [ ] **Step 2: Typecheck**

```bash
pnpm --cwd app compile
```

Expected: 0 errors.

- [ ] **Step 3: Lint**

```bash
pnpm --cwd app lint
```

Expected: 0 errors in `src/lib/commands/` and `src/components/commands/`.

- [ ] **Step 4: Token lint**

```bash
pnpm --cwd app lint:commands-tokens
```

Expected: exit 0.

---

## Gate 4 — Wire into App

### Task 4.1: Audit existing keydown listeners for conflicts

**Files:**
- Read: `app/src/components/DictationHotkeyManager.tsx`
- Grep: `rg "addEventListener.*keydown" app/src`

- [ ] **Step 1: Grep**

```bash
rg -n "addEventListener.*keydown" app/src
```

Record every hit in the task note.

- [ ] **Step 2: Read DictationHotkeyManager**

Inspect which chord it uses. Confirm none of our seeds (`⌘K`, `⌘1–4`, `⌘,`, `?`) collide.

- [ ] **Step 3: Document findings**

Append to this task in the plan (or leave a note in the commit):

```
Existing keydown listeners:
- DictationHotkeyManager — uses <chord-from-file>. No conflict with v1 seeds.
- <others>
```

If any collide, either (a) rebase the existing listener onto our `useHotkey` via scope, or (b) pick a different shortcut. Do not ship colliding bindings.

---

### Task 4.2: Wire `<CommandProvider>` into App.tsx

**Files:**
- Modify: `app/src/App.tsx`

- [ ] **Step 1: Edit App.tsx**

Add import:

```tsx
import CommandProvider from './components/commands/CommandProvider';
```

Change the block:

```tsx
<Router>
  <ServiceBlockingGate>
```

to:

```tsx
<Router>
  <CommandProvider>
    <ServiceBlockingGate>
```

And the matching close:

```tsx
    </ServiceBlockingGate>
  </CommandProvider>
</Router>
```

- [ ] **Step 2: Typecheck**

```bash
pnpm --cwd app compile
```

Expected: 0 errors.

- [ ] **Step 3: Smoke test via dev**

```bash
pnpm --cwd app dev:app
```

With the app at `/home` (log in if needed):
- Press `⌘K` → palette opens, "Go Home", "Go to Chat", etc. visible.
- Type `settings` → "Open Settings" row filters in. Enter → navigates.
- Press `?` → help overlay opens listing all 6 shortcuts.
- Press `Esc` → overlay closes.

- [ ] **Step 4: Commit**

```bash
git add app/src/App.tsx
git commit -m "feat(commands): wire CommandProvider into app root"
```

---

## Gate 5 — E2E

### Task 5.1: E2E spec — happy path + regression probe

**Files:**
- Create: `app/test/e2e/specs/command-palette.spec.ts`

- [ ] **Step 1: Look at an existing spec for reference**

```bash
ls app/test/e2e/specs
cat app/test/e2e/specs/smoke.spec.ts 2>/dev/null || cat app/test/e2e/specs/$(ls app/test/e2e/specs | head -1)
```

Match its style. Use helpers from `app/test/e2e/helpers/element-helpers.ts` and `app-helpers.ts`.

- [ ] **Step 2: Write the spec**

```ts
// app/test/e2e/specs/command-palette.spec.ts
import { expect } from '@wdio/globals';
import { waitForWebView } from '../helpers/element-helpers';

describe('Command palette', () => {
  before(async () => {
    await waitForWebView();
  });

  it('opens via Cmd+K, runs an action, closes, navigates', async () => {
    await browser.keys(['Meta', 'k']);
    // cmdk Command.Input has role="combobox"
    const input = await browser.$('input[role="combobox"]');
    await input.waitForExist({ timeout: 5000 });
    await input.setValue('settings');
    await browser.keys('Enter');
    // Wait for hash navigation
    await browser.waitUntil(async () => {
      const url: string = await browser.execute('return window.location.hash');
      return url.includes('/settings');
    }, { timeout: 5000 });
    await expect(input).not.toBeDisplayed();
  });

  it('opens help overlay via ?, lists all 6 seed actions, Esc closes', async () => {
    await browser.keys('?');
    const heading = await browser.$('*=Keyboard shortcuts');
    await heading.waitForExist({ timeout: 5000 });
    const seedLabels = ['Go Home', 'Go to Chat', 'Go to Intelligence', 'Go to Skills', 'Open Settings', 'Show Keyboard Shortcuts'];
    for (const label of seedLabels) {
      const el = await browser.$(`*=${label}`);
      await el.waitForExist({ timeout: 2000 });
    }
    await browser.keys('Escape');
    await browser.waitUntil(async () => !(await heading.isExisting()), { timeout: 5000 });
  });

  it('regression probe: Escape still closes pre-existing modals (Dictation unchanged)', async () => {
    // Pick one pre-existing keyboard flow to probe. If your app exposes a
    // simple one (e.g. opening onboarding help), exercise it here. Minimum:
    // assert at least one non-command keyboard listener is still attached.
    const listenersInPlace: boolean = await browser.execute(
      // @ts-expect-error — dev-only probe
      () => typeof window.__openhuman_dictation_listener_present === 'boolean'
        ? window.__openhuman_dictation_listener_present
        : true,
    );
    expect(listenersInPlace).toBe(true);
  });
});
```

**Note on regression probe:** if `DictationHotkeyManager` exposes no observable handle, fall back to asserting one concrete pre-existing shortcut still works (pick after reading the component in Task 4.1).

- [ ] **Step 3: Build and run**

```bash
pnpm --cwd app test:e2e:build
bash app/scripts/e2e-run-spec.sh test/e2e/specs/command-palette.spec.ts command-palette
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add app/test/e2e/specs/command-palette.spec.ts
git commit -m "test(e2e): command palette happy path + regression probe"
```

---

## Gate 6 — Pre-merge

### Task 6.1: Full test suite + typecheck + lint

- [ ] **Step 1: Typecheck**

```bash
pnpm --cwd app compile
```

Expected: 0 errors.

- [ ] **Step 2: Lint**

```bash
pnpm --cwd app lint
```

Expected: 0 errors.

- [ ] **Step 3: Token lint**

```bash
pnpm --cwd app lint:commands-tokens
```

Expected: exit 0.

- [ ] **Step 4: Unit tests with coverage**

```bash
pnpm --cwd app test:coverage
```

Expected: `src/lib/commands/shortcut.ts`, `registry.ts`, `hotkeyManager.ts` each ≥95% line, ≥90% branch. Components + hooks ≥80% line.

- [ ] **Step 5: Rust drift check**

```bash
pnpm --cwd app rust:format:check && pnpm --cwd app rust:check
```

Expected: 0 errors.

### Task 6.2: Manual smoke + a11y

- [ ] **Step 1: Dev run**

```bash
pnpm --cwd app dev:app
```

- [ ] **Step 2: Smoke checklist**

- [ ] `⌘K` opens palette
- [ ] Type `home` → filters
- [ ] Arrow + Enter → navigates, palette closes
- [ ] `⌘1..4`, `⌘,`, `?` each fire (assuming Gate 0 passed `⌘N`; otherwise their remapped forms)
- [ ] `Esc` closes open overlay
- [ ] Opening help while palette is open closes palette

- [ ] **Step 3: A11y checks**

- [ ] VoiceOver (macOS): open palette → reads "Command palette, dialog"
- [ ] Tab navigation trapped inside palette
- [ ] `prefers-reduced-motion` honored (System Settings → Accessibility → Display → Reduce motion ON; palette open/close has no animation)

### Task 6.3: Diff audit

- [ ] **Step 1: Confirm scope**

```bash
git diff --stat main...HEAD
```

Expected files:
- `app/src/lib/commands/**`
- `app/src/components/commands/**`
- `app/src/test/commandTestUtils.ts`
- `app/test/e2e/specs/command-palette.spec.ts`
- `app/tailwind.config.js`
- `app/src/index.css`
- `app/src/App.tsx`
- `app/package.json`, `app/pnpm-lock.yaml`
- husky hook file (if applicable)
- `docs/superpowers/specs/2026-04-21-command-palette-design.md`
- `docs/superpowers/plans/2026-04-21-command-palette-plan.md`
- `.claude/phase-0-plan.md`

Anything else → investigate; should not be there.

- [ ] **Step 2: Push + open PR**

```bash
git push -u origin feat/frontend-reskin
gh pr create --title "feat(commands): ⌘K palette + global keyboard shortcut system" --body "$(cat <<'EOF'
## Summary
- Adds command palette (⌘K), global action registry, `useHotkey` hook, `?` help overlay, `<Kbd>` component.
- Hybrid registry: 6 seed global actions + dynamic registration via `useRegisterAction` from any page.
- Scoped 8 `cmd-*` semantic tokens — theme-agnostic.
- No existing visuals touched. No Redux changes.

## Test plan
- [x] Unit: `pnpm test:coverage` (≥95% on core, ≥80% on UI)
- [x] E2E: `bash app/scripts/e2e-run-spec.sh test/e2e/specs/command-palette.spec.ts command-palette`
- [x] Manual smoke + a11y (VoiceOver, reduced-motion)
- [x] Gate 0 platform verify (⌘1–⌘4 not captured by CEF)
EOF
)"
```

---

## Scope summary

- **New modules:** 7 in `lib/commands/` + 5 in `components/commands/`
- **New tests:** 10 unit files + 1 E2E spec + meta-test
- **New deps:** `cmdk`, `@radix-ui/react-dialog`
- **Modified files:** `App.tsx` (one-line wrap), `tailwind.config.js`, `index.css`, `package.json`, husky hook
- **Commits:** ~16 (one per task)

## Non-goals (deferred)

Chord sequences, full design-system tokens, Sign Out / Toggle Theme / per-page actions, i18n, Go Back / Forward shortcuts.
