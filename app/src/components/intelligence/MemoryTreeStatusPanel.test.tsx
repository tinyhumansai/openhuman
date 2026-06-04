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
import {
  classifyIntegration,
  MemoryTreeStatusPanel,
  providerIconChar,
} from './MemoryTreeStatusPanel';

const mockPipelineStatus = vi.fn();
const mockSetEnabled = vi.fn();
const mockSyncStatusList = vi.fn();

vi.mock('../../utils/tauriCommands', async importOriginal => {
  // Inherit everything else (types, sibling wrappers) verbatim so the panel
  // sees the same module shape as production — only the two new helpers
  // under test get swapped for spies.
  const actual = await importOriginal<typeof import('../../utils/tauriCommands')>();
  return {
    ...actual,
    memoryTreePipelineStatus: (...args: unknown[]) => mockPipelineStatus(...args),
    memoryTreeSetEnabled: (...args: unknown[]) => mockSetEnabled(...args),
    memorySyncStatusList: (...args: unknown[]) => mockSyncStatusList(...args),
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
    mockSyncStatusList.mockReset();
    mockSyncStatusList.mockResolvedValue([]); // default: empty, harmless to existing tests
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

  it('fetches integration list and pipeline status in parallel on the same tick', async () => {
    mockPipelineStatus.mockResolvedValue(payload());
    mockSyncStatusList.mockResolvedValue([
      {
        provider: 'slack',
        chunks_synced: 5231,
        chunks_pending: 0,
        batch_total: 0,
        batch_processed: 0,
        last_chunk_at_ms: FIXED_NOW_MS - 3 * 60 * 1000,
        freshness: 'active',
      },
    ]);

    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(mockPipelineStatus).toHaveBeenCalledTimes(1);
      expect(mockSyncStatusList).toHaveBeenCalledTimes(1);
    });
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

  it('renders a row per integration with provider name, chunk count, freshness pill', async () => {
    mockPipelineStatus.mockResolvedValue(payload());
    mockSyncStatusList.mockResolvedValue([
      {
        provider: 'slack',
        chunks_synced: 5231,
        chunks_pending: 0,
        batch_total: 0,
        batch_processed: 0,
        last_chunk_at_ms: FIXED_NOW_MS - 3 * 60 * 1000,
        freshness: 'active',
      },
      {
        provider: 'gmail',
        chunks_synced: 842,
        chunks_pending: 0,
        batch_total: 0,
        batch_processed: 0,
        last_chunk_at_ms: FIXED_NOW_MS - 2 * 60 * 60 * 1000,
        freshness: 'idle',
      },
    ]);

    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-integrations')).toBeInTheDocument();
    });

    const rows = screen.getAllByTestId(/^memory-tree-integration-row-/);
    expect(rows).toHaveLength(2);

    const slackRow = screen.getByTestId('memory-tree-integration-row-slack');
    expect(slackRow).toHaveTextContent(/slack/i);
    expect(slackRow).toHaveTextContent(/Chunks: 5,231/);
    expect(slackRow).toHaveTextContent(/Active/);

    const gmailRow = screen.getByTestId('memory-tree-integration-row-gmail');
    expect(gmailRow).toHaveTextContent(/gmail/i);
    expect(gmailRow).toHaveTextContent(/Stale/);
  });

  it('falls back to the never label when last_chunk_at_ms is null', async () => {
    mockPipelineStatus.mockResolvedValue(payload());
    mockSyncStatusList.mockResolvedValue([
      {
        provider: 'slack',
        chunks_synced: 0,
        chunks_pending: 0,
        batch_total: 0,
        batch_processed: 0,
        last_chunk_at_ms: null,
        freshness: 'idle',
      },
    ]);

    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-integration-row-slack')).toBeInTheDocument();
    });
    expect(screen.getByTestId('memory-tree-integration-row-slack')).toHaveTextContent(/Never/);
  });

  it('renders an empty list and logs a warn when memorySyncStatusList rejects', async () => {
    mockPipelineStatus.mockResolvedValue(payload());
    mockSyncStatusList.mockRejectedValue(new Error('boom'));
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);

    render(<MemoryTreeStatusPanel />);

    // Tiles still render (pipeline succeeded) and the strip shows the empty state.
    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-status-label')).toBeInTheDocument();
      expect(screen.getByTestId('memory-tree-integrations-empty')).toBeInTheDocument();
    });
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it('shows the empty state when there are no integrations', async () => {
    mockPipelineStatus.mockResolvedValue(payload());
    mockSyncStatusList.mockResolvedValue([]);

    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-integrations-empty')).toBeInTheDocument();
    });
    expect(screen.getByTestId('memory-tree-integrations-empty')).toHaveTextContent(
      /no integrations connected/i
    );
  });

  it('renders the integration strip between the tile grid and the toggle row', async () => {
    mockPipelineStatus.mockResolvedValue(payload());
    mockSyncStatusList.mockResolvedValue([
      {
        provider: 'slack',
        chunks_synced: 1,
        chunks_pending: 0,
        batch_total: 0,
        batch_processed: 0,
        last_chunk_at_ms: FIXED_NOW_MS - 1000,
        freshness: 'active',
      },
    ]);

    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-integrations')).toBeInTheDocument();
    });

    const panel = screen.getByTestId('memory-tree-status-panel');
    const tiles = screen.getByTestId('memory-tree-status-tiles');
    const strip = screen.getByTestId('memory-tree-integrations');
    const toggle = screen.getByTestId('memory-tree-status-toggle-row');

    const order = Array.from(panel.querySelectorAll('[data-testid]'))
      .map(el => el.getAttribute('data-testid'))
      .filter(id =>
        [
          'memory-tree-status-tiles',
          'memory-tree-integrations',
          'memory-tree-status-toggle-row',
        ].includes(id ?? '')
      );

    expect(order).toEqual([
      'memory-tree-status-tiles',
      'memory-tree-integrations',
      'memory-tree-status-toggle-row',
    ]);
    expect(tiles).toBeInTheDocument();
    expect(strip).toBeInTheDocument();
    expect(toggle).toBeInTheDocument();
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

  // ── #002 (T018): degraded status + first-blocking-cause banner ──────────

  it('renders the first-blocking-cause remediation banner with a degraded recall badge', async () => {
    mockPipelineStatus.mockResolvedValueOnce(
      payload({
        status: 'degraded',
        reason: 'semantic recall disabled',
        first_blocking_cause: {
          code: 'embeddings_unconfigured',
          class: 'unrecoverable',
          remediation_key: 'memory.health.remediation.embeddings_unconfigured',
        },
        degraded: { semantic_recall: true, structure: false },
      })
    );
    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-status-label')).toHaveTextContent(/degraded/i);
    });

    // The remediation text comes from the i18n key the core supplied.
    const remediation = screen.getByTestId('memory-tree-blocking-cause-remediation');
    expect(remediation).toHaveTextContent(/embeddings provider is configured/i);
    // Recall badge present, structure badge absent.
    expect(screen.getByTestId('memory-tree-badge-recall')).toBeInTheDocument();
    expect(screen.queryByTestId('memory-tree-badge-structure')).not.toBeInTheDocument();
  });

  it('shows the structure badge when only extraction is degraded', async () => {
    mockPipelineStatus.mockResolvedValueOnce(
      payload({
        status: 'degraded',
        first_blocking_cause: {
          code: 'extraction_timeout',
          class: 'unrecoverable',
          remediation_key: 'memory.health.remediation.extraction_timeout',
        },
        degraded: { semantic_recall: false, structure: true },
      })
    );
    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-blocking-cause')).toBeInTheDocument();
    });
    expect(screen.getByTestId('memory-tree-badge-structure')).toBeInTheDocument();
    expect(screen.queryByTestId('memory-tree-badge-recall')).not.toBeInTheDocument();
    expect(screen.getByTestId('memory-tree-blocking-cause-remediation')).toHaveTextContent(
      /extraction model is timing out/i
    );
  });

  it('does not render the blocking-cause banner on a healthy pipeline', async () => {
    mockPipelineStatus.mockResolvedValueOnce(payload({ status: 'running' }));
    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-status-label')).toHaveTextContent(/running/i);
    });
    expect(screen.queryByTestId('memory-tree-blocking-cause')).not.toBeInTheDocument();
  });
});

describe('integration health helpers', () => {
  describe('classifyIntegration', () => {
    it('maps active freshness to active', () => {
      expect(classifyIntegration('active')).toBe('active');
    });
    it('maps recent freshness to stale', () => {
      expect(classifyIntegration('recent')).toBe('stale');
    });
    it('maps idle freshness to stale', () => {
      expect(classifyIntegration('idle')).toBe('stale');
    });
  });

  describe('providerIconChar', () => {
    it('returns a known glyph for slack', () => {
      expect(providerIconChar('slack')).toBe('💬');
    });
    it('returns a known glyph for gmail', () => {
      expect(providerIconChar('gmail')).toBe('📧');
    });
    it('falls back to the plug glyph for unknown providers', () => {
      expect(providerIconChar('definitely-not-a-real-provider')).toBe('🔌');
    });
  });
});
