import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import MemoryAskTab from './MemoryAskTab';

const hoisted = vi.hoisted(() => ({ recall: vi.fn(), fetch: vi.fn() }));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryRecall: (...a: unknown[]) => hoisted.recall(...a),
  memoryFetch: (...a: unknown[]) => hoisted.fetch(...a),
}));

beforeEach(() => {
  hoisted.recall.mockReset();
  hoisted.fetch.mockReset();
});

function ask(text: string) {
  fireEvent.change(screen.getByTestId('memory-ask-input'), { target: { value: text } });
  fireEvent.click(screen.getByTestId('memory-ask-submit'));
}

describe('MemoryAskTab', () => {
  it('disables Ask until there is a question', () => {
    renderWithProviders(<MemoryAskTab fetchModes={['hybrid']} />);
    expect(screen.getByTestId('memory-ask-submit')).toBeDisabled();
  });

  it('asks memory and shows the answer with its citations', async () => {
    hoisted.recall.mockResolvedValue({
      answer: 'We ship on **Friday**.',
      citations: [
        {
          id: 'c1',
          kind: 'document',
          snippet: 'Launch moved to Friday',
          meta: { file_path: '/notes/launch.md', url: 'https://example.com/launch' },
          score: 0.8,
        },
        {
          id: 'c2',
          kind: 'conversation',
          snippet: 'Agreed in chat',
          meta: { thread_id: 'thread-7' },
        },
      ],
    });
    renderWithProviders(<MemoryAskTab fetchModes={['hybrid']} />);
    ask('When do we ship?');

    await waitFor(() =>
      expect(hoisted.recall).toHaveBeenCalledWith({ question: 'When do we ship?' })
    );
    const answer = await screen.findByTestId('memory-ask-answer');
    expect(answer).toHaveTextContent('We ship on Friday.');
    const c1 = screen.getByTestId('memory-citation-c1');
    expect(c1).toHaveTextContent('Document');
    expect(c1).toHaveTextContent('Launch moved to Friday');
    expect(within(c1).getByTestId('memory-meta-file')).toHaveTextContent('/notes/launch.md');
    expect(within(c1).getByTestId('memory-meta-url')).toHaveTextContent(
      'https://example.com/launch'
    );
    // Citations do not show a score; raw results do.
    expect(within(c1).queryByTestId('memory-hit-score')).not.toBeInTheDocument();
    const c2 = screen.getByTestId('memory-citation-c2');
    expect(within(c2).getByTestId('memory-meta-thread')).toHaveTextContent('thread-7');
  });

  it('runs a raw fetch with the chosen mode and lists scored hits', async () => {
    hoisted.fetch.mockResolvedValue({
      hits: [
        {
          id: 'h1',
          kind: 'learning',
          text: 'Prefers tabs',
          meta: { folder: '/notes' },
          score: 0.912,
        },
      ],
    });
    renderWithProviders(<MemoryAskTab fetchModes={['keyword', 'vector']} />);
    fireEvent.click(screen.getByTestId('memory-ask-raw-toggle'));

    const mode = screen.getByTestId('memory-ask-mode');
    expect(
      within(mode)
        .getAllByRole('option')
        .map(o => o.textContent)
    ).toEqual(['Keyword', 'Semantic']);
    fireEvent.change(mode, { target: { value: 'vector' } });
    ask('tabs');

    await waitFor(() =>
      expect(hoisted.fetch).toHaveBeenCalledWith({ query: 'tabs', mode: 'vector', limit: 20 })
    );
    const hit = await screen.findByTestId('memory-hit-h1');
    expect(hit).toHaveTextContent('Learning');
    expect(within(hit).getByTestId('memory-hit-score')).toHaveTextContent('0.91');
    expect(within(hit).getByTestId('memory-meta-folder')).toHaveTextContent('/notes');
    expect(hoisted.recall).not.toHaveBeenCalled();
  });

  it('hides the mode picker when the engine lists no fetch modes', () => {
    renderWithProviders(<MemoryAskTab fetchModes={[]} />);
    fireEvent.click(screen.getByTestId('memory-ask-raw-toggle'));
    expect(screen.queryByTestId('memory-ask-mode')).not.toBeInTheDocument();
  });

  it('says so when a raw search finds nothing', async () => {
    hoisted.fetch.mockResolvedValue({ hits: [] });
    renderWithProviders(<MemoryAskTab fetchModes={['hybrid']} />);
    fireEvent.click(screen.getByTestId('memory-ask-raw-toggle'));
    ask('nothing');
    expect(await screen.findByTestId('memory-ask-hits')).toHaveTextContent(
      'Nothing in memory matches that search.'
    );
  });

  it('shows a recall failure', async () => {
    hoisted.recall.mockRejectedValue(new Error('ENGINE: upstream timeout'));
    renderWithProviders(<MemoryAskTab fetchModes={['hybrid']} />);
    ask('anything');
    expect(await screen.findByTestId('memory-ask-error')).toHaveTextContent('upstream timeout');
  });
});
