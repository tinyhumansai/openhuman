import { describe, expect, it, vi } from 'vitest';

import { ChatMascotSendStore } from './sendBinding';

const binding = (disabled = false) => ({ submit: vi.fn(), onError: vi.fn(), disabled });

describe('ChatMascotSendStore', () => {
  it('starts unbound', () => {
    expect(new ChatMascotSendStore().get()).toBeNull();
  });

  it('notifies subscribers when the binding changes', () => {
    const store = new ChatMascotSendStore();
    const listener = vi.fn();
    store.subscribe(listener);

    const b = binding();
    store.set(b);

    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.get()).toBe(b);
  });

  it('does not notify when nothing observable changed', () => {
    // `Conversations` re-publishes on every render; waking the stage each time
    // would re-render MicComposer on every keystroke in the text composer.
    const store = new ChatMascotSendStore();
    const b = binding();
    store.set(b);

    const listener = vi.fn();
    store.subscribe(listener);
    store.set({ submit: b.submit, onError: b.onError, disabled: b.disabled });

    expect(listener).not.toHaveBeenCalled();
  });

  it('notifies when only the disabled flag flips', () => {
    const store = new ChatMascotSendStore();
    const b = binding(false);
    store.set(b);

    const listener = vi.fn();
    store.subscribe(listener);
    store.set({ ...b, disabled: true });

    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.get()?.disabled).toBe(true);
  });

  it('stops notifying after unsubscribe', () => {
    const store = new ChatMascotSendStore();
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);
    unsubscribe();

    store.set(binding());

    expect(listener).not.toHaveBeenCalled();
  });

  it('clears the binding on unbind', () => {
    const store = new ChatMascotSendStore();
    store.set(binding());
    const listener = vi.fn();
    store.subscribe(listener);

    store.set(null);

    expect(store.get()).toBeNull();
    expect(listener).toHaveBeenCalledTimes(1);
  });
});
