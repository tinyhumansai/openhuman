import { fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Chunk, EntityRef, Source } from '../../utils/tauriCommands';
import { MemoryNavigator, type NavigatorSelection } from './MemoryNavigator';

const EMPTY_SELECTION: NavigatorSelection = { sourceIds: [], entityIds: [] };

// Freeze the clock so the today/this-week bucket calculations and any
// `timestamp_ms: Date.now()` defaults are deterministic. Picked a noon-UTC
// weekday far from a DST boundary to avoid clock-edge surprises.
const FROZEN_NOW = new Date('2026-03-04T12:00:00.000Z');

function makeChunk(overrides: Partial<Chunk> = {}): Chunk {
  return {
    id: 'c1',
    source_kind: 'email',
    source_id: 'src-1',
    owner: 'me',
    timestamp_ms: FROZEN_NOW.getTime(),
    token_count: 4,
    lifecycle_status: 'admitted',
    has_embedding: false,
    tags: [],
    ...overrides,
  };
}

function makeSource(overrides: Partial<Source> = {}): Source {
  return {
    source_id: 'src-1',
    display_name: 'Alice',
    source_kind: 'email',
    chunk_count: 3,
    most_recent_ms: FROZEN_NOW.getTime(),
    ...overrides,
  };
}

function makeEntity(overrides: Partial<EntityRef> = {}): EntityRef {
  return { entity_id: 'person:Alice', kind: 'person', surface: 'Alice', count: 5, ...overrides };
}

describe('<MemoryNavigator />', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(FROZEN_NOW);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders the search input and dispatches onSearchChange', () => {
    const onSearch = vi.fn();
    render(
      <MemoryNavigator
        chunks={[]}
        sources={[]}
        topPeople={[]}
        topTopics={[]}
        selection={EMPTY_SELECTION}
        onSelectionChange={() => {}}
        searchQuery=""
        onSearchChange={onSearch}
      />
    );
    const input = screen.getByRole('textbox');
    fireEvent.change(input, { target: { value: 'foo' } });
    expect(onSearch).toHaveBeenCalledWith('foo');
  });

  it('renders a single navigator pane with a heatmap host', () => {
    render(
      <MemoryNavigator
        chunks={[]}
        sources={[]}
        topPeople={[]}
        topTopics={[]}
        selection={EMPTY_SELECTION}
        onSelectionChange={() => {}}
        searchQuery=""
        onSearchChange={() => {}}
      />
    );
    expect(screen.getByTestId('memory-navigator')).toBeInTheDocument();
    expect(screen.getByTestId('memory-navigator-heatmap')).toBeInTheDocument();
  });

  it('toggles a source on click and emits the updated selection', () => {
    const onChange = vi.fn();
    render(
      <MemoryNavigator
        chunks={[]}
        sources={[makeSource({ source_id: 'gmail:alice', display_name: 'Alice' })]}
        topPeople={[]}
        topTopics={[]}
        selection={EMPTY_SELECTION}
        onSelectionChange={onChange}
        searchQuery=""
        onSearchChange={() => {}}
      />
    );
    // Source-kind buckets default-closed; open the inner kind section first.
    const kindHeader = screen.getByRole('button', { expanded: false, name: /email/i });
    fireEvent.click(kindHeader);
    fireEvent.click(screen.getByRole('button', { name: /Alice/i, pressed: false }));
    expect(onChange).toHaveBeenCalledWith({ sourceIds: ['gmail:alice'], entityIds: [] });
  });

  it('toggles an entity (person) on click and emits the updated selection', () => {
    const onChange = vi.fn();
    render(
      <MemoryNavigator
        chunks={[]}
        sources={[]}
        topPeople={[makeEntity({ entity_id: 'person:Bob', surface: 'Bob' })]}
        topTopics={[]}
        selection={EMPTY_SELECTION}
        onSelectionChange={onChange}
        searchQuery=""
        onSearchChange={() => {}}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Bob/i, pressed: false }));
    expect(onChange).toHaveBeenCalledWith({ sourceIds: [], entityIds: ['person:Bob'] });
  });

  it('un-toggles an already-active entity', () => {
    const onChange = vi.fn();
    render(
      <MemoryNavigator
        chunks={[]}
        sources={[]}
        topPeople={[makeEntity({ entity_id: 'person:Bob', surface: 'Bob' })]}
        topTopics={[]}
        selection={{ sourceIds: [], entityIds: ['person:Bob'] }}
        onSelectionChange={onChange}
        searchQuery=""
        onSearchChange={() => {}}
      />
    );
    // aria-pressed=true since Bob is in the active set.
    fireEvent.click(screen.getByRole('button', { name: /Bob/i, pressed: true }));
    expect(onChange).toHaveBeenCalledWith({ sourceIds: [], entityIds: [] });
  });

  it('collapses a NavSection when its heading is clicked', () => {
    render(
      <MemoryNavigator
        chunks={[]}
        sources={[]}
        topPeople={[makeEntity({ entity_id: 'person:Bob', surface: 'Bob' })]}
        topTopics={[]}
        selection={EMPTY_SELECTION}
        onSelectionChange={() => {}}
        searchQuery=""
        onSearchChange={() => {}}
      />
    );
    // People section is open by default and Bob is visible.
    expect(screen.getByRole('button', { name: /Bob/i })).toBeInTheDocument();
    const peopleHeader = screen.getByRole('button', { name: /people/i, expanded: true });
    fireEvent.click(peopleHeader);
    expect(screen.queryByRole('button', { name: /Bob/i })).not.toBeInTheDocument();
  });

  it('counts chunks within today and the rolling 7-day window', () => {
    const now = FROZEN_NOW.getTime();
    const chunks = [
      makeChunk({ id: 'today', timestamp_ms: now }),
      makeChunk({ id: 'week', timestamp_ms: now - 3 * 24 * 60 * 60 * 1000 }),
    ];
    render(
      <MemoryNavigator
        chunks={chunks}
        sources={[]}
        topPeople={[]}
        topTopics={[]}
        selection={EMPTY_SELECTION}
        onSelectionChange={() => {}}
        searchQuery=""
        onSearchChange={() => {}}
      />
    );
    // The "recent" section renders "<Today label> <n>" and "<This Week label> <n>"
    // — assert against those labeled counters, not loose digits.
    const recent = within(screen.getByTestId('memory-navigator'));
    expect(recent.getByText(/Today\s+1/i)).toBeInTheDocument();
    expect(recent.getByText(/This Week\s+2/i)).toBeInTheDocument();
  });
});
