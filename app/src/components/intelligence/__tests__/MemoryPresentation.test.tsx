import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ActionableItem } from '../../../types/intelligence';
import type { Chunk } from '../../../utils/tauriCommands';
import { ActionableCard } from '../ActionableCard';
import { MemoryEmptyPlaceholder } from '../MemoryEmptyPlaceholder';
import { MemoryResultList } from '../MemoryResultList';
import { MemoryStatsBar } from '../MemoryStatsBar';

function makeChunk(overrides: Partial<Chunk> = {}): Chunk {
  return {
    id: 'chunk-1',
    source_kind: 'email',
    source_id: 'gmail:alice@example.com|bob@example.com',
    owner: 'bob@example.com',
    timestamp_ms: Date.now(),
    token_count: 120,
    lifecycle_status: 'admitted',
    content_preview: 'Review launch checklist. Include final QA notes.',
    has_embedding: true,
    tags: [],
    ...overrides,
  };
}

function localDayAt(daysAgo: number, hour: number, minute = 0): number {
  const d = new Date(Date.now());
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - daysAgo);
  d.setHours(hour, minute, 0, 0);
  return d.getTime();
}

function makeActionableItem(overrides: Partial<ActionableItem> = {}): ActionableItem {
  const createdAt = new Date(Date.now() - 2 * 60 * 1000);
  return {
    id: 'action-1',
    title: 'Follow up with Alice',
    description: 'Send the investor update before the meeting.',
    source: 'email',
    sourceLabel: 'Gmail',
    priority: 'important',
    status: 'active',
    createdAt,
    updatedAt: createdAt,
    actionable: true,
    ...overrides,
  };
}

describe('memory presentation components', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-05-16T12:00:00Z'));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders the empty memory placeholder copy', () => {
    render(<MemoryEmptyPlaceholder />);

    expect(screen.getByTestId('memory-empty-placeholder')).toBeInTheDocument();
    expect(
      screen.getByRole('heading', { name: /Nothing yet|No memories yet/i })
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Connect an integration in Settings|Start interacting/i)
    ).toBeInTheDocument();
  });

  it('formats memory stats and secondary labels', () => {
    render(
      <MemoryStatsBar
        totalDocs={1234}
        totalFiles={2}
        totalNamespaces={5}
        totalRelations={6}
        totalSessions={7}
        totalTokens={2048}
        estimatedStorageBytes={1536}
        oldestDocTimestamp={null}
        newestDocTimestamp={null}
        docsToday={3}
      />
    );

    expect(screen.getByText('Storage')).toBeInTheDocument();
    expect(screen.getByText('1.5 KB')).toBeInTheDocument();
    expect(screen.getByText('2 files')).toBeInTheDocument();
    expect(screen.getByText('Documents')).toBeInTheDocument();
    expect(screen.getByText(/^1[,.]234$/)).toBeInTheDocument();
    expect(screen.getByText('+3 today')).toBeInTheDocument();
    expect(screen.getByText('Sessions')).toBeInTheDocument();
    expect(screen.getByText(/^2[,.]048 tokens$/)).toBeInTheDocument();
  });

  it('groups memory chunks by age and calls the selection handler', () => {
    const onSelectChunk = vi.fn();
    const chunks = [
      makeChunk({
        id: 'today',
        timestamp_ms: localDayAt(0, 9, 15),
        content_preview: 'Review launch checklist. Include final QA notes.',
      }),
      makeChunk({
        id: 'yesterday',
        timestamp_ms: localDayAt(1, 14, 30),
        source_kind: 'chat',
        source_id: 'slack:product',
        content_preview: 'Discussed onboarding metrics with the team.',
      }),
      makeChunk({
        id: 'week',
        timestamp_ms: localDayAt(3, 8, 45),
        content_preview: 'Draft roadmap priorities for next sprint.',
      }),
      makeChunk({ id: 'older', timestamp_ms: localDayAt(9, 16, 0), content_preview: '' }),
    ];

    render(
      <MemoryResultList chunks={chunks} selectedChunkId="yesterday" onSelectChunk={onSelectChunk} />
    );

    const list = screen.getByTestId('memory-result-list');
    expect(within(list).getByText('TODAY')).toBeInTheDocument();
    expect(within(list).getByText('YESTERDAY')).toBeInTheDocument();
    expect(within(list).getByText('THIS WEEK')).toBeInTheDocument();
    expect(within(list).getByText('OLDER')).toBeInTheDocument();
    expect(within(list).getByText('Review launch checklist.')).toBeInTheDocument();
    expect(within(list).getByText('older')).toBeInTheDocument();

    const selectedRow = within(list).getByText('Discussed onboarding metrics with the team.');
    expect(selectedRow.closest('button')).toHaveAttribute('aria-pressed', 'true');

    fireEvent.click(within(list).getByText('Review launch checklist.'));
    expect(onSelectChunk).toHaveBeenCalledWith('today');
  });

  it('renders an empty result-list state', () => {
    render(<MemoryResultList chunks={[]} selectedChunkId={null} onSelectChunk={vi.fn()} />);

    expect(screen.getByTestId('memory-result-list')).toBeInTheDocument();
    expect(screen.getByText(/No matching chunks|No memories found/i)).toBeInTheDocument();
  });

  it('renders actionable item details and fires direct actions', () => {
    const item = makeActionableItem();
    const onComplete = vi.fn();
    const onDismiss = vi.fn();

    render(
      <ActionableCard
        item={item}
        onComplete={onComplete}
        onDismiss={onDismiss}
        onSnooze={vi.fn()}
      />
    );

    expect(screen.getByText('Follow up with Alice')).toBeInTheDocument();
    expect(screen.getByText('Send the investor update before the meeting.')).toBeInTheDocument();
    expect(screen.getByText('Gmail')).toBeInTheDocument();
    expect(screen.getByText('2 mins ago')).toBeInTheDocument();
    expect(screen.getByText('New')).toBeInTheDocument();

    fireEvent.click(screen.getByTitle('Complete'));
    fireEvent.click(screen.getByTitle('Dismiss'));

    expect(onComplete).toHaveBeenCalledWith(item);
    expect(onDismiss).toHaveBeenCalledWith(item);
  });

  it('snoozes actionable items after the exit animation delay', async () => {
    const item = makeActionableItem();
    const onSnooze = vi.fn();

    render(
      <ActionableCard item={item} onComplete={vi.fn()} onDismiss={vi.fn()} onSnooze={onSnooze} />
    );

    fireEvent.click(screen.getByTitle('Snooze'));
    fireEvent.click(screen.getByRole('button', { name: '6 hours' }));

    expect(onSnooze).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(200);
    });

    expect(onSnooze).toHaveBeenCalledWith(item, 6 * 60 * 60 * 1000);
  });
});
