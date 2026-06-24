import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Tooltip from './Tooltip';

describe('Tooltip', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
  });

  it('does not show the pill until the trigger is hovered past the delay', () => {
    render(
      <Tooltip label="Wallet" delayMs={300}>
        <button type="button" aria-label="Wallet">
          icon
        </button>
      </Tooltip>
    );
    expect(screen.queryByTestId('tooltip')).toBeNull();

    fireEvent.mouseEnter(screen.getByRole('button', { name: 'Wallet' }));
    // Before the dwell elapses, nothing is shown.
    act(() => {
      vi.advanceTimersByTime(200);
    });
    expect(screen.queryByTestId('tooltip')).toBeNull();

    act(() => {
      vi.advanceTimersByTime(100);
    });
    expect(screen.getByTestId('tooltip')).toHaveTextContent('Wallet');
  });

  it('hides the pill on mouse leave and cancels a pending show', () => {
    render(
      <Tooltip label="Settings">
        <button type="button" aria-label="Settings">
          icon
        </button>
      </Tooltip>
    );
    const btn = screen.getByRole('button', { name: 'Settings' });

    fireEvent.mouseEnter(btn);
    fireEvent.mouseLeave(btn);
    act(() => {
      vi.advanceTimersByTime(500);
    });
    // Leaving before the delay cancels the show entirely.
    expect(screen.queryByTestId('tooltip')).toBeNull();
  });

  it('shows on keyboard focus and hides on blur', () => {
    render(
      <Tooltip label="Home" delayMs={0}>
        <button type="button" aria-label="Home">
          icon
        </button>
      </Tooltip>
    );
    const btn = screen.getByRole('button', { name: 'Home' });

    fireEvent.focus(btn);
    act(() => {
      vi.advanceTimersByTime(0);
    });
    expect(screen.getByTestId('tooltip')).toHaveTextContent('Home');

    fireEvent.blur(btn);
    expect(screen.queryByTestId('tooltip')).toBeNull();
  });

  it('preserves the trigger’s own handlers', () => {
    const onMouseEnter = vi.fn();
    const onClick = vi.fn();
    render(
      <Tooltip label="Chat" delayMs={0}>
        <button type="button" aria-label="Chat" onMouseEnter={onMouseEnter} onClick={onClick}>
          icon
        </button>
      </Tooltip>
    );
    const btn = screen.getByRole('button', { name: 'Chat' });

    fireEvent.mouseEnter(btn);
    fireEvent.click(btn);
    expect(onMouseEnter).toHaveBeenCalledTimes(1);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('marks the pill aria-hidden so screen readers use the trigger label', () => {
    render(
      <Tooltip label="Connections" delayMs={0}>
        <button type="button" aria-label="Connections">
          icon
        </button>
      </Tooltip>
    );
    fireEvent.mouseEnter(screen.getByRole('button', { name: 'Connections' }));
    act(() => {
      vi.advanceTimersByTime(0);
    });
    expect(screen.getByTestId('tooltip')).toHaveAttribute('aria-hidden', 'true');
  });

  it.each([
    ['right', 'translateY(-50%)'],
    ['left', 'translate(-100%, -50%)'],
    ['top', 'translate(-50%, -100%)'],
    ['bottom', 'translate(-50%, 0)'],
  ] as const)('positions the pill against the %s edge', (side, transform) => {
    render(
      <Tooltip label="Brain" side={side} delayMs={0}>
        <button type="button" aria-label="Brain">
          icon
        </button>
      </Tooltip>
    );
    fireEvent.mouseEnter(screen.getByRole('button', { name: 'Brain' }));
    act(() => {
      vi.advanceTimersByTime(0);
    });
    expect(screen.getByTestId('tooltip')).toHaveStyle({ transform });
  });

  it('renders an invalid (non-element) child as-is without a pill', () => {
    render(<Tooltip label="Noop">{'just text' as never}</Tooltip>);
    expect(screen.getByText('just text')).toBeInTheDocument();
    expect(screen.queryByTestId('tooltip')).toBeNull();
  });

  it('applies a native title fallback on the trigger (CEF-occlusion safety net)', () => {
    render(
      <Tooltip label="Wallet" delayMs={0}>
        <button type="button" aria-label="Wallet">
          icon
        </button>
      </Tooltip>
    );
    expect(screen.getByRole('button', { name: 'Wallet' })).toHaveAttribute('title', 'Wallet');
  });

  it('respects a trigger-supplied title over the label fallback', () => {
    render(
      <Tooltip label="Wallet" delayMs={0}>
        <button type="button" aria-label="Wallet" title="Custom">
          icon
        </button>
      </Tooltip>
    );
    expect(screen.getByRole('button', { name: 'Wallet' })).toHaveAttribute('title', 'Custom');
  });

  it('clears a pending show when the trigger unmounts mid-hover', () => {
    const { unmount } = render(
      <Tooltip label="Human" delayMs={300}>
        <button type="button" aria-label="Human">
          icon
        </button>
      </Tooltip>
    );
    fireEvent.mouseEnter(screen.getByRole('button', { name: 'Human' }));
    unmount();
    // The timer must not fire setState on the unmounted component.
    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(screen.queryByTestId('tooltip')).toBeNull();
  });
});
