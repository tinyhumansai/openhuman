/**
 * Tests for MemorySourcesRegistry (issue #3295):
 *
 * RC#1 — concurrent syncs: multiple sources can show "syncing" simultaneously.
 * RC#2 — source_id matching: events matched by source_id (preferred) or
 *         connection_id (fallback) for backward compat.
 * RC#4 — tolerant parseSyncProgress: numeric ratio, stage fallback, and
 *         indeterminate (null) cases.
 */
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import {
  MemorySourcesRegistry,
  parseIngestedCount,
  parseSyncProgress,
} from '../MemorySourcesRegistry';

// ── i18n mock (returns key as the translation) ────────────────────────────────
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

// ── memorySourcesService mock ─────────────────────────────────────────────────
vi.mock('../../../services/memorySourcesService', () => ({
  listMemorySources: vi.fn().mockResolvedValue([]),
  memorySourcesStatusList: vi.fn().mockResolvedValue([]),
  syncMemorySource: vi.fn().mockResolvedValue(undefined),
  removeMemorySource: vi.fn().mockResolvedValue(undefined),
  updateMemorySource: vi
    .fn()
    .mockImplementation((id: string, patch: Record<string, unknown>) =>
      Promise.resolve({ id, kind: 'folder', label: 'Test', enabled: true, ...patch })
    ),
  SOURCE_KIND_ICONS: {
    folder: 'F',
    composio: 'C',
    github_repo: 'G',
    rss_feed: 'R',
    web_page: 'W',
    twitter_query: 'T',
  },
  SOURCE_KIND_LABEL_KEYS: {
    folder: 'memorySources.kind.folder',
    composio: 'memorySources.kind.composio',
    github_repo: 'memorySources.kind.github_repo',
    rss_feed: 'memorySources.kind.rss_feed',
    web_page: 'memorySources.kind.web_page',
    twitter_query: 'memorySources.kind.twitter_query',
  },
}));

// ── tauriCommands mock ────────────────────────────────────────────────────────
vi.mock('../../../utils/tauriCommands/memoryTree', () => ({
  memoryTreeFlushSource: vi.fn().mockResolvedValue({ seals_fired: 0 }),
}));

// ── helpers ───────────────────────────────────────────────────────────────────

function makeSyncStageEvent(detail: {
  stage: string;
  source_id?: string | null;
  connection_id?: string | null;
  detail?: string;
}): CustomEvent {
  return new CustomEvent('openhuman:memory-sync-stage', { detail });
}

function makeSource(id: string) {
  return { id, kind: 'folder' as const, label: `Source ${id}`, enabled: true };
}

// ── parseSyncProgress unit tests ──────────────────────────────────────────────

describe('parseSyncProgress', () => {
  it('returns ratio for "N/M ..." numeric pattern', () => {
    expect(parseSyncProgress('5/10 processed', 'ingesting')).toBe(50);
    expect(parseSyncProgress('3/12 docs', 'fetching')).toBe(25);
    expect(parseSyncProgress('1/1 done', 'ingesting')).toBe(100);
  });

  it('returns stage fallback when no numeric ratio is present', () => {
    expect(parseSyncProgress('queue_depth=3', 'ingesting')).toBe(40);
    expect(parseSyncProgress('listing items', 'fetching')).toBe(5);
    expect(parseSyncProgress(null, 'requested')).toBe(2);
    expect(parseSyncProgress('canonicalized 3 chunks', 'stored')).toBe(15);
    expect(parseSyncProgress('queued chunk extraction', 'queued')).toBe(25);
  });

  it('returns 100 for completed stage', () => {
    expect(parseSyncProgress(null, 'completed')).toBe(100);
    expect(parseSyncProgress('ingested 5 item(s)', 'completed')).toBe(100);
  });

  it('returns null when no ratio and no recognized stage', () => {
    expect(parseSyncProgress('some detail', 'unknown_stage')).toBeNull();
    expect(parseSyncProgress(null, undefined)).toBeNull();
    expect(parseSyncProgress(null)).toBeNull();
  });

  it('handles non-ratio numeric strings gracefully', () => {
    // "N discovered" — no slash — should use stage fallback, not parse as ratio
    expect(parseSyncProgress('3 discovered', 'stored')).toBe(15);
    // "N/0 ..." — divide by zero guard
    expect(parseSyncProgress('5/0 items', 'fetching')).toBe(5); // falls through to stage fallback
  });
});

// ── parseIngestedCount unit tests ─────────────────────────────────────────────

describe('parseIngestedCount', () => {
  it('parses the count from "ingested N item(s)"', () => {
    expect(parseIngestedCount('ingested 5 item(s)')).toBe(5);
    expect(parseIngestedCount('ingested 0 item(s)')).toBe(0);
    expect(parseIngestedCount('ingested 123 items')).toBe(123);
  });

  it('returns null when no count is present', () => {
    expect(parseIngestedCount(null)).toBeNull();
    expect(parseIngestedCount('done')).toBeNull();
    expect(parseIngestedCount('delegating to composio sync')).toBeNull();
  });
});

// ── MemorySourcesRegistry integration tests ───────────────────────────────────

describe('MemorySourcesRegistry', () => {
  // Expose mock so tests can control what listMemorySources returns.
  let listMemorySources: ReturnType<typeof vi.fn>;
  let memorySourcesStatusList: ReturnType<typeof vi.fn>;
  let syncMemorySource: ReturnType<typeof vi.fn>;

  beforeEach(async () => {
    const svc = await import('../../../services/memorySourcesService');
    listMemorySources = svc.listMemorySources as ReturnType<typeof vi.fn>;
    memorySourcesStatusList = svc.memorySourcesStatusList as ReturnType<typeof vi.fn>;
    syncMemorySource = svc.syncMemorySource as ReturnType<typeof vi.fn>;
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('shows two sources as syncing when two concurrent sync-stage events arrive (RC#1)', async () => {
    const sources = [makeSource('src-alpha'), makeSource('src-beta')];
    listMemorySources.mockResolvedValue(sources);
    memorySourcesStatusList.mockResolvedValue([]);

    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);

    // Wait for sources to render.
    await waitFor(() => {
      expect(screen.getByText('Source src-alpha')).toBeInTheDocument();
      expect(screen.getByText('Source src-beta')).toBeInTheDocument();
    });

    // Dispatch two concurrent "requested" events with different source_ids.
    act(() => {
      window.dispatchEvent(
        makeSyncStageEvent({
          stage: 'requested',
          source_id: 'src-alpha',
          connection_id: 'src-alpha',
        })
      );
      window.dispatchEvent(
        makeSyncStageEvent({ stage: 'requested', source_id: 'src-beta', connection_id: 'src-beta' })
      );
    });

    // Both rows should show the syncing spinner/text (sync.syncing key → "sync.syncing").
    await waitFor(() => {
      const syncingButtons = screen.getAllByText('sync.syncing');
      expect(syncingButtons).toHaveLength(2);
    });
  });

  it('clears only one source when completed, leaving the other syncing (RC#1)', async () => {
    const sources = [makeSource('src-alpha'), makeSource('src-beta')];
    listMemorySources.mockResolvedValue(sources);
    memorySourcesStatusList.mockResolvedValue([]);

    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);

    await waitFor(() => {
      expect(screen.getByText('Source src-alpha')).toBeInTheDocument();
    });

    // Both start syncing.
    act(() => {
      window.dispatchEvent(
        makeSyncStageEvent({
          stage: 'requested',
          source_id: 'src-alpha',
          connection_id: 'src-alpha',
        })
      );
      window.dispatchEvent(
        makeSyncStageEvent({ stage: 'requested', source_id: 'src-beta', connection_id: 'src-beta' })
      );
    });

    await waitFor(() => {
      expect(screen.getAllByText('sync.syncing')).toHaveLength(2);
    });

    // Complete src-alpha — only src-beta should remain syncing.
    act(() => {
      window.dispatchEvent(
        makeSyncStageEvent({
          stage: 'completed',
          source_id: 'src-alpha',
          connection_id: 'src-alpha',
        })
      );
    });

    await waitFor(() => {
      const syncingButtons = screen.getAllByText('sync.syncing');
      expect(syncingButtons).toHaveLength(1);
    });

    // The one remaining syncing button should be for src-beta.
    // The sync button for src-alpha should now show "sync.sync".
    const syncButtons = screen.getAllByText('sync.sync');
    expect(syncButtons).toHaveLength(1); // src-alpha back to idle
  });

  it('failed event also clears the syncing source (RC#1)', async () => {
    const sources = [makeSource('src-gamma')];
    listMemorySources.mockResolvedValue(sources);
    memorySourcesStatusList.mockResolvedValue([]);

    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);

    await waitFor(() => {
      expect(screen.getByText('Source src-gamma')).toBeInTheDocument();
    });

    act(() => {
      window.dispatchEvent(makeSyncStageEvent({ stage: 'fetching', source_id: 'src-gamma' }));
    });

    await waitFor(() => {
      expect(screen.getByText('sync.syncing')).toBeInTheDocument();
    });

    act(() => {
      window.dispatchEvent(makeSyncStageEvent({ stage: 'failed', source_id: 'src-gamma' }));
    });

    await waitFor(() => {
      expect(screen.queryByText('sync.syncing')).not.toBeInTheDocument();
      expect(screen.getByText('sync.sync')).toBeInTheDocument();
    });
  });

  it('matches events by source_id when present, ignoring connection_id (RC#2)', async () => {
    const sources = [makeSource('src-new')];
    listMemorySources.mockResolvedValue(sources);
    memorySourcesStatusList.mockResolvedValue([]);

    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);

    await waitFor(() => {
      expect(screen.getByText('Source src-new')).toBeInTheDocument();
    });

    // Event with source_id matching the row but a different connection_id
    // (e.g. an intermediate stage from the bridge where connection_id = document_id).
    act(() => {
      window.dispatchEvent(
        makeSyncStageEvent({
          stage: 'ingesting',
          source_id: 'src-new', // matches the row
          connection_id: 'mem_src:src-new:doc-123', // ingest-pipeline id, NOT the row id
        })
      );
    });

    // Row should light up because source_id matches.
    await waitFor(() => {
      expect(screen.getByText('sync.syncing')).toBeInTheDocument();
    });
  });

  it('falls back to connection_id when source_id is absent (RC#2 backward compat)', async () => {
    const sources = [makeSource('src-legacy')];
    listMemorySources.mockResolvedValue(sources);
    memorySourcesStatusList.mockResolvedValue([]);

    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);

    await waitFor(() => {
      expect(screen.getByText('Source src-legacy')).toBeInTheDocument();
    });

    // Old-style event: no source_id, connection_id is the row id.
    act(() => {
      window.dispatchEvent(
        makeSyncStageEvent({
          stage: 'fetching',
          // source_id absent
          connection_id: 'src-legacy',
        })
      );
    });

    await waitFor(() => {
      expect(screen.getByText('sync.syncing')).toBeInTheDocument();
    });
  });

  it('ignores events with neither source_id nor connection_id', async () => {
    const sources = [makeSource('src-quiet')];
    listMemorySources.mockResolvedValue(sources);
    memorySourcesStatusList.mockResolvedValue([]);

    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);

    await waitFor(() => {
      expect(screen.getByText('Source src-quiet')).toBeInTheDocument();
    });

    act(() => {
      window.dispatchEvent(
        new CustomEvent('openhuman:memory-sync-stage', {
          detail: { stage: 'fetching' }, // no source_id, no connection_id
        })
      );
    });

    // Row should remain idle.
    await waitFor(() => {
      expect(screen.queryByText('sync.syncing')).not.toBeInTheDocument();
    });
  });

  // ── Terminal result chips (#3295) ──────────────────────────────────────────

  it('shows "N items synced" after a completed event with a non-zero count', async () => {
    const sources = [makeSource('src-done')];
    listMemorySources.mockResolvedValue(sources);
    memorySourcesStatusList.mockResolvedValue([]);

    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);
    await waitFor(() => expect(screen.getByText('Source src-done')).toBeInTheDocument());

    act(() => {
      window.dispatchEvent(makeSyncStageEvent({ stage: 'fetching', source_id: 'src-done' }));
    });
    await waitFor(() => expect(screen.getByText('sync.syncing')).toBeInTheDocument());

    act(() => {
      window.dispatchEvent(
        makeSyncStageEvent({
          stage: 'completed',
          source_id: 'src-done',
          detail: 'ingested 5 item(s)',
        })
      );
    });

    // The result chip persists and shows the parsed count + the (mocked) i18n key.
    await waitFor(() => {
      const chip = screen.getByTestId('memory-source-result-src-done');
      expect(chip).toHaveTextContent('5');
      expect(chip).toHaveTextContent('memorySources.sync.itemsSynced');
    });
    // The progress bar / syncing state is gone.
    expect(screen.queryByText('sync.syncing')).not.toBeInTheDocument();
  });

  it('shows "Up to date" after a completed event with zero new items', async () => {
    const sources = [makeSource('src-noop')];
    listMemorySources.mockResolvedValue(sources);
    memorySourcesStatusList.mockResolvedValue([]);

    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);
    await waitFor(() => expect(screen.getByText('Source src-noop')).toBeInTheDocument());

    act(() => {
      window.dispatchEvent(
        makeSyncStageEvent({
          stage: 'completed',
          source_id: 'src-noop',
          detail: 'ingested 0 item(s)',
        })
      );
    });

    await waitFor(() => {
      const chip = screen.getByTestId('memory-source-result-src-noop');
      expect(chip).toHaveTextContent('memorySources.sync.upToDate');
    });
  });

  it('shows the failure reason after a failed event', async () => {
    const sources = [makeSource('src-bad')];
    listMemorySources.mockResolvedValue(sources);
    memorySourcesStatusList.mockResolvedValue([]);

    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);
    await waitFor(() => expect(screen.getByText('Source src-bad')).toBeInTheDocument());

    act(() => {
      window.dispatchEvent(
        makeSyncStageEvent({
          stage: 'failed',
          source_id: 'src-bad',
          detail: 'composio sync failed: rate limit exceeded',
        })
      );
    });

    await waitFor(() => {
      const chip = screen.getByTestId('memory-source-result-src-bad');
      expect(chip).toHaveTextContent('memorySources.sync.failedLabel');
      expect(chip).toHaveTextContent('rate limit exceeded');
    });
  });

  it('clears a prior result chip when a new sync starts for that row', async () => {
    const sources = [makeSource('src-redo')];
    listMemorySources.mockResolvedValue(sources);
    memorySourcesStatusList.mockResolvedValue([]);

    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);
    await waitFor(() => expect(screen.getByText('Source src-redo')).toBeInTheDocument());

    // First sync completes → chip shown.
    act(() => {
      window.dispatchEvent(
        makeSyncStageEvent({
          stage: 'completed',
          source_id: 'src-redo',
          detail: 'ingested 2 item(s)',
        })
      );
    });
    await waitFor(() =>
      expect(screen.getByTestId('memory-source-result-src-redo')).toBeInTheDocument()
    );

    // A new sync starts (non-terminal event) → chip cleared, progress shown.
    act(() => {
      window.dispatchEvent(makeSyncStageEvent({ stage: 'fetching', source_id: 'src-redo' }));
    });
    await waitFor(() => {
      expect(screen.queryByTestId('memory-source-result-src-redo')).not.toBeInTheDocument();
      expect(screen.getByText('sync.syncing')).toBeInTheDocument();
    });
  });

  it('shows a failed chip when the sync RPC itself rejects (no stage event arrives)', async () => {
    const sources = [makeSource('src-rpcfail')];
    listMemorySources.mockResolvedValue(sources);
    memorySourcesStatusList.mockResolvedValue([]);
    syncMemorySource.mockRejectedValueOnce(new Error('core unreachable'));

    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);
    await waitFor(() => expect(screen.getByText('Source src-rpcfail')).toBeInTheDocument());

    // Click the row's sync button (folder kind → testid suffix "folder").
    fireEvent.click(screen.getByTestId('memory-source-sync-folder'));

    await waitFor(() => {
      const chip = screen.getByTestId('memory-source-result-src-rpcfail');
      expect(chip).toHaveTextContent('memorySources.sync.failedLabel');
      expect(chip).toHaveTextContent('core unreachable');
    });
    // The optimistic syncing state is cleared after the RPC rejection.
    expect(screen.queryByText('sync.syncing')).not.toBeInTheDocument();
  });
});
