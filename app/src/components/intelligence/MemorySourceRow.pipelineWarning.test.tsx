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

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

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

/** The incident's shape: everything ingested, nothing embedded. */
function storedWithoutVectors(overrides: Partial<SourceStatus> = {}): SourceStatus {
  return {
    source_id: SOURCE_ID,
    chunks_synced: 2581,
    chunks_pending: 2581,
    last_chunk_at_ms: Date.now(),
    freshness: 'recent',
    ...overrides,
  } as SourceStatus;
}

function renderRow(overrides: Partial<React.ComponentProps<typeof MemorySourceRow>> = {}) {
  const props: React.ComponentProps<typeof MemorySourceRow> = {
    source: makeSource(),
    status: null,
    pipeline: null,
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
  // `settled = !progress && !result` in MemorySourceRow.tsx. This suppression is
  // correct (mid-sync `chunks_pending` is legitimately transient) and is also the
  // most plausible way for the indicator to never appear: a source wedged in a
  // progress state would warn about nothing forever. Both directions are pinned.
  it('hides the warning while a sync is actively reporting progress', () => {
    renderRow({
      status: storedWithoutVectors(),
      progress: { processed: 10, total: 2581 } as unknown as React.ComponentProps<
        typeof MemorySourceRow
      >['progress'],
    });

    expect(warning()).not.toBeInTheDocument();
  });

  // Skipped, not deleted, and it asserts the DESIRED behaviour rather than the
  // current one — which is the only framing that satisfies both reviews on this
  // thread.
  //
  // The bug: `settled = !progress && !result` (MemorySourceRow.tsx:100), and
  // MemorySourcesRegistry clears `result` only when the NEXT sync starts
  // (`:213`, `:358`). Nothing else drops it. So after any completed or failed
  // sync the "stored without vectors" warning is suppressed indefinitely —
  // exactly the incident this file exists for.
  //
  // Asserting the suppression as correct would make the bug a required
  // contract, so a genuine fix would turn this red (CodeRabbit's objection, and
  // it is right). Deleting the case would leave it invisible again, which is
  // how it hid in the first place (Codex's objection, also right). Defining the
  // expected behaviour and skipping until it holds does neither: it is a
  // written-down spec for the fix, and it goes green the moment the fix lands.
  //
  // UNSKIP when the suppression is narrowed to `!progress`, or when the
  // registry clears `syncResults` once the row's status refreshes post-sync.
  // Tracked in ~/tinyhuman/bugs/W6-ui-bugs.md #14.
  it.skip('should still warn after a sync finishes while chunks remain unembedded', () => {
    renderRow({
      status: storedWithoutVectors(),
      progress: null,
      result: { ok: true } as unknown as React.ComponentProps<typeof MemorySourceRow>['result'],
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
