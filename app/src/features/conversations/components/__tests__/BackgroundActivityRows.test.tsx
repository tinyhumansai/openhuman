import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { CoreCronJob } from '../../../../utils/tauriCommands/cron';
import type { MemorySyncStatusRow } from '../../../../utils/tauriCommands/memoryTree';
import type { MemorySyncSummary, SubconsciousSummary } from '../../hooks/useBackgroundActivity';
import { CronJobRow, MemorySection, SubconsciousRow } from '../BackgroundActivityRows';

function cronJob(partial: Partial<CoreCronJob> & { id: string }): CoreCronJob {
  return {
    expression: '',
    schedule: { kind: 'cron', expr: '0 9 * * *' },
    command: '',
    job_type: 'agent',
    session_target: 'isolated',
    enabled: true,
    delivery: { mode: 'silent', best_effort: true },
    delete_after_run: false,
    created_at: '2024-01-01T00:00:00Z',
    next_run: '2999-01-01T00:00:00Z',
    ...partial,
  };
}

describe('CronJobRow', () => {
  it('renders an enabled job with name, schedule and a next-run hint', () => {
    render(<CronJobRow job={cronJob({ id: 'j1', name: 'Daily standup' })} />);
    expect(screen.getByText('Daily standup')).toBeInTheDocument();
    expect(screen.getByText(/0 9 \* \* \*/)).toBeInTheDocument();
    // formatResetTime renders a future "in …" hint for an enabled job.
    expect(screen.getByText(/^Next in /)).toBeInTheDocument();
    expect(screen.queryByText('Paused')).not.toBeInTheDocument();
  });

  it('falls back to the prompt, then command, then a generic title', () => {
    const { rerender } = render(
      <CronJobRow job={cronJob({ id: 'j2', prompt: 'summarize my inbox' })} />
    );
    expect(screen.getByText('summarize my inbox')).toBeInTheDocument();

    rerender(<CronJobRow job={cronJob({ id: 'j3', command: 'echo hi' })} />);
    expect(screen.getByText('echo hi')).toBeInTheDocument();

    rerender(<CronJobRow job={cronJob({ id: 'j4' })} />);
    expect(screen.getByText('Untitled job')).toBeInTheDocument();
  });

  it('marks a disabled job as Paused and shows the never-run state', () => {
    const { container } = render(
      <CronJobRow job={cronJob({ id: 'j5', name: 'Paused job', enabled: false })} />
    );
    expect(screen.getByText('Paused')).toBeInTheDocument();
    expect(screen.getByText('Hasn’t run yet')).toBeInTheDocument();
    expect(container.querySelector('[data-testid="background-cron-row"]')?.className).toContain(
      'opacity-50'
    );
  });

  it('summarizes "every" and "at" schedules', () => {
    const { rerender } = render(
      <CronJobRow
        job={cronJob({
          id: 'e1',
          name: 'Interval',
          schedule: { kind: 'every', every_ms: 900_000 },
        })}
      />
    );
    expect(screen.getByText('Every 15m')).toBeInTheDocument();

    rerender(
      <CronJobRow
        job={cronJob({
          id: 'a1',
          name: 'One-off run',
          schedule: { kind: 'at', at: '2999-01-01T00:00:00Z' },
        })}
      />
    );
    expect(screen.getByText('One-off run')).toBeInTheDocument();
    expect(screen.getByText('Once')).toBeInTheDocument();
  });
});

describe('SubconsciousRow', () => {
  function summary(partial: Partial<SubconsciousSummary>): SubconsciousSummary {
    return {
      enabled: true,
      mode: 'event_driven',
      lastTickAt: null,
      totalTicks: 0,
      working: false,
      queueDepth: null,
      ...partial,
    };
  }

  it('shows a live "Working…" pill when the orchestrator is busy', () => {
    render(<SubconsciousRow summary={summary({ working: true, queueDepth: 3, totalTicks: 7 })} />);
    expect(screen.getByText('Working…')).toBeInTheDocument();
    expect(screen.getByText(/3 queued/)).toBeInTheDocument();
    expect(screen.getByText(/7 runs/)).toBeInTheDocument();
  });

  it('shows Idle when enabled but not working', () => {
    render(<SubconsciousRow summary={summary({ working: false })} />);
    expect(screen.getByText('Idle')).toBeInTheDocument();
  });

  it('shows Off when disabled or mode is off', () => {
    const { rerender } = render(<SubconsciousRow summary={summary({ enabled: false })} />);
    expect(screen.getByText('Off')).toBeInTheDocument();
    rerender(<SubconsciousRow summary={summary({ mode: 'off' })} />);
    expect(screen.getByText('Off')).toBeInTheDocument();
  });
});

describe('MemorySection', () => {
  function provider(
    partial: Partial<MemorySyncStatusRow> & { provider: string }
  ): MemorySyncStatusRow {
    return {
      chunks_synced: 0,
      chunks_pending: 0,
      batch_total: 0,
      batch_processed: 0,
      last_chunk_at_ms: null,
      freshness: 'idle',
      ...partial,
    };
  }

  it('renders the up-to-date empty state when nothing is happening', () => {
    const memory: MemorySyncSummary = { ingesting: false, queueDepth: 0, providers: [] };
    render(<MemorySection memory={memory} />);
    expect(screen.getByText('All memories up to date')).toBeInTheDocument();
  });

  it('renders the ingesting row and per-provider freshness', () => {
    const memory: MemorySyncSummary = {
      ingesting: true,
      currentTitle: 'Team channel',
      queueDepth: 2,
      providers: [
        provider({ provider: 'slack', freshness: 'active' }),
        provider({ provider: 'gmail', freshness: 'recent' }),
        provider({ provider: 'notion', freshness: 'idle' }),
      ],
    };
    render(<MemorySection memory={memory} />);
    expect(screen.getByText('Indexing Team channel')).toBeInTheDocument();
    expect(screen.getByText('2 queued')).toBeInTheDocument();
    expect(screen.getByText('slack')).toBeInTheDocument();
    expect(screen.getByText('Syncing now')).toBeInTheDocument();
    expect(screen.getByText('Synced recently')).toBeInTheDocument();
    expect(screen.getByText('Idle')).toBeInTheDocument();
  });

  it('does NOT call a stale, un-drained backlog "Syncing now" (idle freshness)', () => {
    // Regression: a fetch wave from days ago whose chunks never finished
    // embedding (batch_total > batch_processed) is a backlog, not live activity.
    const memory: MemorySyncSummary = {
      ingesting: false,
      queueDepth: 0,
      providers: [
        provider({ provider: 'gmail', freshness: 'idle', batch_total: 18, batch_processed: 0 }),
      ],
    };
    render(<MemorySection memory={memory} />);
    expect(screen.queryByText('Syncing now')).not.toBeInTheDocument();
    expect(screen.getByText('Idle')).toBeInTheDocument();
    // The incomplete wave is surfaced as a muted, non-alarming progress hint.
    expect(screen.getByText('0/18 indexed')).toBeInTheDocument();
  });

  it('still shows "Syncing now" for genuinely live (active) freshness', () => {
    const memory: MemorySyncSummary = {
      ingesting: false,
      queueDepth: 0,
      providers: [provider({ provider: 'slack', freshness: 'active' })],
    };
    render(<MemorySection memory={memory} />);
    expect(screen.getByText('Syncing now')).toBeInTheDocument();
  });
});
