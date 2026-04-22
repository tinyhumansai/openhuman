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
    return `${version}:${stack.map((s) => s.description ?? '?').join('>')}:${stack.length}`;
  }

  function registerAction(action: Action, scopeFrame: symbol): () => void {
    let frame = byFrame.get(scopeFrame);
    if (!frame) {
      frame = new Map();
      byFrame.set(scopeFrame, frame);
    }
    if (frame.has(action.id)) {
      // eslint-disable-next-line no-console
      console.warn(
        `[commands] duplicate action id "${action.id}" in the same scope — replacing`,
      );
    }
    const registered: RegisteredAction = { ...action, scopeFrame };
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
        r.catch((err) => console.error('[commands] action rejected', id, err));
    } catch (err) {
      console.error('[commands] action threw', id, err);
    }
    return true;
  }

  function setActiveStack(stack: symbol[]): void {
    activeStack = [...stack];
  }

  return { registerAction, getAction, getActiveActions, subscribe, runAction, setActiveStack };
}

export const registry = createRegistry();
