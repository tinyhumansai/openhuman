import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Chunk } from '../../utils/tauriCommands';
import { MemoryChunkDetail } from './MemoryChunkDetail';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../utils/tauriCommands')>(
    '../../utils/tauriCommands'
  );
  return {
    ...actual,
    memoryTreeEntityIndexFor: vi.fn().mockResolvedValue([]),
    memoryTreeChunkScore: vi.fn().mockResolvedValue(null),
  };
});

function chunk(over: Partial<Chunk> = {}): Chunk {
  return {
    id: 'chunk-abcdef1234',
    source_kind: 'obsidian',
    source_id: 'steven|team',
    source_ref: 'obsidian://vault/note.md',
    owner: 'team',
    timestamp_ms: 1_700_000_000_000,
    token_count: 42,
    lifecycle_status: 'active',
    content_preview: 'A short subject line. Followed by the body.',
    has_embedding: true,
    tags: [],
    ...over,
  } as Chunk;
}

describe('MemoryChunkDetail', () => {
  beforeEach(() => {
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
  });

  it('renders the letter with a derived subject and the chunk id footer button', () => {
    render(<MemoryChunkDetail chunk={chunk()} onSelectEntity={vi.fn()} />);

    expect(screen.getByTestId('memory-chunk-detail')).toBeInTheDocument();
    expect(screen.getByText('A short subject line')).toBeInTheDocument();

    const copyButton = screen.getByRole('button', {
      name: /intelligence.memoryChunk.detail.chunk/,
    });
    expect(copyButton).toHaveAttribute('data-slot', 'button');
  });

  it('copies the chunk id to the clipboard on click', async () => {
    render(<MemoryChunkDetail chunk={chunk()} onSelectEntity={vi.fn()} />);

    const copyButton = screen.getByRole('button', {
      name: /intelligence.memoryChunk.detail.chunk/,
    });
    fireEvent.click(copyButton);

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('chunk-abcdef1234');
  });
});
