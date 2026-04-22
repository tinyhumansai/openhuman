import { describe, it, expect, beforeEach, vi } from 'vitest';
import { createRegistry } from '../registry';
import type { Action } from '../types';

const baseAction: Action = { id: 'a.test', label: 'Test', handler: vi.fn() };

describe('registry', () => {
  let reg: ReturnType<typeof createRegistry>;
  beforeEach(() => {
    reg = createRegistry();
  });

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
    expect(active.filter((a) => a.id === 'a.test')).toHaveLength(1);
    expect(active.find((a) => a.id === 'a.test')?.label).toBe('page');
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
