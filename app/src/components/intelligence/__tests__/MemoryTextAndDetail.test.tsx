import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  Chunk,
  EntityRef,
  MemoryRetrievalEntity,
  ScoreBreakdown,
} from '../../../utils/tauriCommands';
import { MemoryChunkDetail } from '../MemoryChunkDetail';
import { MemoryTextWithEntities } from '../MemoryTextWithEntities';

const rpcMocks = vi.hoisted(() => ({
  memoryTreeEntityIndexFor: vi.fn(),
  memoryTreeChunkScore: vi.fn(),
}));

vi.mock('../../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../../utils/tauriCommands')>(
    '../../../utils/tauriCommands'
  );
  return {
    ...actual,
    memoryTreeEntityIndexFor: rpcMocks.memoryTreeEntityIndexFor,
    memoryTreeChunkScore: rpcMocks.memoryTreeChunkScore,
  };
});

const BASE_CHUNK: Chunk = {
  id: 'chunk-1234567890abcdef',
  source_kind: 'email',
  source_id: 'gmail:alice@example.com|bob@example.com',
  source_ref: 'gmail://msg/abc',
  owner: 'bob@example.com',
  timestamp_ms: Date.UTC(2026, 4, 16, 9, 30, 0),
  token_count: 220,
  lifecycle_status: 'admitted',
  content_preview:
    'Project Atlas kickoff. Alice (PERSON) owns Atlas (PROJECT). Follow up tomorrow.',
  has_embedding: true,
  tags: ['person/Alice'],
};

const ENTITIES: EntityRef[] = [
  { entity_id: 'person:Alice', kind: 'person', surface: 'Alice', count: 2 },
  { entity_id: 'project:Atlas', kind: 'project', surface: 'Atlas', count: 1 },
];

const SCORE: ScoreBreakdown = {
  signals: [
    { name: 'recency', weight: 0.4, value: 0.8 },
    { name: 'importance', weight: 0.6, value: 0.6 },
  ],
  total: 0.68,
  threshold: 0.5,
  kept: true,
  llm_consulted: false,
};

describe('MemoryTextWithEntities', () => {
  it('renders nothing when there is no text or entity chip data', () => {
    const { container } = render(<MemoryTextWithEntities text="" />);

    expect(container).toBeEmptyDOMElement();
  });

  it('renders structured entity chips with type titles', () => {
    const entities: MemoryRetrievalEntity[] = [
      { id: 'e1', name: 'Alice', entity_type: 'PERSON' },
      { id: 'e2', name: 'Atlas', entity_type: 'PROJECT' },
    ];

    render(<MemoryTextWithEntities text="Relevant context" entities={entities} />);

    expect(screen.getByTitle('Alice (PERSON)')).toBeInTheDocument();
    expect(screen.getByTitle('Atlas (PROJECT)')).toBeInTheDocument();
    expect(screen.getByText('Relevant context')).toBeInTheDocument();
  });

  it('turns inline entity annotations into accessible badges', () => {
    render(<MemoryTextWithEntities text="Alice (PERSON) owns Atlas (PROJECT)." />);

    expect(screen.getByTitle('Entity type: PERSON')).toHaveTextContent('PERSON');
    expect(screen.getByTitle('Entity type: PROJECT')).toHaveTextContent('PROJECT');
    expect(screen.getByText(/Alice/)).toBeInTheDocument();
    expect(screen.getByText(/owns Atlas/)).toBeInTheDocument();
  });
});

describe('MemoryChunkDetail', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    rpcMocks.memoryTreeEntityIndexFor.mockResolvedValue(ENTITIES);
    rpcMocks.memoryTreeChunkScore.mockResolvedValue(SCORE);
  });

  it('loads chunk entities and score details for the letter view', async () => {
    const onSelectEntity = vi.fn();
    render(<MemoryChunkDetail chunk={BASE_CHUNK} onSelectEntity={onSelectEntity} />);

    expect(screen.getByTestId('memory-chunk-detail')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Project Atlas kickoff' })).toBeInTheDocument();
    expect(screen.getByTitle('Entity type: PERSON')).toHaveTextContent('PERSON');
    expect(screen.getByTitle('Entity type: PROJECT')).toHaveTextContent('PROJECT');

    await waitFor(() => {
      expect(rpcMocks.memoryTreeEntityIndexFor).toHaveBeenCalledWith(BASE_CHUNK.id);
      expect(rpcMocks.memoryTreeChunkScore).toHaveBeenCalledWith(BASE_CHUNK.id);
    });

    const mentioned = await screen.findByTestId('memory-chunk-mentioned');
    expect(within(mentioned).getByText('Alice')).toBeInTheDocument();
    expect(within(mentioned).getByText('2 chunks')).toBeInTheDocument();
    expect(within(mentioned).getByText('Atlas')).toBeInTheDocument();
    expect(within(mentioned).getByText('1 chunk')).toBeInTheDocument();

    fireEvent.click(within(mentioned).getByText('Alice'));
    expect(onSelectEntity).toHaveBeenCalledWith(ENTITIES[0]);

    expect(screen.getByTestId('memory-chunk-scorebars')).toBeInTheDocument();
    expect(screen.getByText('recency')).toBeInTheDocument();
    expect(screen.getByLabelText('recency score 80 percent')).toBeInTheDocument();
    expect(screen.getByText('gmail://msg/abc')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /chunk 12345678/i })).toBeInTheDocument();
    expect(screen.getByText('bge-m3 1024dim')).toBeInTheDocument();
  });

  it('copies the full chunk id from the footer button', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });

    render(<MemoryChunkDetail chunk={BASE_CHUNK} onSelectEntity={vi.fn()} />);

    fireEvent.click(screen.getByTitle('Copy chunk id'));

    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(BASE_CHUNK.id);
    });
    expect(await screen.findByText('copied')).toBeInTheDocument();
  });
});
