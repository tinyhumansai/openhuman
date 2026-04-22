import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { createHotkeyManager } from '../hotkeyManager';

function dispatchKey(key: string, opts: Partial<KeyboardEventInit> = {}): KeyboardEvent {
  const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...opts });
  window.dispatchEvent(e);
  return e;
}

describe('hotkeyManager', () => {
  let mgr: ReturnType<typeof createHotkeyManager>;
  beforeEach(() => {
    mgr = createHotkeyManager();
    mgr.init();
  });
  afterEach(() => {
    mgr.teardown();
  });

  it('init is idempotent', () => {
    const listenerSpy = vi.spyOn(window, 'addEventListener');
    mgr.init();
    const keydownCalls = listenerSpy.mock.calls.filter((c) => c[0] === 'keydown');
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
    await new Promise((r) => setTimeout(r, 0));
    expect(err).toHaveBeenCalled();
    err.mockRestore();
  });

  it('unregister during dispatch does not crash or double-fire', () => {
    const f = mgr.pushFrame('global', 'root');
    const b = vi.fn();
    const bindBSym = mgr.bind(f, { shortcut: 'k', handler: b });
    mgr.bind(f, { shortcut: 'k', handler: () => mgr.unbind(f, bindBSym) });
    dispatchKey('k');
    expect(b).not.toHaveBeenCalled();
  });

  it('pop frame during dispatch does not crash', () => {
    const f = mgr.pushFrame('modal', 'dialog');
    mgr.bind(f, { shortcut: 'escape', handler: () => mgr.popFrame(f) });
    expect(() => dispatchKey('Escape')).not.toThrow();
  });
});
