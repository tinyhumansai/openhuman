import { fireEvent, render } from '@testing-library/react';
import { createElement } from 'react';
import { describe, expect, it } from 'vitest';

import { isNearBottom, useStickToBottom } from './useStickToBottom';

/**
 * jsdom performs no real layout, so `scrollHeight`/`clientHeight`/`scrollTop`
 * are inert (always 0) unless we define them ourselves. This gives the
 * container concrete, mutable scroll metrics so `isNearBottom` and
 * `snapToBottom` (inside the hook) behave exactly as they would in a real
 * browser for the geometry each test sets up.
 */
function mockScrollMetrics(
  el: HTMLElement,
  metrics: { scrollTop: number; scrollHeight: number; clientHeight: number }
) {
  Object.defineProperty(el, 'scrollHeight', { configurable: true, value: metrics.scrollHeight });
  Object.defineProperty(el, 'clientHeight', { configurable: true, value: metrics.clientHeight });
  Object.defineProperty(el, 'scrollTop', {
    configurable: true,
    writable: true,
    value: metrics.scrollTop,
  });
}

interface HarnessProps {
  messages: unknown[];
  threadKey: string | null;
  resetKey: string;
}

/** Minimal host component — the hook never touches the DOM directly, callers
 * wire `containerRef` onto their own scroll container. */
function Harness({ messages, threadKey, resetKey }: HarnessProps) {
  const { containerRef } = useStickToBottom(messages, threadKey, resetKey);
  return createElement(
    'div',
    { ref: containerRef, 'data-testid': 'scroll-container' },
    messages.map((m, i) => createElement('div', { key: i }, String(m)))
  );
}

describe('useStickToBottom', () => {
  it('is a pure, independently-testable is-at-bottom check', () => {
    // Exactly at the default 80px threshold: still "at bottom".
    expect(
      isNearBottom({ scrollHeight: 180, scrollTop: 100, clientHeight: 80 } as HTMLElement)
    ).toBe(true);
    // One pixel past the threshold (distance = 181 - 20 - 80 = 81): no
    // longer "at bottom".
    expect(
      isNearBottom({ scrollHeight: 181, scrollTop: 20, clientHeight: 80 } as HTMLElement)
    ).toBe(false);
    // A custom threshold is honoured.
    expect(
      isNearBottom({ scrollHeight: 300, scrollTop: 100, clientHeight: 80 } as HTMLElement, 200)
    ).toBe(true);
  });

  it('auto-scrolls to the newest content when the user is already pinned to the bottom', () => {
    const { getByTestId, rerender } = render(
      createElement(Harness, { messages: ['first'], threadKey: 't1', resetKey: 'k' })
    );
    const container = getByTestId('scroll-container');

    // Establish "at the bottom" after the initial (always-snaps) mount.
    mockScrollMetrics(container, { scrollTop: 50, scrollHeight: 100, clientHeight: 50 });
    rerender(createElement(Harness, { messages: ['first'], threadKey: 't1', resetKey: 'k' }));

    // A new message arrives — content grows (scrollHeight increases) while
    // the user never moved away from the bottom. Because `stickingRef` is
    // still true, the layout effect must snap to the new bottom.
    mockScrollMetrics(container, { scrollTop: 50, scrollHeight: 300, clientHeight: 50 });
    rerender(
      createElement(Harness, { messages: ['first', 'second'], threadKey: 't1', resetKey: 'k' })
    );

    expect(container.scrollTop).toBe(300);
  });

  it('does NOT auto-scroll (respects the user reading history) once they scroll away from the bottom', () => {
    const { getByTestId, rerender } = render(
      createElement(Harness, { messages: ['first'], threadKey: 't1', resetKey: 'k' })
    );
    const container = getByTestId('scroll-container');

    mockScrollMetrics(container, { scrollTop: 50, scrollHeight: 100, clientHeight: 50 });
    rerender(createElement(Harness, { messages: ['first'], threadKey: 't1', resetKey: 'k' }));

    // The user scrolls up, well past the stick threshold (400 - 0 - 50 =
    // 350px from the bottom) — the `scroll` listener must disengage sticking.
    mockScrollMetrics(container, { scrollTop: 0, scrollHeight: 400, clientHeight: 50 });
    fireEvent.scroll(container);

    // A new message arrives (content grows further) while the user is still
    // reading up top. This is exactly the bug being fixed: the old
    // unconditional `scrollTo` effect would yank the user back down here —
    // the container must stay exactly where the reader left it.
    mockScrollMetrics(container, { scrollTop: 0, scrollHeight: 700, clientHeight: 50 });
    rerender(
      createElement(Harness, { messages: ['first', 'second'], threadKey: 't1', resetKey: 'k' })
    );

    expect(container.scrollTop).toBe(0);
  });

  it('re-engages sticking once the user manually scrolls back to the bottom', () => {
    const { getByTestId, rerender } = render(
      createElement(Harness, { messages: ['first'], threadKey: 't1', resetKey: 'k' })
    );
    const container = getByTestId('scroll-container');

    mockScrollMetrics(container, { scrollTop: 50, scrollHeight: 100, clientHeight: 50 });
    rerender(createElement(Harness, { messages: ['first'], threadKey: 't1', resetKey: 'k' }));

    // Scroll away (350px from bottom — well past the threshold).
    mockScrollMetrics(container, { scrollTop: 0, scrollHeight: 400, clientHeight: 50 });
    fireEvent.scroll(container);

    // Scroll back within the threshold (400 - 350 - 50 = 0px from bottom).
    mockScrollMetrics(container, { scrollTop: 350, scrollHeight: 400, clientHeight: 50 });
    fireEvent.scroll(container);

    // A new message now DOES pull the view down again.
    mockScrollMetrics(container, { scrollTop: 350, scrollHeight: 600, clientHeight: 50 });
    rerender(
      createElement(Harness, { messages: ['first', 'second'], threadKey: 't1', resetKey: 'k' })
    );

    expect(container.scrollTop).toBe(600);
  });

  it('always snaps on a thread change, even mid-read (a fresh conversation should open at the bottom)', () => {
    const { getByTestId, rerender } = render(
      createElement(Harness, { messages: ['a1'], threadKey: 'thread-a', resetKey: 'k' })
    );
    const container = getByTestId('scroll-container');
    mockScrollMetrics(container, { scrollTop: 50, scrollHeight: 100, clientHeight: 50 });
    rerender(createElement(Harness, { messages: ['a1'], threadKey: 'thread-a', resetKey: 'k' }));

    // Scroll away on thread A.
    mockScrollMetrics(container, { scrollTop: 0, scrollHeight: 400, clientHeight: 50 });
    fireEvent.scroll(container);

    // Switch to a different thread entirely — must snap regardless of the
    // stale `stickingRef` from thread A.
    mockScrollMetrics(container, { scrollTop: 0, scrollHeight: 250, clientHeight: 50 });
    rerender(createElement(Harness, { messages: ['b1'], threadKey: 'thread-b', resetKey: 'k' }));

    expect(container.scrollTop).toBe(250);
  });
});
