import { afterEach, describe, expect, it, vi } from 'vitest';

import { __resetChatSurfaces, getChatSurface, registerChatSurface } from '../chatSurfaceHandlers';

afterEach(() => __resetChatSurfaces());

describe('chatSurfaceHandlers', () => {
  it('returns null for an unknown or absent thread', () => {
    expect(getChatSurface('nope')).toBeNull();
    expect(getChatSurface(null)).toBeNull();
  });

  it('resolves the handlers registered for a thread', () => {
    const handlers = { send: vi.fn(async () => {}) };
    registerChatSurface('t1', handlers);
    expect(getChatSurface('t1')).toBe(handlers);
  });

  it('keeps an explicitly registered unscoped surface for a new chat', () => {
    const handlers = { send: vi.fn(async () => {}) };
    registerChatSurface(null, handlers);
    expect(getChatSurface(null)).toBe(handlers);
  });

  it('keeps threads isolated from one another', () => {
    const a = { send: vi.fn(async () => {}) };
    const b = { send: vi.fn(async () => {}) };
    registerChatSurface('t1', a);
    registerChatSurface('t2', b);
    expect(getChatSurface('t1')).toBe(a);
    expect(getChatSurface('t2')).toBe(b);
  });

  it('unregisters on cleanup', () => {
    const unregister = registerChatSurface('t1', { send: vi.fn(async () => {}) });
    unregister();
    expect(getChatSurface('t1')).toBeNull();
  });

  it('does not let a stale cleanup evict a newer registration', () => {
    // Remount ordering: the new instance registers before the old one's effect
    // cleanup runs. The outgoing cleanup must be a no-op.
    const oldCleanup = registerChatSurface('t1', { send: vi.fn(async () => {}) });
    const fresh = { send: vi.fn(async () => {}) };
    registerChatSurface('t1', fresh);
    oldCleanup();
    expect(getChatSurface('t1')).toBe(fresh);
  });
});
