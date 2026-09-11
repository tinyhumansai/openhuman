import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { installAutoHideScrollbars, SCROLL_IDLE_MS, SCROLLING_ATTR } from './autoHideScrollbars';

/** Dispatch a non-bubbling `scroll` event the way a real scroll pane does. */
function scroll(el: Element) {
  el.dispatchEvent(new Event('scroll', { bubbles: false }));
}

describe('installAutoHideScrollbars', () => {
  let dispose: () => void;
  let pane: HTMLDivElement;

  beforeEach(() => {
    vi.useFakeTimers();
    pane = document.createElement('div');
    document.body.appendChild(pane);
    dispose = installAutoHideScrollbars(document);
  });

  afterEach(() => {
    dispose();
    pane.remove();
    vi.useRealTimers();
  });

  it('marks a pane as scrolling', () => {
    scroll(pane);
    expect(pane.hasAttribute(SCROLLING_ATTR)).toBe(true);
  });

  it('clears the mark once the pane goes idle', () => {
    scroll(pane);
    vi.advanceTimersByTime(SCROLL_IDLE_MS + 1);
    expect(pane.hasAttribute(SCROLLING_ATTR)).toBe(false);
  });

  it('keeps the mark while scrolling continues', () => {
    scroll(pane);
    // Each further scroll must restart the idle countdown, not stack timers —
    // otherwise a long scroll would hide the bar mid-gesture.
    for (let i = 0; i < 5; i++) {
      vi.advanceTimersByTime(SCROLL_IDLE_MS - 50);
      scroll(pane);
    }
    expect(pane.hasAttribute(SCROLLING_ATTR)).toBe(true);
    vi.advanceTimersByTime(SCROLL_IDLE_MS + 1);
    expect(pane.hasAttribute(SCROLLING_ATTR)).toBe(false);
  });

  it('tracks panes independently', () => {
    const other = document.createElement('div');
    document.body.appendChild(other);
    scroll(pane);
    vi.advanceTimersByTime(SCROLL_IDLE_MS - 50);
    scroll(other);
    vi.advanceTimersByTime(100);

    // `pane` has gone idle; `other` scrolled more recently and is still marked.
    expect(pane.hasAttribute(SCROLLING_ATTR)).toBe(false);
    expect(other.hasAttribute(SCROLLING_ATTR)).toBe(true);
    other.remove();
  });

  it('attributes a document-level scroll to the root element', () => {
    document.dispatchEvent(new Event('scroll', { bubbles: false }));
    expect(document.documentElement.hasAttribute(SCROLLING_ATTR)).toBe(true);
  });

  it('catches panes mounted after install (capture listener, no per-pane wiring)', () => {
    const late = document.createElement('div');
    document.body.appendChild(late);
    scroll(late);
    expect(late.hasAttribute(SCROLLING_ATTR)).toBe(true);
    late.remove();
  });

  it('stops marking and clears pending state once disposed', () => {
    scroll(pane);
    dispose();
    expect(pane.hasAttribute(SCROLLING_ATTR)).toBe(false);

    scroll(pane);
    expect(pane.hasAttribute(SCROLLING_ATTR)).toBe(false);

    // Re-installed in afterEach's dispose contract; make it a no-op double call.
    dispose = () => {};
  });
});
