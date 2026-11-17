import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Source } from '../../services/api/memoryApi';
import { renderWithProviders } from '../../test/test-utils';
import MemorySyncedSources from './MemorySyncedSources';

const hoisted = vi.hoisted(() => ({ list: vi.fn(), add: vi.fn(), remove: vi.fn(), sync: vi.fn() }));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memorySourcesList: (...a: unknown[]) => hoisted.list(...a),
  memorySourcesAdd: (...a: unknown[]) => hoisted.add(...a),
  memorySourcesRemove: (...a: unknown[]) => hoisted.remove(...a),
  memorySourcesSync: (...a: unknown[]) => hoisted.sync(...a),
}));

const NOTES: Source = {
  id: 's1',
  kind: 'folder',
  target: '/Users/me/notes',
  label: 'Notes',
  schedule_mins: 60,
  last_sync_at: '2026-10-01T10:00:00Z',
  status: 'idle',
  items: 42,
};
const FEED: Source = {
  id: 's2',
  kind: 'rss',
  target: 'https://example.com/feed.xml',
  label: '',
  status: 'error',
  error: 'feed returned 404',
  items: 0,
};

beforeEach(() => {
  hoisted.list.mockReset().mockResolvedValue({ sources: [NOTES, FEED] });
  hoisted.add.mockReset();
  hoisted.remove.mockReset();
  hoisted.sync.mockReset();
});

describe('MemorySyncedSources', () => {
  it('lists sources with status, items, schedule and errors', async () => {
    renderWithProviders(<MemorySyncedSources />);
    const notes = await screen.findByTestId('memory-source-s1');
    expect(notes).toHaveTextContent('Notes');
    expect(notes).toHaveTextContent('Folder');
    expect(notes).toHaveTextContent('42 items');
    expect(notes).toHaveTextContent('Every 60 min');
    expect(notes).toHaveTextContent('Last synced');
    expect(screen.getByTestId('memory-source-s2-status')).toHaveTextContent('Error');
    expect(screen.getByTestId('memory-source-s2-error')).toHaveTextContent('feed returned 404');
    expect(screen.getByTestId('memory-source-s2')).toHaveTextContent('Never synced');
  });

  it('shows the empty state', async () => {
    hoisted.list.mockResolvedValue({ sources: [] });
    renderWithProviders(<MemorySyncedSources />);
    expect(await screen.findByTestId('memory-sources-empty')).toBeInTheDocument();
    expect(screen.getByTestId('memory-sources-sync-all')).toBeDisabled();
  });

  it('adds a folder source', async () => {
    const added: Source = { ...NOTES, id: 's3', label: 'Docs', target: '/docs', items: 0 };
    hoisted.add.mockResolvedValue({ source: added });
    renderWithProviders(<MemorySyncedSources />);
    fireEvent.click(await screen.findByTestId('memory-sources-add'));

    fireEvent.change(screen.getByTestId('memory-add-source-target'), {
      target: { value: '/docs' },
    });
    fireEvent.change(screen.getByTestId('memory-add-source-label'), { target: { value: 'Docs' } });
    fireEvent.change(screen.getByTestId('memory-add-source-schedule'), { target: { value: '30' } });
    fireEvent.click(screen.getByTestId('memory-add-source-submit'));

    await waitFor(() =>
      expect(hoisted.add).toHaveBeenCalledWith({
        kind: 'folder',
        target: '/docs',
        label: 'Docs',
        schedule_mins: 30,
      })
    );
    expect(await screen.findByTestId('memory-source-s3')).toHaveTextContent('Docs');
    await waitFor(() => expect(screen.queryByTestId('memory-add-source')).not.toBeInTheDocument());
  });

  it('offers every source kind except Composio', async () => {
    renderWithProviders(<MemorySyncedSources />);
    fireEvent.click(await screen.findByTestId('memory-sources-add'));
    const kinds = Array.from(
      (screen.getByTestId('memory-add-source-kind') as HTMLSelectElement).options
    ).map(option => option.value);
    expect(kinds).toEqual(['folder', 'file', 'link', 'github', 'rss']);
  });

  it('adds a GitHub source and keeps the dialog open on failure', async () => {
    hoisted.add.mockRejectedValue(new Error('INVALID_REQUEST: repo not found'));
    renderWithProviders(<MemorySyncedSources />);
    fireEvent.click(await screen.findByTestId('memory-sources-add'));
    fireEvent.change(screen.getByTestId('memory-add-source-kind'), { target: { value: 'github' } });
    expect(screen.getByTestId('memory-add-source-target')).toHaveAttribute(
      'placeholder',
      'owner/repo'
    );
    fireEvent.change(screen.getByTestId('memory-add-source-target'), {
      target: { value: 'acme/missing' },
    });
    fireEvent.click(screen.getByTestId('memory-add-source-submit'));
    expect(await screen.findByTestId('memory-add-source-error')).toHaveTextContent(
      'repo not found'
    );
    expect(hoisted.add).toHaveBeenCalledWith({ kind: 'github', target: 'acme/missing' });
  });

  it('prompts a top-up in the add dialog when the account is out of credits', async () => {
    hoisted.add.mockRejectedValue(new Error('INSUFFICIENT_CREDITS: HTTP 402'));
    renderWithProviders(<MemorySyncedSources />);
    fireEvent.click(await screen.findByTestId('memory-sources-add'));
    fireEvent.change(screen.getByTestId('memory-add-source-target'), {
      target: { value: '/Users/me/notes' },
    });
    fireEvent.click(screen.getByTestId('memory-add-source-submit'));
    const prompt = await screen.findByTestId('memory-add-source-error');
    expect(prompt).toHaveAttribute('data-kind', 'out-of-credits');
    expect(prompt).not.toHaveTextContent('HTTP 402');
    expect(screen.getByTestId('memory-top-up')).toBeInTheDocument();
  });

  it('syncs one source and marks it syncing', async () => {
    hoisted.sync.mockResolvedValue({ started: ['s1'] });
    renderWithProviders(<MemorySyncedSources />);
    fireEvent.click(await screen.findByTestId('memory-source-s1-sync'));
    await waitFor(() => expect(hoisted.sync).toHaveBeenCalledWith('s1'));
    expect(await screen.findByTestId('memory-source-s1-status')).toHaveTextContent('Syncing');
  });

  it('syncs every source', async () => {
    hoisted.sync.mockResolvedValue({ started: ['s1', 's2'] });
    renderWithProviders(<MemorySyncedSources />);
    fireEvent.click(await screen.findByTestId('memory-sources-sync-all'));
    await waitFor(() => expect(hoisted.sync).toHaveBeenCalledWith(undefined));
  });

  it('removes a source and optionally forgets its items', async () => {
    hoisted.remove.mockResolvedValue({ removed: true });
    renderWithProviders(<MemorySyncedSources />);
    fireEvent.click(await screen.findByTestId('memory-source-s1-remove'));
    fireEvent.click(screen.getByTestId('memory-remove-source-forget'));
    fireEvent.click(screen.getByTestId('memory-remove-source-confirm'));
    await waitFor(() => expect(hoisted.remove).toHaveBeenCalledWith('s1', true));
    await waitFor(() => expect(screen.queryByTestId('memory-source-s1')).not.toBeInTheDocument());
  });

  it('shows a load error', async () => {
    hoisted.list.mockRejectedValue(new Error('ENGINE: down'));
    renderWithProviders(<MemorySyncedSources />);
    expect(await screen.findByTestId('memory-synced-sources-error')).toHaveTextContent('down');
  });
});
