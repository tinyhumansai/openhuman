import { matchEvent, parseShortcut } from './shortcut';
import type { ActiveBinding, HotkeyBinding, ScopeFrame, ScopeKind } from './types';

interface FrameInternal extends ScopeFrame {
  bindings: Map<symbol, { binding: HotkeyBinding; parsed: ReturnType<typeof parseShortcut> }>;
}

function isEditableTarget(e: KeyboardEvent): boolean {
  const path = e.composedPath?.() || [];
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
    // biome-ignore lint/suspicious/noExplicitAny: checking for browser-specific properties
    const composing = (e as any).isComposing === true || (e as any).keyCode === 229;
    if (composing) return;

    const inEditable = isEditableTarget(e);

    // Snapshot frames + bindings so handlers that push/pop frames
    // or bind/unbind during dispatch can't corrupt iteration.
    const frames = stack.slice();
    for (let i = frames.length - 1; i >= 0; i--) {
      const frame = frames[i];
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

  function bindingDedupKey(parsed: ReturnType<typeof parseShortcut>): string {
    const flags =
      (parsed.mod ? 'M' : '') +
      (parsed.ctrl ? 'C' : '') +
      (parsed.shift ? 'S' : '') +
      (parsed.alt ? 'A' : '');
    return `${flags}|${parsed.key}`;
  }

  function getActiveBindings(): ActiveBinding[] {
    const out: ActiveBinding[] = [];
    const seen = new Set<string>();
    // Walk top-of-stack downwards so inner scopes shadow outer ones.
    for (let i = stack.length - 1; i >= 0; i--) {
      const frame = stack[i];
      for (const { binding, parsed } of frame.bindings.values()) {
        if (binding.enabled && !binding.enabled()) continue;
        const key = bindingDedupKey(parsed);
        if (seen.has(key)) continue;
        seen.add(key);
        out.push({
          frame: { symbol: frame.symbol, id: frame.id, kind: frame.kind },
          binding,
          parsed,
        });
      }
    }
    return out;
  }

  function subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }

  return {
    init,
    teardown,
    pushFrame,
    popFrame,
    bind,
    unbind,
    getStackSymbols,
    getActiveBindings,
    subscribe,
  };
}

export const hotkeyManager = createHotkeyManager();
