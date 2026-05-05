import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, type Mock, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import type { Chunk, EntityRef, ScoreBreakdown, Source } from '../../../utils/tauriCommands';
import { MemoryWorkspace } from '../MemoryWorkspace';

// The MemoryWorkspace orchestrator + its detail-pane child both fan out
// to the `memory_tree_*` JSON-RPC wrappers. The setup.ts global mock
// stubs auth helpers; we extend it here with the read-side surface so
// the workspace can render against a deterministic fixture set.
vi.mock('../../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => true),
  memoryTreeListChunks: vi.fn(),
  memoryTreeListSources: vi.fn(),
  memoryTreeTopEntities: vi.fn(),
  memoryTreeEntityIndexFor: vi.fn(),
  memoryTreeChunkScore: vi.fn(),
}));

const {
  memoryTreeListChunks,
  memoryTreeListSources,
  memoryTreeTopEntities,
  memoryTreeEntityIndexFor,
  memoryTreeChunkScore,
} = (await import('../../../utils/tauriCommands')) as unknown as {
  memoryTreeListChunks: Mock;
  memoryTreeListSources: Mock;
  memoryTreeTopEntities: Mock;
  memoryTreeEntityIndexFor: Mock;
  memoryTreeChunkScore: Mock;
};

// ── Fixtures — small but realistic ───────────────────────────────────────

const NOW_MS = Date.UTC(2026, 4, 4, 9, 14, 0);
const HOUR = 60 * 60 * 1000;

const FIXTURE_CHUNKS: Chunk[] = [
  {
    id: 'chunk-today-01',
    source_kind: 'email',
    source_id: 'gmail:enamakel@mail.tinyhumans.ai|sanil@vezures.xyz',
    source_ref: 'gmail://msg/aaa',
    owner: 'sanil@vezures.xyz',
    timestamp_ms: NOW_MS,
    token_count: 312,
    lifecycle_status: 'admitted',
    content_preview:
      'welcome to the future of ai assistants — openhuman. hey hey Sanil Jain! steve here.',
    has_embedding: true,
    tags: ['person/Steven-Enamakel', 'organization/TinyHumans', 'product/openhuman'],
  },
  {
    id: 'chunk-today-02',
    source_kind: 'email',
    source_id: 'gmail:notifications@github.com|sanil@vezures.xyz',
    source_ref: 'gmail://msg/bbb',
    owner: 'sanil@vezures.xyz',
    timestamp_ms: NOW_MS - 90 * 60 * 1000,
    token_count: 94,
    lifecycle_status: 'admitted',
    content_preview: '[tinyhumansai/openhuman] PR #1175 merged.',
    has_embedding: true,
    tags: ['organization/GitHub', 'product/openhuman', 'event/pr-merged'],
  },
  {
    id: 'chunk-today-03',
    source_kind: 'chat',
    source_id: 'slack:T0123|C-engineering',
    source_ref: 'slack://channel/eng/p1',
    owner: 'sanil@vezures.xyz',
    timestamp_ms: NOW_MS - 3 * HOUR,
    token_count: 47,
    lifecycle_status: 'admitted',
    content_preview: 'maya patel: pushed the staging chart fix',
    has_embedding: true,
    tags: ['person/Maya-Patel', 'organization/TinyHumans'],
  },
];

const FIXTURE_SOURCES: Source[] = [
  {
    source_id: 'gmail:enamakel@mail.tinyhumans.ai|sanil@vezures.xyz',
    display_name: 'Steven Enamakel',
    source_kind: 'email',
    chunk_count: 1,
    most_recent_ms: NOW_MS,
    lifecycle_status: 'admitted',
  },
  {
    source_id: 'gmail:notifications@github.com|sanil@vezures.xyz',
    display_name: 'GitHub notifications',
    source_kind: 'email',
    chunk_count: 1,
    most_recent_ms: NOW_MS - 90 * 60 * 1000,
    lifecycle_status: 'admitted',
  },
  {
    source_id: 'slack:T0123|C-engineering',
    display_name: 'Slack: #engineering',
    source_kind: 'chat',
    chunk_count: 1,
    most_recent_ms: NOW_MS - 3 * HOUR,
    lifecycle_status: 'admitted',
  },
];

const FIXTURE_PEOPLE: EntityRef[] = [
  { entity_id: 'person:Steven Enamakel', kind: 'person', surface: 'Steven Enamakel', count: 2 },
  { entity_id: 'person:Maya Patel', kind: 'person', surface: 'Maya Patel', count: 1 },
];

const FIXTURE_TOPICS: EntityRef[] = [
  { entity_id: 'product:openhuman', kind: 'product', surface: 'openhuman', count: 3 },
  { entity_id: 'event:pr-merged', kind: 'event', surface: 'pr-merged', count: 1 },
];

const FIXTURE_SCORE: ScoreBreakdown = {
  signals: [
    { name: 'source', weight: 0.3, value: 0.8 },
    { name: 'entities', weight: 0.4, value: 0.7 },
    { name: 'recency', weight: 0.3, value: 0.9 },
  ],
  total: 0.79,
  threshold: 0.85,
  kept: true,
  llm_consulted: false,
};

beforeEach(() => {
  memoryTreeListChunks.mockReset();
  memoryTreeListSources.mockReset();
  memoryTreeTopEntities.mockReset();
  memoryTreeEntityIndexFor.mockReset();
  memoryTreeChunkScore.mockReset();

  memoryTreeListChunks.mockResolvedValue({ chunks: FIXTURE_CHUNKS, total: FIXTURE_CHUNKS.length });
  memoryTreeListSources.mockResolvedValue(FIXTURE_SOURCES);
  // The workspace calls topEntities twice: ('person', 12) and (undefined, 40).
  memoryTreeTopEntities.mockImplementation((kind?: string) => {
    if (kind === 'person') return Promise.resolve(FIXTURE_PEOPLE);
    return Promise.resolve([...FIXTURE_PEOPLE, ...FIXTURE_TOPICS]);
  });
  memoryTreeEntityIndexFor.mockResolvedValue([
    { entity_id: 'person:Steven Enamakel', kind: 'person', surface: 'Steven Enamakel', count: 1 },
    { entity_id: 'organization:TinyHumans', kind: 'organization', surface: 'TinyHumans', count: 1 },
  ]);
  memoryTreeChunkScore.mockResolvedValue(FIXTURE_SCORE);
});

describe('MemoryWorkspace — 2-pane + overlay browser', () => {
  it('renders the navigator + result list scaffold and the search box', async () => {
    renderWithProviders(<MemoryWorkspace />);
    // Workspace renders the empty placeholder until the first fixture
    // round-trip lands — the full 2-pane shell only mounts once allChunks
    // is populated. Wait for the post-load state, then assert all four
    // anchors exist together.
    await waitFor(() => {
      expect(screen.getByTestId('memory-workspace')).toBeInTheDocument();
    });
    expect(screen.getByTestId('memory-navigator')).toBeInTheDocument();
    expect(screen.getByTestId('memory-result-list')).toBeInTheDocument();
    expect(screen.getByLabelText('Search memory')).toBeInTheDocument();
  });

  it('calls the canonical memory_tree_* RPCs on mount', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      expect(memoryTreeListChunks).toHaveBeenCalledWith({ limit: 500 });
      expect(memoryTreeListSources).toHaveBeenCalled();
      expect(memoryTreeTopEntities).toHaveBeenCalledWith('person', 12);
      expect(memoryTreeTopEntities).toHaveBeenCalledWith(undefined, 40);
    });
  });

  it('renders navigator section headings (recent, sources, people, topics)', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      expect(screen.getByText('recent')).toBeInTheDocument();
    });
    expect(screen.getByText('sources')).toBeInTheDocument();
    expect(screen.getByText('people')).toBeInTheDocument();
    expect(screen.getByText('topics')).toBeInTheDocument();
  });

  it('does NOT auto-open the detail overlay on mount (2-pane is the default rest state)', async () => {
    renderWithProviders(<MemoryWorkspace />);
    // Wait for fixtures to land so we know the workspace is fully rendered.
    await waitFor(() => screen.getByTestId('memory-result-list'));
    // The new layout opens detail only on row click; no overlay until then.
    expect(screen.queryByTestId('memory-chunk-detail')).toBeNull();
  });

  it('renders the Sources section count + per-kind nesting after the load resolves', async () => {
    renderWithProviders(<MemoryWorkspace />);
    // Inside the Sources NavSection, fixtures group into Email (2) + Chat (1).
    // Each per-kind sub-section is rendered as its own NavSection — closed by
    // default, but the labels are visible.
    await waitFor(() => {
      expect(screen.getByText('Email')).toBeInTheDocument();
      expect(screen.getByText('Chat')).toBeInTheDocument();
    });
    // Person entities (from FIXTURE_PEOPLE) ARE visible by default — the
    // people NavSection is `defaultOpen`.
    expect(screen.getAllByText('Steven Enamakel').length).toBeGreaterThan(0);
    expect(screen.getByText('Maya Patel')).toBeInTheDocument();
  });

  it('typing in the search box narrows the result-list rows', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      const rows = screen.getAllByRole('button').filter(b => b.dataset.chunkId);
      expect(rows.length).toBe(FIXTURE_CHUNKS.length);
    });

    const search = screen.getByLabelText('Search memory') as HTMLInputElement;
    fireEvent.change(search, { target: { value: 'PR #1175' } });

    await waitFor(() => {
      const visible = screen.getAllByRole('button').filter(b => b.dataset.chunkId);
      expect(visible.length).toBe(1);
      expect(visible[0]?.textContent ?? '').toMatch(/PR #1175|github/i);
    });
  });

  it('opens the detail overlay when a result row is clicked', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      const rows = screen.getAllByRole('button').filter(b => b.dataset.chunkId);
      expect(rows.length).toBeGreaterThan(0);
    });

    const rows = screen.getAllByRole('button').filter(b => b.dataset.chunkId);
    fireEvent.click(rows[0]!);

    await waitFor(() => {
      // Detail overlay mounts the ChunkDetail (data-testid="memory-chunk-detail")
      // along with the letterhead — both show only after a row click in the
      // 2-pane + overlay layout.
      expect(screen.getByTestId('memory-chunk-detail')).toBeInTheDocument();
      expect(screen.getByTestId('memory-chunk-letterhead')).toBeInTheDocument();
    });
  });

  it('closes the detail overlay on Escape key', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      const rows = screen.getAllByRole('button').filter(b => b.dataset.chunkId);
      expect(rows.length).toBeGreaterThan(0);
    });

    const rows = screen.getAllByRole('button').filter(b => b.dataset.chunkId);
    fireEvent.click(rows[0]!);
    await waitFor(() => screen.getByTestId('memory-chunk-detail'));

    fireEvent.keyDown(window, { key: 'Escape' });

    await waitFor(() => {
      expect(screen.queryByTestId('memory-chunk-detail')).toBeNull();
    });
  });
});

describe('MemoryWorkspace — empty state', () => {
  it('renders the empty placeholder when the core returns zero chunks', async () => {
    memoryTreeListChunks.mockResolvedValueOnce({ chunks: [], total: 0 });
    memoryTreeListSources.mockResolvedValueOnce([]);
    memoryTreeTopEntities.mockResolvedValue([]);

    renderWithProviders(<MemoryWorkspace />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-empty-placeholder')).toBeInTheDocument();
    });
    expect(screen.getByText('Nothing yet.')).toBeInTheDocument();
  });
});
