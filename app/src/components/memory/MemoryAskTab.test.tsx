import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import MemoryAskTab from './MemoryAskTab';

const hoisted = vi.hoisted(() => ({
  recall: vi.fn(),
  fetch: vi.fn(),
  preview: vi.fn(),
  agents: vi.fn(),
  policy: vi.fn(),
}));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryRecall: (...a: unknown[]) => hoisted.recall(...a),
  memoryFetch: (...a: unknown[]) => hoisted.fetch(...a),
  memoryPackPreview: (...a: unknown[]) => hoisted.preview(...a),
  memoryAgentsList: (...a: unknown[]) => hoisted.agents(...a),
  memoryPolicyGet: (...a: unknown[]) => hoisted.policy(...a),
}));

beforeEach(() => {
  hoisted.recall.mockReset();
  hoisted.fetch.mockReset();
  hoisted.preview.mockReset();
  hoisted.agents
    .mockReset()
    .mockResolvedValue({ root: 'user:me', agents: [{ agent_id: 'researcher', turns: 12 }] });
  hoisted.policy
    .mockReset()
    .mockResolvedValue({
      log_conversations: true,
      recall: { enabled: true, budget_tokens: 1500 },
      root: 'user:me',
      agent_id: 'main',
      host_bound: false,
    });
});

const PACK = {
  agent_id: 'researcher',
  root: 'user:me',
  mode: 'turn',
  pack: {
    markdown: '## Learnings\n\n- Prefers tea',
    tokens: 1800,
    refs: ['l1'],
    sections: [
      {
        heading: 'Learnings',
        hits: [{ id: 'l1', kind: 'learning', text: 'Prefers tea', meta: {}, score: 1 }],
      },
      { heading: 'Brain', hits: [] },
    ],
    skipped: [{ heading: 'Team', reason: 'no team memory' }],
    engine: 'tinyhumans',
  },
};

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
    fireEvent.click(screen.getByTestId('memory-ask-mode-raw'));

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
    fireEvent.click(screen.getByTestId('memory-ask-mode-raw'));
    expect(screen.queryByTestId('memory-ask-mode')).not.toBeInTheDocument();
  });

  it('says so when a raw search finds nothing', async () => {
    hoisted.fetch.mockResolvedValue({ hits: [] });
    renderWithProviders(<MemoryAskTab fetchModes={['hybrid']} />);
    fireEvent.click(screen.getByTestId('memory-ask-mode-raw'));
    ask('nothing');
    expect(await screen.findByTestId('memory-ask-hits')).toHaveTextContent(
      'Nothing in memory matches that search.'
    );
  });

  it('shows a recall failure', async () => {
    hoisted.recall.mockRejectedValue(new Error('ENGINE: upstream timeout'));
    renderWithProviders(<MemoryAskTab fetchModes={['hybrid']} />);
    ask('anything');
    const error = await screen.findByTestId('memory-ask-error');
    expect(error).toHaveTextContent('upstream timeout');
    expect(error).toHaveAttribute('data-variant', 'destructive');
    expect(screen.queryByTestId('memory-top-up')).not.toBeInTheDocument();
  });

  it('prompts a top-up, not an error, when recall is out of credits', async () => {
    hoisted.recall.mockRejectedValue(
      Object.assign(new Error('insufficient credits: [USER_INSUFFICIENT_CREDITS] HTTP 402'), {
        data: { code: 'INSUFFICIENT_CREDITS', kind: 'INSUFFICIENT_CREDITS' },
      })
    );
    renderWithProviders(<MemoryAskTab fetchModes={['hybrid']} />);
    ask('anything');
    const prompt = await screen.findByTestId('memory-ask-error');
    expect(prompt).toHaveAttribute('data-kind', 'out-of-credits');
    expect(prompt).toHaveAttribute('data-variant', 'warning');
    expect(prompt).toHaveTextContent('Out of credits');
    expect(prompt).toHaveTextContent('Top up to restore it');
    expect(prompt).not.toHaveTextContent('HTTP 402');
    expect(screen.getByTestId('memory-top-up')).toHaveTextContent('Top up');
  });

  it('prompts a top-up for a core that only sends the code as text', async () => {
    hoisted.recall.mockRejectedValue(new Error('INSUFFICIENT_CREDITS: top up'));
    renderWithProviders(<MemoryAskTab fetchModes={['hybrid']} />);
    ask('anything');
    expect(await screen.findByTestId('memory-ask-error')).toHaveAttribute(
      'data-kind',
      'out-of-credits'
    );
  });

  it('previews the pack a turn would get for a chosen agent', async () => {
    hoisted.preview.mockResolvedValue(PACK);
    renderWithProviders(<MemoryAskTab fetchModes={['hybrid']} />);
    fireEvent.click(screen.getByTestId('memory-ask-mode-pack'));

    const agent = await screen.findByTestId('memory-pack-agent');
    await waitFor(() =>
      expect(within(agent).getByRole('option', { name: 'researcher' })).toBeInTheDocument()
    );
    fireEvent.change(agent, { target: { value: 'researcher' } });
    fireEvent.change(screen.getByTestId('memory-pack-query'), { target: { value: 'tea?' } });
    fireEvent.click(screen.getByTestId('memory-pack-submit'));

    await waitFor(() =>
      expect(hoisted.preview).toHaveBeenCalledWith({ query: 'tea?', agent_id: 'researcher' })
    );
    expect(await screen.findByTestId('memory-pack-markdown')).toHaveTextContent('Prefers tea');
    expect(screen.getByTestId('memory-pack-tokens')).toHaveTextContent('1800 of 1500 tokens');
    expect(screen.getByTestId('memory-pack-section-Learnings')).toHaveTextContent('1 hits');
    expect(screen.getByTestId('memory-pack-section-Brain')).toHaveTextContent('0 hits');
    expect(screen.getByTestId('memory-pack-skipped-Team')).toHaveTextContent('no team memory');
    expect(hoisted.recall).not.toHaveBeenCalled();
  });

  it('previews a session start without a query and survives a failed agent list', async () => {
    hoisted.agents.mockRejectedValue(new Error('MEMORY_OFF'));
    hoisted.policy.mockRejectedValue(new Error('MEMORY_OFF'));
    hoisted.preview.mockResolvedValue({
      ...PACK,
      mode: 'session',
      pack: { ...PACK.pack, markdown: '', tokens: 0, sections: [], skipped: [] },
    });
    renderWithProviders(<MemoryAskTab fetchModes={['hybrid']} />);
    fireEvent.click(screen.getByTestId('memory-ask-mode-pack'));
    fireEvent.click(await screen.findByTestId('memory-pack-submit'));

    await waitFor(() => expect(hoisted.preview).toHaveBeenCalledWith({}));
    expect(await screen.findByTestId('memory-pack-result')).toHaveTextContent(
      'Memory for a new session'
    );
    expect(screen.getByTestId('memory-pack-tokens')).toHaveTextContent('0 tokens');
    expect(screen.getByTestId('memory-pack-markdown')).toHaveTextContent(
      'Nothing in memory made it into this pack.'
    );
  });

  it('shows a pack preview failure', async () => {
    hoisted.preview.mockRejectedValue(new Error('ENGINE: down'));
    renderWithProviders(<MemoryAskTab fetchModes={['hybrid']} />);
    fireEvent.click(screen.getByTestId('memory-ask-mode-pack'));
    fireEvent.click(await screen.findByTestId('memory-pack-submit'));
    expect(await screen.findByTestId('memory-pack-error')).toHaveTextContent('ENGINE: down');
  });
});
