import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Chunk, EntityRef, GraphRelation, Source } from '../../../utils/tauriCommands';
import { MemoryHeatmap } from '../MemoryHeatmap';
import { MemoryInsights } from '../MemoryInsights';
import { MemoryNavigator, type NavigatorSelection } from '../MemoryNavigator';

function localDayAt(daysAgo: number, hour: number, minute = 0): number {
  const d = new Date(Date.now());
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - daysAgo);
  d.setHours(hour, minute, 0, 0);
  return d.getTime();
}

function makeChunk(overrides: Partial<Chunk> = {}): Chunk {
  return {
    id: 'chunk-1',
    source_kind: 'email',
    source_id: 'gmail:alice@example.com',
    owner: 'bob@example.com',
    timestamp_ms: localDayAt(0, 10),
    token_count: 100,
    lifecycle_status: 'admitted',
    content_preview: 'Memory item',
    has_embedding: true,
    tags: [],
    ...overrides,
  };
}

function makeSource(overrides: Partial<Source> = {}): Source {
  return {
    source_id: 'gmail:alice@example.com',
    display_name: 'Alice Inbox',
    source_kind: 'email',
    chunk_count: 3,
    most_recent_ms: localDayAt(0, 10),
    lifecycle_status: 'admitted',
    ...overrides,
  };
}

function makeEntity(overrides: Partial<EntityRef> = {}): EntityRef {
  return { entity_id: 'person:Alice', kind: 'person', surface: 'Alice', count: 3, ...overrides };
}

function makeRelation(overrides: Partial<GraphRelation> = {}): GraphRelation {
  return {
    namespace: 'gmail',
    subject: 'Alice',
    predicate: 'prefers',
    object: 'morning updates',
    attrs: { entity_types: { subject: 'person', object: 'preference' } },
    updatedAt: localDayAt(0, 10),
    evidenceCount: 2,
    orderIndex: null,
    documentIds: ['doc-1'],
    chunkIds: ['chunk-1'],
    ...overrides,
  };
}

function paragraphMatching(pattern: RegExp) {
  return (_content: string, node: Element | null) =>
    node?.tagName.toLowerCase() === 'p' &&
    pattern.test((node.textContent ?? '').replace(/\s+/g, ' '));
}

describe('memory overview components', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-05-16T12:00:00Z'));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders the heatmap summary and hover tooltip', () => {
    const { container } = render(
      <MemoryHeatmap
        timestamps={[
          Math.floor(localDayAt(0, 0) / 1000),
          Math.floor(localDayAt(0, 0) / 1000),
          Math.floor(localDayAt(1, 0) / 1000),
        ]}
      />
    );

    expect(screen.getByRole('heading', { name: 'Ingestion Activity' })).toBeInTheDocument();
    expect(screen.getByText(/3 events over the last 8 months/)).toBeInTheDocument();
    expect(screen.getByText(paragraphMatching(/peak\s*:\s*2\/day/i))).toBeInTheDocument();

    const cells = container.querySelectorAll('svg rect[width="11"]');
    expect(cells.length).toBeGreaterThan(0);
    const targetCell = cells[cells.length - 1];
    expect(targetCell).toBeDefined();
    fireEvent.mouseEnter(targetCell);
    expect(screen.getByText(/events?$/)).toBeInTheDocument();
  });

  it('renders the heatmap loading skeleton', () => {
    const { container } = render(<MemoryHeatmap timestamps={[]} loading />);

    expect(screen.getByRole('heading', { name: 'Ingestion Activity' })).toBeInTheDocument();
    expect(container.querySelector('.animate-pulse')).toBeInTheDocument();
  });

  it('groups insights by predicate category and expands long groups', () => {
    const relations = [
      makeRelation({ object: 'morning updates', evidenceCount: 4 }),
      makeRelation({ subject: 'Bob', object: 'brief summaries' }),
      makeRelation({ subject: 'Carol', object: 'async notes' }),
      makeRelation({ subject: 'Dana', object: 'voice memos' }),
      makeRelation({ subject: 'Alice', predicate: 'knows', object: 'Bob', evidenceCount: 1 }),
      makeRelation({
        subject: 'Alice',
        predicate: 'skilled',
        object: 'Rust',
        attrs: { entity_types: { object: 'technology' } },
      }),
    ];

    render(<MemoryInsights relations={relations} />);

    expect(screen.getByRole('heading', { name: /Insights/i })).toBeInTheDocument();
    expect(screen.getByText(paragraphMatching(/6.*relations/i))).toBeInTheDocument();
    expect(screen.getByText('Preferences')).toBeInTheDocument();
    expect(screen.getByText('Relationships')).toBeInTheDocument();
    expect(screen.getByText(/Skills/)).toBeInTheDocument();
    expect(screen.getByText('x4')).toBeInTheDocument();
    expect(screen.getByText('+1 more')).toBeInTheDocument();

    fireEvent.click(screen.getByText('Preferences').closest('button')!);
    expect(screen.getByText('voice memos')).toBeInTheDocument();
    expect(screen.getAllByText('person')[0]).toBeInTheDocument();
  });

  it('renders loading and empty insight states', () => {
    const { rerender, container } = render(<MemoryInsights relations={[]} loading />);

    expect(screen.getByRole('heading', { name: /Insights/i })).toBeInTheDocument();
    expect(container.querySelector('.animate-pulse')).toBeInTheDocument();

    rerender(<MemoryInsights relations={[]} />);
    expect(screen.getByText(/No insights yet/)).toBeInTheDocument();
  });

  it('wires navigator search, source selection, and entity selection', async () => {
    const onSearchChange = vi.fn();
    const onSelectionChange = vi.fn();
    const selection: NavigatorSelection = { sourceIds: [], entityIds: [] };

    render(
      <MemoryNavigator
        chunks={[
          makeChunk({ id: 'today', timestamp_ms: localDayAt(0, 9) }),
          makeChunk({ id: 'week', timestamp_ms: localDayAt(3, 9) }),
        ]}
        sources={[makeSource()]}
        topPeople={[makeEntity()]}
        topTopics={[makeEntity({ entity_id: 'topic:Atlas', kind: 'topic', surface: 'Atlas' })]}
        selection={selection}
        onSelectionChange={onSelectionChange}
        searchQuery=""
        onSearchChange={onSearchChange}
      />
    );

    fireEvent.change(screen.getByLabelText(/Search memor/i), { target: { value: 'atlas' } });
    expect(onSearchChange).toHaveBeenCalledWith('atlas');

    expect(screen.getByText('Today 1')).toBeInTheDocument();
    expect(screen.getByText('This Week 2')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Email/ }));
    fireEvent.click(screen.getByRole('button', { name: /Alice Inbox/ }));
    expect(onSelectionChange).toHaveBeenCalledWith({
      sourceIds: ['gmail:alice@example.com'],
      entityIds: [],
    });

    fireEvent.click(screen.getByTitle('Alice').closest('button')!);
    expect(onSelectionChange).toHaveBeenCalledWith({ sourceIds: [], entityIds: ['person:Alice'] });
  });
});
