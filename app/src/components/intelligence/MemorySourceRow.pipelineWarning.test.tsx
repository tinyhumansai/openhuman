/**
 * The "stored without vectors" warning must actually reach the screen.
 *
 * Motivating incident: a workspace sat with 2,581 chunks synced and 0 embedded,
 * and no degraded indicator appeared anywhere in the UI. The user's semantic
 * search was silently returning nothing findable, while every surface reported
 * a healthy sync.
 *
 * The *verdict* behind that warning — `deriveSourcePipelineHealth` — is already
 * exhaustively covered by `sourcePipelineStatus.test.ts` (14 cases, every
 * branch). What was NOT covered is whether the verdict is ever rendered:
 * `MemorySourceRow.test.tsx` only exercises the settings disclosure. Deleting
 * the whole `{ingestedOnly && …}` block from `MemorySourceRow.tsx` leaves every
 * other test in the repo green — a correct verdict computed into a void, which
 * is exactly the shape of the reported incident.
 *
 * These tests therefore assert the *rendered* contract, not the derivation:
 * the warning appears when chunks are stored without vectors, it carries the
 * message the user needs, and — the easy thing to get wrong — it is suppressed
 * only while a sync is genuinely in flight.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { MemorySourceEntry, SourceStatus } from '../../services/memorySourcesService';
import { MemorySourceRow } from './MemorySourceRow';

// Keys come back verbatim, except the one with a placeholder: it comes back as
// its English shape so the count interpolation is asserted, not just the key.
vi.mock('../../lib/i18n/I18nContext', () => ({
  useT: () => ({
    t: (k: string) =>
      k === 'sync.pipeline.vectorsPending' ? 'Chunks waiting for vectors: {count}' : k,
  }),
}));

const SOURCE_ID = 'src_brain_1';

function makeSource(overrides: Partial<MemorySourceEntry> = {}): MemorySourceEntry {
  return {
    id: SOURCE_ID,
    kind: 'github_repo',
    label: 'My Repo',
    enabled: true,
    url: 'https://github.com/org/repo',
    ...overrides,
  };
}

/**
 * The incident's shape: everything ingested, nothing embedded, and nothing
 * moving — the newest chunk is long past the core's five-minute `recent`
 * window and no backfill snapshot says a chain is draining it. (A backlog
 * that IS moving is the neutral "waiting for vectors" note, pinned at the
 * bottom of this file — openhuman#6025.)
 */
function storedWithoutVectors(overrides: Partial<SourceStatus> = {}): SourceStatus {
  return {
    source_id: SOURCE_ID,
    chunks_synced: 2581,
    chunks_pending: 2581,
    last_chunk_at_ms: Date.now() - 60 * 60_000,
    freshness: 'idle',
    ...overrides,
  } as SourceStatus;
}

function renderRow(overrides: Partial<React.ComponentProps<typeof MemorySourceRow>> = {}) {
  const props: React.ComponentProps<typeof MemorySourceRow> = {
    source: makeSource(),
    status: null,
    pipeline: null,
    backfill: null,
    isAuthenticated: true,
    isSyncing: false,
    isBuilding: false,
    progress: null,
    result: null,
    settingsExpanded: false,
    onToggle: vi.fn(),
    onRemove: vi.fn(),
    onSync: vi.fn(),
    onBuild: vi.fn(),
    onToggleSettings: vi.fn(),
    onSettingsSaved: vi.fn(),
    onViewHealth: vi.fn(),
    onSignIn: vi.fn(),
    ...overrides,
  };
  render(
    <ul>
      <MemorySourceRow {...props} />
    </ul>
  );
}

const warning = () => screen.queryByTestId(`memory-source-pipeline-warning-${SOURCE_ID}`);

describe('MemorySourceRow — the row tells the truth about embedding state', () => {
  it('shows the pipeline warning when every synced chunk is unembedded', () => {
    renderRow({ status: storedWithoutVectors() });

    const banner = warning();
    expect(banner).toBeInTheDocument();
    expect(banner).toHaveTextContent('sync.pipeline.storedWithoutVectors');
  });

  it('shows the warning for a single unembedded chunk, not just a large backlog', () => {
    // There is no threshold in the contract: one chunk that semantic search
    // cannot reach is still a source that is not retrieval-ready.
    renderRow({ status: storedWithoutVectors({ chunks_synced: 10, chunks_pending: 1 }) });

    expect(warning()).toBeInTheDocument();
  });

  it('stays silent when every chunk is embedded', () => {
    renderRow({ status: storedWithoutVectors({ chunks_pending: 0 }) });

    expect(warning()).not.toBeInTheDocument();
  });

  it('stays silent before anything has been ingested', () => {
    // A brand-new source has no chunks and therefore nothing to warn about;
    // warning here would train users to ignore the banner.
    renderRow({ status: storedWithoutVectors({ chunks_synced: 0, chunks_pending: 0 }) });

    expect(warning()).not.toBeInTheDocument();
  });

  it('surfaces the warning from the global semantic-recall latch with no pending chunks', () => {
    // The per-source count can be 0 while the process-wide embeddings provider
    // is down; the row still is not retrieval-ready.
    renderRow({
      status: storedWithoutVectors({ chunks_pending: 0 }),
      pipeline: {
        status: 'degraded',
        degraded: { semantic_recall: true },
      } as unknown as React.ComponentProps<typeof MemorySourceRow>['pipeline'],
    });

    expect(warning()).toBeInTheDocument();
    expect(warning()).toHaveTextContent('sync.pipeline.storedWithoutVectors');
  });
});

describe('MemorySourceRow — what suppresses the warning', () => {
  // `settled = !progress` in MemorySourceRow.tsx (openhuman#6025 dropped the
  // `!result` half). Suppressing while progress streams is correct (mid-sync
  // `chunks_pending` is legitimately transient) and is also the most plausible
  // way for the indicator to never appear: a source wedged in a progress state
  // would warn about nothing forever. Both directions are pinned.
  it('hides the warning while a sync is actively reporting progress', () => {
    renderRow({
      status: storedWithoutVectors(),
      progress: { processed: 10, total: 2581 } as unknown as React.ComponentProps<
        typeof MemorySourceRow
      >['progress'],
    });

    expect(warning()).not.toBeInTheDocument();
  });

  // A terminal result chip no longer suppresses the verdict (openhuman#6025):
  // the chip sits on the row until the NEXT sync starts, so `!result` hid a
  // real hole in recall indefinitely — exactly the incident this file exists
  // for. The verdict now tells a draining backlog from a stuck one itself,
  // so the chip is not needed as a proxy. This case was written, and skipped,
  // as the spec for that fix; it went green the moment the fix landed.
  it('still warns after a sync finishes while chunks remain unembedded', () => {
    renderRow({
      status: storedWithoutVectors(),
      progress: null,
      result: { kind: 'success', items: 2581, note: null } as unknown as React.ComponentProps<
        typeof MemorySourceRow
      >['result'],
    });

    expect(warning()).toBeInTheDocument();
  });

  it('reports the truth again once the sync has settled', () => {
    renderRow({ status: storedWithoutVectors(), progress: null, result: null });

    expect(warning()).toBeInTheDocument();
  });

  it('offers sign-in only when the failure is a missing backend session', () => {
    renderRow({
      isAuthenticated: false,
      status: storedWithoutVectors(),
      pipeline: {
        status: 'degraded',
        first_blocking_cause: { code: 'auth_missing' },
      } as unknown as React.ComponentProps<typeof MemorySourceRow>['pipeline'],
    });

    expect(screen.getByTestId(`memory-source-signin-${SOURCE_ID}`)).toBeInTheDocument();
  });

  it('does not offer sign-in for a non-auth embeddings failure', () => {
    renderRow({
      isAuthenticated: false,
      status: storedWithoutVectors(),
      pipeline: {
        status: 'degraded',
        first_blocking_cause: { code: 'provider_unreachable' },
      } as unknown as React.ComponentProps<typeof MemorySourceRow>['pipeline'],
    });

    expect(warning()).toBeInTheDocument();
    expect(screen.queryByTestId(`memory-source-signin-${SOURCE_ID}`)).not.toBeInTheDocument();
  });
});

describe('MemorySourceRow — a backlog that is still draining is pending, not failed (openhuman#6025)', () => {
  const pendingNote = () => screen.queryByTestId(`memory-source-vectors-pending-${SOURCE_ID}`);

  it('shows the neutral waiting note, not the warning, while a re-embed chain has rows to process', () => {
    // The report: 676 chunks, 322 pending, ten minutes after a Gmail sync, and
    // the row went amber for a backlog the engine was already draining.
    renderRow({
      status: storedWithoutVectors({ chunks_synced: 676, chunks_pending: 322 }),
      backfill: { in_progress: true, pending_jobs: 1 },
    });

    expect(warning()).not.toBeInTheDocument();
    const note = pendingNote();
    expect(note).toBeInTheDocument();
    expect(note).toHaveTextContent('Chunks waiting for vectors: 322');
    // The freshness pill stays; the amber "Ingested only" pill does not appear.
    expect(screen.getByText('sync.idle')).toBeInTheDocument();
    expect(
      screen.queryByTestId(`memory-source-ingested-only-${SOURCE_ID}`)
    ).not.toBeInTheDocument();
  });

  it('shows the waiting note while the newest chunk is recent, even with no backfill snapshot', () => {
    renderRow({
      status: storedWithoutVectors({
        chunks_pending: 4,
        freshness: 'recent',
        last_chunk_at_ms: Date.now(),
      }),
      backfill: null,
    });

    expect(warning()).not.toBeInTheDocument();
    expect(pendingNote()).toBeInTheDocument();
  });

  it('keeps the waiting note beside a fresh result chip', () => {
    // The minutes right after `completed` are exactly when the backlog is
    // visible; the chip must hide neither the note nor (above) the warning.
    renderRow({
      status: storedWithoutVectors({ chunks_pending: 322, freshness: 'active' }),
      backfill: { in_progress: true, pending_jobs: 1 },
      result: { kind: 'success', items: 500, note: null } as unknown as React.ComponentProps<
        typeof MemorySourceRow
      >['result'],
    });

    expect(screen.getByTestId(`memory-source-result-${SOURCE_ID}`)).toBeInTheDocument();
    expect(pendingNote()).toBeInTheDocument();
    expect(warning()).not.toBeInTheDocument();
  });

  it('turns amber once the backlog is idle with no chain queued', () => {
    renderRow({
      status: storedWithoutVectors({ chunks_pending: 322 }),
      backfill: { in_progress: false, pending_jobs: 0 },
    });

    expect(pendingNote()).not.toBeInTheDocument();
    expect(warning()).toBeInTheDocument();
  });

  it('hides the note while a sync is actively reporting progress', () => {
    renderRow({
      status: storedWithoutVectors({ chunks_pending: 322, freshness: 'active' }),
      backfill: { in_progress: true, pending_jobs: 1 },
      progress: { processed: 10, total: 2581 } as unknown as React.ComponentProps<
        typeof MemorySourceRow
      >['progress'],
    });

    expect(pendingNote()).not.toBeInTheDocument();
  });
});
