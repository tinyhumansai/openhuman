/**
 * Vitest for `<MemoryTreeStatusPanel />`. Covers the four-tile dashboard,
 * the toggle round-trip (calls `memoryTreeSetEnabled` and re-fetches),
 * paused/error rendering branches, and the failure → retry path.
 *
 * Fake timers are pinned in `beforeEach` so `Date.now()` (in
 * `formatRelativeMs` and the `payload()` helper) yields the same value on
 * every assertion, and the polling `setTimeout` cannot race CI runners.
 * The first `fetchOnce()` still resolves as a microtask via `waitFor`, so
 * the suite never needs to advance timers manually.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { MemoryTreePipelineStatus } from '../../utils/tauriCommands';
import { MemoryTreeStatusPanel } from './MemoryTreeStatusPanel';

const mockPipelineStatus = vi.fn();
const mockSetEnabled = vi.fn();

vi.mock('../../utils/tauriCommands', async importOriginal => {
  // Inherit everything else (types, sibling wrappers) verbatim so the panel
  // sees the same module shape as production — only the two new helpers
  // under test get swapped for spies.
  const actual = await importOriginal<typeof import('../../utils/tauriCommands')>();
  return {
    ...actual,
    memoryTreePipelineStatus: (...args: unknown[]) => mockPipelineStatus(...args),
    memoryTreeSetEnabled: (...args: unknown[]) => mockSetEnabled(...args),
  };
});

/** Stable wall-clock used by every test in this file. */
const FIXED_NOW_MS = new Date('2026-01-01T12:00:00.000Z').getTime();

function payload(overrides: Partial<MemoryTreePipelineStatus> = {}): MemoryTreePipelineStatus {
  return {
    status: 'running',
    reason: null,
    last_sync_ms: FIXED_NOW_MS - 5 * 60 * 1000, // 5 minutes ago (stable under fake timers)
    total_chunks: 1234,
    wiki_size_bytes: 2 * 1024 * 1024, // 2 MiB
    pipeline_jobs: { ready: 0, running: 0, failed: 0 },
    is_syncing: false,
    is_paused: false,
    ...overrides,
  };
}

describe('<MemoryTreeStatusPanel />', () => {
  beforeEach(() => {
    // `shouldAdvanceTime` lets fake `setTimeout`/`setInterval` tick at the
    // real-time cadence so RTL's `waitFor` (and the panel's polling loop)
    // make progress without each test having to call
    // `vi.advanceTimersByTime` manually — while `setSystemTime` still
    // freezes `Date.now()` so `formatRelativeMs` resolves deterministically.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date(FIXED_NOW_MS));
    mockPipelineStatus.mockReset();
    mockSetEnabled.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('renders the four tiles with formatted values once the status loads', async () => {
    mockPipelineStatus.mockResolvedValueOnce(payload());
    render(<MemoryTreeStatusPanel />);

    // Status label flows in from the wire status.
    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-status-label')).toHaveTextContent(/running/i);
    });

    // Number is thousands-separated via Intl.NumberFormat.
    expect(screen.getByTestId('memory-tree-total-chunks')).toHaveTextContent('1,234');
    // 2 MiB formatter renders "2.0 MiB".
    expect(screen.getByTestId('memory-tree-wiki-size')).toHaveTextContent(/2\.0 MiB/);
    // last_sync_ms ~5 min ago bucketed to "5 min ago".
    expect(screen.getByTestId('memory-tree-last-sync')).toHaveTextContent(/min ago/);
  });

  it('shows skeleton placeholders before the first status payload resolves', async () => {
    // Suspend the promise so the panel paints its loading state.
    let resolve: (v: MemoryTreePipelineStatus) => void = () => {};
    mockPipelineStatus.mockReturnValueOnce(
      new Promise<MemoryTreePipelineStatus>(r => {
        resolve = r;
      })
    );
    render(<MemoryTreeStatusPanel />);
    // Tiles container is on-screen; status-label has not been written yet.
    expect(screen.getByTestId('memory-tree-status-tiles')).toBeInTheDocument();
    expect(screen.queryByTestId('memory-tree-status-label')).toBeNull();
    // Resolve to unblock the cleanup path.
    await act(async () => {
      resolve(payload());
    });
  });

  it('toggles auto-sync by calling memoryTreeSetEnabled and re-fetching', async () => {
    mockPipelineStatus
      .mockResolvedValueOnce(payload({ status: 'running', is_paused: false }))
      .mockResolvedValueOnce(
        payload({ status: 'paused', is_paused: true, reason: 'scheduler gate mode = off' })
      );
    mockSetEnabled.mockResolvedValueOnce({ enabled: false, changed: true, mode: 'off' });

    render(<MemoryTreeStatusPanel />);
    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-status-label')).toHaveTextContent(/running/i);
    });

    const toggle = screen.getByTestId('memory-tree-status-toggle');
    expect(toggle.getAttribute('aria-checked')).toBe('true');

    fireEvent.click(toggle);

    await waitFor(() => {
      expect(mockSetEnabled).toHaveBeenCalledWith(false);
    });
    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-status-label')).toHaveTextContent(/paused/i);
    });
    expect(toggle.getAttribute('aria-checked')).toBe('false');
  });

  it('renders a paused pill with the reason from the wire payload', async () => {
    mockPipelineStatus.mockResolvedValueOnce(
      payload({ status: 'paused', is_paused: true, reason: 'scheduler gate mode = off' })
    );
    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-status-label')).toHaveTextContent(/paused/i);
    });
    expect(screen.getByText(/scheduler gate mode = off/i)).toBeInTheDocument();
    expect(screen.getByTestId('memory-tree-status-toggle').getAttribute('aria-checked')).toBe(
      'false'
    );
  });

  it('shows an error banner with retry button when the fetch rejects', async () => {
    mockPipelineStatus.mockRejectedValueOnce(new Error('rpc went boom'));
    mockPipelineStatus.mockResolvedValueOnce(payload({ status: 'idle', total_chunks: 0 }));
    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-status-error')).toBeInTheDocument();
    });

    const retry = screen.getByRole('button', { name: /retry/i });
    fireEvent.click(retry);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-status-label')).toHaveTextContent(/idle/i);
    });
  });

  it('reports toggle errors via the onToast callback', async () => {
    mockPipelineStatus.mockResolvedValueOnce(payload({ status: 'running', is_paused: false }));
    mockSetEnabled.mockRejectedValueOnce(new Error('disk write failed'));
    const onToast = vi.fn();

    render(<MemoryTreeStatusPanel onToast={onToast} />);
    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-status-label')).toHaveTextContent(/running/i);
    });

    fireEvent.click(screen.getByTestId('memory-tree-status-toggle'));
    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'error', message: 'disk write failed' })
      );
    });
  });
});
