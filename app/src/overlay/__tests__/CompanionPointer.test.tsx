import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import CompanionPointer, { type PointTarget } from '../CompanionPointer';

describe('CompanionPointer', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    // Make rAF fire synchronously so we can observe the visible→true transition
    // without juggling async/await boundaries in every case.
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(cb => {
      cb(0);
      return 1;
    });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(() => undefined);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('renders nothing when targets is empty', () => {
    const { container } = render(<CompanionPointer targets={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it('renders one label per target with the label text', () => {
    const targets: PointTarget[] = [
      { absolute_x: 10, absolute_y: 20, label: 'Click here' },
      { absolute_x: 30, absolute_y: 40, label: 'Then this' },
    ];
    render(<CompanionPointer targets={targets} />);
    expect(screen.getByText('Click here')).toBeInTheDocument();
    expect(screen.getByText('Then this')).toBeInTheDocument();
  });

  it('auto-dismisses after dismissMs elapses', () => {
    const { container } = render(
      <CompanionPointer
        targets={[{ absolute_x: 0, absolute_y: 0, label: 'Vanish' }]}
        dismissMs={500}
      />
    );
    expect(screen.getByText('Vanish')).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(500);
    });

    expect(container.firstChild).toBeNull();
  });

  it('uses default dismissMs of 2000 when not provided', () => {
    const { container } = render(
      <CompanionPointer targets={[{ absolute_x: 0, absolute_y: 0, label: 'Default' }]} />
    );
    act(() => {
      vi.advanceTimersByTime(1999);
    });
    expect(screen.getByText('Default')).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(container.firstChild).toBeNull();
  });

  it('cancels rAF and timer on unmount', () => {
    const cancelTimerSpy = vi.spyOn(window, 'clearTimeout');
    const cancelRafSpy = vi.spyOn(window, 'cancelAnimationFrame');

    const { unmount } = render(
      <CompanionPointer targets={[{ absolute_x: 0, absolute_y: 0, label: 'A' }]} />
    );
    unmount();

    expect(cancelRafSpy).toHaveBeenCalled();
    expect(cancelTimerSpy).toHaveBeenCalled();
  });
});
