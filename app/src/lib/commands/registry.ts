import { parseShortcut } from './shortcut';
import type { Action, RegisteredAction } from './types';

export interface Registry {
  registerAction: (action: Action, scopeFrame: symbol) => () => void;
  getAction: (id: string) => RegisteredAction | undefined;
  getActiveActions: (scopeStack: symbol[]) => RegisteredAction[];
  subscribe: (listener: () => void) => () => void;
  runAction: (id: string) => boolean;
  setActiveStack: (stack: symbol[]) => void;
  reset: () => void;
}

function shortcutDedupKey(shortcut: string): string {
  try {
    const p = parseShortcut(shortcut);
    const flags =
      (p.mod ? 'M' : '') + (p.ctrl ? 'C' : '') + (p.shift ? 'S' : '') + (p.alt ? 'A' : '');
    return `${flags}|${p.key}`;
  } catch {
    return `raw|${shortcut}`;
  }
}

export function createRegistry(): Registry {
  const byFrame = new Map<symbol, Map<string, RegisteredAction>>();
  const listeners = new Set<() => void>();
  let version = 0;
  const snapshotCache = new Map<string, RegisteredAction[]>();
  let activeStack: symbol[] = [];
  const symbolIds = new Map<symbol, number>();
  let nextSymbolId = 1;

  function getSymbolId(sym: symbol): number {
    let id = symbolIds.get(sym);
    if (!id) {
      id = nextSymbolId++;
      symbolIds.set(sym, id);
    }
    return id;
  }

  function bump(): void {
    version += 1;
    snapshotCache.clear();
    for (const l of listeners) l();
  }

  function stackKey(stack: symbol[]): string {
    return `${version}:${stack.map(getSymbolId).join('>')}`;
  }

  function registerAction(action: Action, scopeFrame: symbol): () => void {
    let frame = byFrame.get(scopeFrame);
    if (!frame) {
      frame = new Map();
      byFrame.set(scopeFrame, frame);
    }
    if (frame.has(action.id)) {
      console.warn(`[commands] duplicate action id "${action.id}" in the same scope — replacing`);
    }
    const registered: RegisteredAction = { ...action, scopeFrame };
    if (action.shortcut) parseShortcut(action.shortcut);
    frame.set(action.id, registered);
    bump();
    return () => {
      const f = byFrame.get(scopeFrame);
      if (!f) return;
      if (f.get(action.id) !== registered) return;
      if (f.delete(action.id)) {
        if (f.size === 0) byFrame.delete(scopeFrame);
        bump();
      }
    };
  }

  function getAction(id: string): RegisteredAction | undefined {
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
    const seenId = new Set<string>();
    const seenShortcut = new Set<string>();
    const out: RegisteredAction[] = [];
    for (let i = scopeStack.length - 1; i >= 0; i--) {
      const frame = byFrame.get(scopeStack[i]);
      if (!frame) continue;
      for (const action of frame.values()) {
        if (seenId.has(action.id)) continue;
        if (action.enabled && !action.enabled()) continue;
        if (action.shortcut) {
          const sk = shortcutDedupKey(action.shortcut);
          if (seenShortcut.has(sk)) {
            seenId.add(action.id);
            continue;
          }
          seenShortcut.add(sk);
        }
        seenId.add(action.id);
        out.push(action);
      }
    }
    snapshotCache.set(key, out);
    return out;
  }

  function subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }

  function runAction(id: string): boolean {
    const action = getAction(id);
    if (!action) return false;
    if (action.enabled && !action.enabled()) return false;
    try {
      const r = action.handler();
      if (r instanceof Promise)
        r.catch(err => console.error('[commands] action rejected', id, err));
    } catch (err) {
      console.error('[commands] action threw', id, err);
    }
    return true;
  }

  function setActiveStack(stack: symbol[]): void {
    const changed =
      stack.length !== activeStack.length || stack.some((sym, index) => sym !== activeStack[index]);
    if (!changed) return;
    activeStack = [...stack];
    snapshotCache.clear();
    for (const l of listeners) l();
  }

  function reset(): void {
    byFrame.clear();
    listeners.clear();
    snapshotCache.clear();
    activeStack = [];
    symbolIds.clear();
    nextSymbolId = 1;
    version = 0;
  }

  return {
    registerAction,
    getAction,
    getActiveActions,
    subscribe,
    runAction,
    setActiveStack,
    reset,
  };
}

export const registry = createRegistry();
