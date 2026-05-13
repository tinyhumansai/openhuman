import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import MascotWindowApp from './MascotWindowApp';

describe('MascotWindowApp', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders the sleep face by default', () => {
    const { container } = render(<MascotWindowApp />);
    const host = container.querySelector('[data-face]') as HTMLElement;
    expect(host).not.toBeNull();
    expect(host.getAttribute('data-face')).toBe('sleep');
  });

  it('switches to idle face on mascot:hover-state with hovering=true', () => {
    const { container } = render(<MascotWindowApp />);
    act(() => {
      window.dispatchEvent(new CustomEvent('mascot:hover-state', { detail: { hovering: true } }));
    });
    const host = container.querySelector('[data-face]') as HTMLElement;
    expect(host.getAttribute('data-face')).toBe('idle');
  });

  it('returns to sleep face after 2s when hovering=false', () => {
    const { container } = render(<MascotWindowApp />);

    // First hover in to get to idle.
    act(() => {
      window.dispatchEvent(new CustomEvent('mascot:hover-state', { detail: { hovering: true } }));
    });
    expect((container.querySelector('[data-face]') as HTMLElement).getAttribute('data-face')).toBe(
      'idle'
    );

    // Hover out — should still be idle before the delay elapses.
    act(() => {
      window.dispatchEvent(new CustomEvent('mascot:hover-state', { detail: { hovering: false } }));
    });
    expect((container.querySelector('[data-face]') as HTMLElement).getAttribute('data-face')).toBe(
      'idle'
    );

    // Advance past the 2-second sleep delay.
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    expect((container.querySelector('[data-face]') as HTMLElement).getAttribute('data-face')).toBe(
      'sleep'
    );
  });

  it('cancels the sleep timeout when hovering again before delay elapses', () => {
    const { container } = render(<MascotWindowApp />);

    // Hover in → out → back in quickly.
    act(() => {
      window.dispatchEvent(new CustomEvent('mascot:hover-state', { detail: { hovering: true } }));
    });
    act(() => {
      window.dispatchEvent(new CustomEvent('mascot:hover-state', { detail: { hovering: false } }));
    });
    // Advance only 1s — still within the sleep delay.
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    // Hover back in — should cancel the pending sleep timer.
    act(() => {
      window.dispatchEvent(new CustomEvent('mascot:hover-state', { detail: { hovering: true } }));
    });
    // Advance past the original deadline — sleep timer should have been cancelled.
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    expect((container.querySelector('[data-face]') as HTMLElement).getAttribute('data-face')).toBe(
      'idle'
    );
  });
});
