import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { MemoryControls } from './MemoryControls';

// Stub i18n and the vault sub-section so we render just the control bar.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('./ObsidianVaultSection', () => ({ ObsidianVaultSection: () => null }));
vi.mock('../../utils/tauriCommands', () => ({
  memoryTreeFlushNow: vi.fn().mockResolvedValue({ enqueued: true, stale_buffers: 0 }),
  memoryTreeResetTree: vi.fn().mockResolvedValue(undefined),
  memoryTreeWipeAll: vi.fn().mockResolvedValue(undefined),
}));

const noop = () => {};

describe('<MemoryControls /> refresh feedback', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('calls onRefresh and shows a busy spinner immediately on click', async () => {
    const onRefresh = vi.fn();
    render(<MemoryControls mode="tree" onModeChange={noop} onRefresh={onRefresh} />);

    const btn = screen.getByTestId('memory-graph-refresh');
    expect(btn).not.toBeDisabled();

    fireEvent.click(btn);

    // The parent re-pull fires right away…
    expect(onRefresh).toHaveBeenCalledTimes(1);
    // …and the button reports a busy state so the click is visibly acknowledged.
    expect(btn).toBeDisabled();
    expect(btn.getAttribute('aria-busy')).toBe('true');
    expect(btn.querySelector('.animate-spin')).not.toBeNull();
  });

  it('clears the busy state after the minimum spin window', async () => {
    const onRefresh = vi.fn();
    render(<MemoryControls mode="tree" onModeChange={noop} onRefresh={onRefresh} />);

    const btn = screen.getByTestId('memory-graph-refresh');
    fireEvent.click(btn);
    expect(btn).toBeDisabled();

    // Advance past the 600ms minimum spin window; the button re-enables.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });
    expect(btn).not.toBeDisabled();
    expect(btn.getAttribute('aria-busy')).toBe('false');
  });

  it('ignores re-clicks while already refreshing', async () => {
    const onRefresh = vi.fn();
    render(<MemoryControls mode="tree" onModeChange={noop} onRefresh={onRefresh} />);

    const btn = screen.getByTestId('memory-graph-refresh');
    fireEvent.click(btn);
    // A disabled button shouldn't re-fire, but guard against programmatic clicks too.
    fireEvent.click(btn);
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });
});
