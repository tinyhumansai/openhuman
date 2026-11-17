import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import MemoryBrainTab from './MemoryBrainTab';

const hoisted = vi.hoisted(() => ({
  sources: vi.fn(),
  search: vi.fn(),
  ingest: vi.fn(),
  forget: vi.fn(),
}));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryBrainSources: (...a: unknown[]) => hoisted.sources(...a),
  memoryBrainSearch: (...a: unknown[]) => hoisted.search(...a),
  memoryBrainIngest: (...a: unknown[]) => hoisted.ingest(...a),
  memoryBrainForget: (...a: unknown[]) => hoisted.forget(...a),
}));

// The synced-sources registry has its own suite.
vi.mock('./MemorySyncedSources', () => ({
  default: () => <div data-testid="stub-synced-sources" />,
}));

const BRAIN = {
  root: 'user:me',
  sources: [
    { source: 'pdf', documents: 4 },
    { source: 'gmail', documents: 2 },
  ],
  unfiled: 1,
};

beforeEach(() => {
  hoisted.sources.mockReset().mockResolvedValue(BRAIN);
  hoisted.search.mockReset();
  hoisted.ingest.mockReset();
  hoisted.forget.mockReset();
});

describe('MemoryBrainTab', () => {
  it('lists per-source counts, the unfiled count and the synced sources', async () => {
    renderWithProviders(<MemoryBrainTab />);
    const pdf = await screen.findByTestId('memory-brain-source-pdf');
    expect(pdf).toHaveTextContent('PDF');
    expect(pdf).toHaveTextContent('4 documents');
    // An unknown source id shows verbatim.
    expect(screen.getByTestId('memory-brain-source-gmail')).toHaveTextContent('gmail');
    expect(screen.getByTestId('memory-brain-unfiled')).toHaveTextContent('1 documents');
    expect(screen.getByTestId('stub-synced-sources')).toBeInTheDocument();
  });

  it('shows the empty state', async () => {
    hoisted.sources.mockResolvedValue({ root: 'user:me', sources: [], unfiled: 0 });
    renderWithProviders(<MemoryBrainTab />);
    expect(await screen.findByTestId('memory-brain-empty')).toBeInTheDocument();
  });

  it('forgets a source only after confirming', async () => {
    hoisted.forget.mockResolvedValue({ forgotten: 4 });
    renderWithProviders(<MemoryBrainTab />);
    fireEvent.click(await screen.findByTestId('memory-brain-source-pdf-forget'));
    expect(await screen.findByTestId('memory-brain-forget')).toHaveTextContent(
      'Every PDF document will be removed'
    );
    expect(hoisted.forget).not.toHaveBeenCalled();

    hoisted.sources.mockResolvedValue({ ...BRAIN, sources: [BRAIN.sources[1]] });
    fireEvent.click(screen.getByTestId('memory-brain-forget-confirm'));
    await waitFor(() => expect(hoisted.forget).toHaveBeenCalledWith('pdf'));
    expect(await screen.findByTestId('memory-brain-notice')).toHaveTextContent(
      'Forgot 4 PDF documents.'
    );
    await waitFor(() =>
      expect(screen.queryByTestId('memory-brain-source-pdf')).not.toBeInTheDocument()
    );
  });

  it('shows a forget failure', async () => {
    hoisted.forget.mockRejectedValue(new Error('ENGINE: down'));
    renderWithProviders(<MemoryBrainTab />);
    fireEvent.click(await screen.findByTestId('memory-brain-source-gmail-forget'));
    fireEvent.click(await screen.findByTestId('memory-brain-forget-confirm'));
    expect(await screen.findByTestId('memory-brain-error')).toHaveTextContent('ENGINE: down');
  });

  it('searches the brain within one source', async () => {
    hoisted.search.mockResolvedValue({
      hits: [{ id: 'd1', kind: 'document', text: 'Q3 plan', meta: {}, score: 0.7 }],
    });
    renderWithProviders(<MemoryBrainTab />);
    const source = await screen.findByTestId('memory-brain-search-source');
    fireEvent.change(source, { target: { value: 'pdf' } });
    fireEvent.change(screen.getByTestId('memory-brain-search-input'), {
      target: { value: 'planning' },
    });
    fireEvent.click(screen.getByTestId('memory-brain-search-submit'));
    await waitFor(() =>
      expect(hoisted.search).toHaveBeenCalledWith({ query: 'planning', source: 'pdf', limit: 20 })
    );
    const hits = await screen.findByTestId('memory-brain-search-hits');
    expect(within(hits).getByTestId('memory-hit-d1')).toHaveTextContent('Q3 plan');
  });

  it('says when a search finds nothing, and shows a search failure', async () => {
    hoisted.search.mockResolvedValueOnce({ hits: [] }).mockRejectedValueOnce(new Error('boom'));
    renderWithProviders(<MemoryBrainTab />);
    const input = await screen.findByTestId('memory-brain-search-input');
    fireEvent.change(input, { target: { value: 'nothing' } });
    fireEvent.click(screen.getByTestId('memory-brain-search-submit'));
    expect(await screen.findByTestId('memory-brain-search-empty')).toBeInTheDocument();
    expect(hoisted.search).toHaveBeenCalledWith({ query: 'nothing', limit: 20 });

    fireEvent.click(screen.getByTestId('memory-brain-search-submit'));
    expect(await screen.findByTestId('memory-brain-search-error')).toHaveTextContent('boom');
  });

  it('adds pasted text with a title and source', async () => {
    hoisted.ingest.mockResolvedValue({ id: 'doc-1', source: 'markdown', replayed: false });
    renderWithProviders(<MemoryBrainTab />);
    fireEvent.click(await screen.findByTestId('memory-brain-add'));
    const submit = screen.getByTestId('memory-brain-ingest-submit');
    expect(submit).toBeDisabled();

    fireEvent.change(screen.getByTestId('memory-brain-ingest-text'), {
      target: { value: '  # Notes  ' },
    });
    fireEvent.change(screen.getByTestId('memory-brain-ingest-title'), {
      target: { value: 'Notes' },
    });
    fireEvent.change(screen.getByTestId('memory-brain-ingest-source'), {
      target: { value: 'markdown' },
    });
    fireEvent.click(submit);

    await waitFor(() =>
      expect(hoisted.ingest).toHaveBeenCalledWith({
        text: '# Notes',
        title: 'Notes',
        source: 'markdown',
      })
    );
    expect(await screen.findByTestId('memory-brain-notice')).toHaveTextContent(
      'Added to the brain under Markdown.'
    );
    expect(screen.queryByTestId('memory-brain-ingest')).not.toBeInTheDocument();
    expect(hoisted.sources).toHaveBeenCalledTimes(2);
  });

  it('adds a file by path and reports a replayed document', async () => {
    hoisted.ingest.mockResolvedValue({ id: 'doc-1', source: 'pdf', replayed: true });
    renderWithProviders(<MemoryBrainTab />);
    fireEvent.click(await screen.findByTestId('memory-brain-add'));
    fireEvent.click(screen.getByTestId('memory-brain-ingest-from-path'));
    fireEvent.change(screen.getByTestId('memory-brain-ingest-path'), {
      target: { value: '/docs/plan.pdf' },
    });
    fireEvent.click(screen.getByTestId('memory-brain-ingest-submit'));
    await waitFor(() => expect(hoisted.ingest).toHaveBeenCalledWith({ path: '/docs/plan.pdf' }));
    expect(await screen.findByTestId('memory-brain-notice')).toHaveTextContent(
      'That document is already in the brain.'
    );
  });

  it('keeps the dialog open with the error when ingest fails', async () => {
    hoisted.ingest.mockRejectedValue(new Error('INVALID_REQUEST: unreadable'));
    renderWithProviders(<MemoryBrainTab />);
    fireEvent.click(await screen.findByTestId('memory-brain-add'));
    fireEvent.change(screen.getByTestId('memory-brain-ingest-text'), { target: { value: 'x' } });
    fireEvent.click(screen.getByTestId('memory-brain-ingest-submit'));
    expect(await screen.findByTestId('memory-brain-ingest-error')).toHaveTextContent('unreadable');
    expect(screen.getByTestId('memory-brain-ingest')).toBeInTheDocument();
  });

  it('prompts a top-up in the ingest dialog when the account is out of credits', async () => {
    hoisted.ingest.mockRejectedValue(new Error('INSUFFICIENT_CREDITS: HTTP 402'));
    renderWithProviders(<MemoryBrainTab />);
    fireEvent.click(await screen.findByTestId('memory-brain-add'));
    fireEvent.change(screen.getByTestId('memory-brain-ingest-text'), { target: { value: 'x' } });
    fireEvent.click(screen.getByTestId('memory-brain-ingest-submit'));
    const prompt = await screen.findByTestId('memory-brain-ingest-error');
    expect(prompt).toHaveAttribute('data-kind', 'out-of-credits');
    expect(screen.getByTestId('memory-top-up')).toBeInTheDocument();
  });

  it('shows a load error', async () => {
    hoisted.sources.mockRejectedValue(new Error('MEMORY_OFF'));
    renderWithProviders(<MemoryBrainTab />);
    expect(await screen.findByTestId('memory-brain-error')).toHaveTextContent('MEMORY_OFF');
    expect(screen.getByTestId('memory-brain-empty')).toBeInTheDocument();
  });
});
